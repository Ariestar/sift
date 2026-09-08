use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use regex::{Regex, RegexBuilder};
use sivtr_core::record::{WorkAt, WorkPartKind, WorkRecord, WorkRef};
use sivtr_core::search::{content_line_matches, Bm25Index, Filter, Searcher, Sort};

use crate::cli::SearchArgs;
use crate::commands::memory::workset::WorkSet;
use crate::commands::memory::{filter, show, workset};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SearchMatch {
    pub(crate) anchor: WorkRef,
    pub(crate) at: WorkAt,
    pub(crate) matched_line: usize,
}

#[derive(Debug)]
pub(crate) struct SearchResult {
    pub(crate) matches: Vec<SearchMatch>,
    pub(crate) regex: Regex,
}

/// Search index over a complete workspace corpus.
pub(crate) struct SearchIndex {
    records: Vec<WorkRecord>,
    bm25: Bm25Index,
}

impl SearchIndex {
    pub(crate) fn new(records: Vec<WorkRecord>) -> Self {
        let bm25 = Bm25Index::build(&records);
        Self { records, bm25 }
    }

    pub(crate) fn search(&self, query: &str, cwd: &Path) -> Result<SearchResult> {
        let term = query.trim();
        if term.is_empty() {
            anyhow::bail!("search query is empty");
        }
        let regex = RegexBuilder::new(term)
            .case_insensitive(true)
            .build()
            .context("invalid search regex")?;
        let filter = Filter {
            pattern: Some(term.to_string()),
            rank: Some(term.to_string()),
            sort: Sort::Relevance,
            ..Filter::default()
        };
        let anchors: Vec<WorkRef> = self
            .records
            .iter()
            .map(|record| record.work_ref.whole())
            .collect();
        let hits =
            Searcher::with_index(&self.records, &self.bm25).search(&filter, &anchors, cwd)?;
        let positions: HashMap<WorkRef, usize> = anchors
            .into_iter()
            .enumerate()
            .map(|(position, anchor)| (anchor, position))
            .collect();
        let mut matches = Vec::new();
        for hit in hits {
            let Some(&record_index) = positions.get(&hit.anchor) else {
                continue;
            };
            let Some(record) = self.records.get(record_index) else {
                continue;
            };
            for matched in content_line_matches(record, &regex) {
                let Some(part) = record.part_for_at(WorkAt::Part(matched.part_seq)) else {
                    continue;
                };
                if matches!(part.kind(), WorkPartKind::ToolCall | WorkPartKind::Skill) {
                    continue;
                }
                matches.push(SearchMatch {
                    anchor: hit.anchor.clone(),
                    at: WorkAt::Part(matched.part_seq),
                    matched_line: matched.line,
                });
            }
        }
        Ok(SearchResult { matches, regex })
    }
}

pub fn execute(args: &SearchArgs) -> Result<()> {
    let mut workset = run(args)?;
    show::print_workset(
        &mut workset,
        show::resolve_output_format(args.format, false, args.refs, args.json),
    )
}

/// Unified query for search: local and remote both run load+filter at the data owner.
pub fn run(args: &SearchArgs) -> Result<WorkSet> {
    let mut set = workset::query(
        &args.source,
        filter::from_search_args(args)?,
        args.cwd.as_deref(),
    )?;
    workset::persist(&mut set, args.save.as_deref()).context("persist search WorkSet")?;
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::record::{
        WorkChannel, WorkPart, WorkPartData, WorkRecordKind, WorkSessionRef, WorkSource, WorkTime,
        RECORD_SCHEMA_VERSION,
    };

    fn record(text: &str, index: usize) -> WorkRecord {
        WorkRecord {
            schema_version: RECORD_SCHEMA_VERSION,
            work_ref: WorkRef::terminal("test", index + 1),
            kind: WorkRecordKind::TerminalCommand,
            source: WorkSource {
                channel: WorkChannel::Terminal,
                provider: None,
            },
            session: WorkSessionRef {
                id: "test".into(),
                canonical_id: Some("test".into()),
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: None,
            title: "test".into(),
            parts: vec![WorkPart {
                seq: 1,
                occurred_at: None,
                data: WorkPartData::Output {
                    content: text.into(),
                    ansi: None,
                },
            }],
        }
    }

    #[test]
    fn search_returns_case_insensitive_line_matches() {
        let records = vec![record("first\nNeedle", 0), record("other", 1)];
        let index = SearchIndex::new(records);

        let result = index
            .search("NEEDLE", Path::new("."))
            .expect("search succeeds");

        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].anchor, WorkRef::terminal("test", 1));
        assert_eq!(result.matches[0].at, WorkAt::Part(1));
        assert_eq!(result.matches[0].matched_line, 2);
    }

    #[test]
    fn search_reports_invalid_regex_without_a_fallback() {
        let index = SearchIndex::new(vec![record("text", 0)]);

        let error = index
            .search("(", Path::new("."))
            .expect_err("invalid regex must fail");

        assert!(error.to_string().contains("invalid search regex"));
    }
}
