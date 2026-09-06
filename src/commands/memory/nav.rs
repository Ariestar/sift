use anyhow::{bail, Context, Result};
use sivtr_core::record::{WorkAt, WorkPath, WorkRecord, WorkRef};
use std::path::PathBuf;

use crate::cli::NavArgs;
use crate::commands::memory::show;
use crate::commands::memory::workset::{self, WorkSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Parent,
    Child(usize),
    Sibling(isize),
    Window { start: isize, end: isize },
    Session,
}

pub fn execute(args: &NavArgs) -> Result<()> {
    let source = workset::query(
        &args.source,
        crate::commands::memory::filter::Filter::none(),
        args.cwd.as_deref(),
    )?;
    let cwd = PathBuf::from(&source.cwd);
    let source_anchors = source.anchors().to_vec();
    let all_records = workset::load_context_records(source.records(), &source_anchors, &cwd)?;
    let anchors = navigate(
        source.records(),
        &source_anchors,
        &all_records,
        &args.motion,
    )?;
    let records = workset::records_for_anchors(&all_records, &anchors);
    let mut set = WorkSet::from_parts(source.cwd, records, anchors);
    workset::save_last(&set)?;
    show::print_workset(
        &mut set,
        show::resolve_output_format(args.format, false, args.refs, args.json),
    )
}

fn navigate(
    source_records: &[WorkRecord],
    source_anchors: &[WorkRef],
    all_records: &[WorkRecord],
    motion: &str,
) -> Result<Vec<WorkRef>> {
    let steps = parse_motion(motion)?;
    let mut anchors = source_anchors.to_vec();
    for step in steps {
        anchors = apply_step(source_records, &anchors, all_records, step)?;
        anchors = workset::unique_anchors(anchors);
    }
    Ok(anchors)
}

fn apply_step(
    source_records: &[WorkRecord],
    anchors: &[WorkRef],
    all_records: &[WorkRecord],
    step: Step,
) -> Result<Vec<WorkRef>> {
    let mut result = Vec::new();
    for anchor in anchors {
        match step {
            Step::Parent => result.extend(parent(anchor, source_records, all_records)?),
            Step::Child(index) => result.extend(child(anchor, source_records, all_records, index)?),
            Step::Sibling(offset) => {
                result.extend(sibling(anchor, source_records, all_records, offset)?)
            }
            Step::Window { start, end } => {
                result.extend(window(anchor, source_records, all_records, start, end)?)
            }
            Step::Session => result.extend(session(anchor, source_records, all_records)?),
        }
    }
    Ok(result)
}

fn parent(
    anchor: &WorkRef,
    source_records: &[WorkRecord],
    all_records: &[WorkRecord],
) -> Result<Vec<WorkRef>> {
    match anchor.at {
        WorkAt::Part(_) => Ok(vec![anchor.whole()]),
        WorkAt::Whole => session(anchor, source_records, all_records),
    }
}

fn child(
    anchor: &WorkRef,
    source_records: &[WorkRecord],
    all_records: &[WorkRecord],
    index: usize,
) -> Result<Vec<WorkRef>> {
    if index == 0 {
        bail!("child index must be 1-based");
    }
    match anchor.at {
        WorkAt::Whole => {
            let record = workset::require_record([source_records, all_records], anchor)
                .with_context(|| format!("resolve child record for `{anchor}`"))?;
            let Some(part) = record.parts.get(index - 1) else {
                return Ok(Vec::new());
            };
            Ok(vec![record.work_ref.with_part(part.seq)])
        }
        WorkAt::Part(_) => Ok(Vec::new()),
    }
}

fn sibling(
    anchor: &WorkRef,
    source_records: &[WorkRecord],
    all_records: &[WorkRecord],
    offset: isize,
) -> Result<Vec<WorkRef>> {
    match anchor.at {
        WorkAt::Whole => {
            let record = workset::require_record([source_records, all_records], anchor)
                .with_context(|| format!("resolve sibling record for `{anchor}`"))?;
            let session_records = session_records_for(record, all_records);
            let Some(position) = session_records
                .iter()
                .position(|candidate| candidate.work_ref.whole() == anchor.whole())
            else {
                return Ok(Vec::new());
            };
            let Some(target) = offset_index(position, offset, session_records.len()) else {
                return Ok(Vec::new());
            };
            Ok(vec![session_records[target].work_ref.whole()])
        }
        WorkAt::Part(seq) => {
            let record = workset::require_record([source_records, all_records], anchor)
                .with_context(|| format!("resolve sibling record for `{anchor}`"))?;
            let Some(position) = record.parts.iter().position(|part| part.seq == seq) else {
                return Ok(Vec::new());
            };
            let Some(target) = offset_index(position, offset, record.parts.len()) else {
                return Ok(Vec::new());
            };
            Ok(vec![record.work_ref.with_part(record.parts[target].seq)])
        }
    }
}

fn window(
    anchor: &WorkRef,
    source_records: &[WorkRecord],
    all_records: &[WorkRecord],
    start: isize,
    end: isize,
) -> Result<Vec<WorkRef>> {
    if start > end {
        bail!("window start must be <= end");
    }
    match anchor.at {
        WorkAt::Whole => {
            let record = workset::require_record([source_records, all_records], anchor)
                .with_context(|| format!("resolve window record for `{anchor}`"))?;
            let session_records = session_records_for(record, all_records);
            let position = session_records
                .iter()
                .position(|candidate| candidate.work_ref.whole() == anchor.whole())
                .with_context(|| format!("No record found for ref `{}`", anchor.whole()))?;
            let start = clamp_offset(position, start, session_records.len());
            let end = clamp_offset(position, end, session_records.len());
            Ok(session_records[start..=end]
                .iter()
                .map(|record| record.work_ref.whole())
                .collect())
        }
        WorkAt::Part(seq) => {
            let record = workset::require_record([source_records, all_records], anchor)
                .with_context(|| format!("resolve window record for `{anchor}`"))?;
            let position = record
                .parts
                .iter()
                .position(|part| part.seq == seq)
                .with_context(|| format!("No part found for ref `{anchor}`"))?;
            let start = clamp_offset(position, start, record.parts.len());
            let end = clamp_offset(position, end, record.parts.len());
            Ok(record.parts[start..=end]
                .iter()
                .map(|part| record.work_ref.with_part(part.seq))
                .collect())
        }
    }
}

fn session(
    anchor: &WorkRef,
    source_records: &[WorkRecord],
    all_records: &[WorkRecord],
) -> Result<Vec<WorkRef>> {
    let record = workset::require_record([source_records, all_records], anchor)
        .with_context(|| format!("resolve session record for `{anchor}`"))?;
    Ok(session_records_for(record, all_records)
        .into_iter()
        .map(|record| record.work_ref.whole())
        .collect())
}

fn session_records_for<'a>(
    record: &WorkRecord,
    all_records: &'a [WorkRecord],
) -> Vec<&'a WorkRecord> {
    let mut records = all_records
        .iter()
        .filter(|candidate| same_stream(record, candidate))
        .collect::<Vec<_>>();
    records.sort_by_key(|record| record.work_ref.index());
    records
}

fn same_stream(left: &WorkRecord, right: &WorkRecord) -> bool {
    match (&left.work_ref.path, &right.work_ref.path) {
        (WorkPath::Terminal { .. }, WorkPath::Terminal { .. }) => {
            left.work_ref.session() == right.work_ref.session()
        }
        (
            WorkPath::Agent {
                provider: left_provider,
                ..
            },
            WorkPath::Agent {
                provider: right_provider,
                ..
            },
        ) => left_provider == right_provider && left.work_ref.session() == right.work_ref.session(),
        _ => false,
    }
}

fn offset_index(position: usize, offset: isize, len: usize) -> Option<usize> {
    position
        .checked_add_signed(offset)
        .filter(|target| *target < len)
}

fn clamp_offset(position: usize, offset: isize, len: usize) -> usize {
    position
        .saturating_add_signed(offset)
        .min(len.saturating_sub(1))
}

fn parse_motion(motion: &str) -> Result<Vec<Step>> {
    if motion.is_empty() {
        bail!("motion cannot be empty");
    }

    let chars = motion.chars().collect::<Vec<_>>();
    let mut index = 0;
    let mut steps = Vec::new();
    while index < chars.len() {
        match chars[index] {
            '<' => {
                steps.push(Step::Parent);
                index += 1;
            }
            '>' => {
                index += 1;
                let (value, next) = parse_usize(&chars, index, "child")?;
                steps.push(Step::Child(value));
                index = next;
            }
            '+' | '-' => {
                let (value, next) = parse_isize(&chars, index, "sibling")?;
                steps.push(Step::Sibling(value));
                index = next;
            }
            '[' => {
                let close = chars[index..]
                    .iter()
                    .position(|ch| *ch == ']')
                    .map(|offset| index + offset)
                    .ok_or_else(|| anyhow::anyhow!("window motion missing closing ]"))?;
                let body = chars[index + 1..close].iter().collect::<String>();
                let (start, end) = body
                    .split_once("..")
                    .ok_or_else(|| anyhow::anyhow!("window motion must use A..B"))?;
                let start = parse_signed_literal(start, "window start")?;
                let end = parse_signed_literal(end, "window end")?;
                if start > end {
                    bail!("window start must be <= end");
                }
                steps.push(Step::Window { start, end });
                index = close + 1;
            }
            '~' => {
                steps.push(Step::Session);
                index += 1;
            }
            other => bail!("invalid motion token `{other}`"),
        }
    }
    Ok(steps)
}

fn parse_usize(chars: &[char], index: usize, label: &str) -> Result<(usize, usize)> {
    let start = index;
    let mut end = index;
    while end < chars.len() && chars[end].is_ascii_digit() {
        end += 1;
    }
    if start == end {
        bail!("{label} motion requires a number");
    }
    let value = chars[start..end]
        .iter()
        .collect::<String>()
        .parse::<usize>()?;
    Ok((value, end))
}

fn parse_isize(chars: &[char], index: usize, label: &str) -> Result<(isize, usize)> {
    let start = index;
    let mut end = index + 1;
    while end < chars.len() && chars[end].is_ascii_digit() {
        end += 1;
    }
    if start + 1 == end {
        bail!("{label} motion requires a number");
    }
    let value = chars[start..end]
        .iter()
        .collect::<String>()
        .parse::<isize>()?;
    Ok((value, end))
}

fn parse_signed_literal(value: &str, label: &str) -> Result<isize> {
    if value.is_empty() {
        bail!("{label} is empty");
    }
    value
        .parse::<isize>()
        .with_context(|| format!("invalid {label} `{value}`"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::record::{
        MessageRole, WorkChannel, WorkRecordKind, WorkSessionRef, WorkSource, WorkTime,
    };

    #[test]
    fn parses_motion_steps() {
        assert_eq!(
            parse_motion("<+1>2[-1..+2]~").expect("parse motion"),
            vec![
                Step::Parent,
                Step::Sibling(1),
                Step::Child(2),
                Step::Window { start: -1, end: 2 },
                Step::Session,
            ]
        );
        assert!(parse_motion(">").is_err());
        assert!(parse_motion(">0").is_ok());
        assert!(parse_motion("[+2..-1]").is_err());
    }

    #[test]
    fn navigates_parent_child_sibling_and_window() {
        let records = (1..=4).map(test_record).collect::<Vec<_>>();
        let start = vec![records[1].work_ref.with_part(2)];

        assert_refs(
            navigate(&records, &start, &records, "<").expect("parent"),
            &["terminal/session_1/2"],
        );
        assert_refs(
            navigate(&records, &start, &records, "<+1>1").expect("next record first child"),
            &["terminal/session_1/3/p1"],
        );
        assert_refs(
            navigate(&records, &start, &records, "<[-1..+1]").expect("record window"),
            &[
                "terminal/session_1/1",
                "terminal/session_1/2",
                "terminal/session_1/3",
            ],
        );
        assert_refs(
            navigate(&records, &start, &records, "~").expect("session"),
            &[
                "terminal/session_1/1",
                "terminal/session_1/2",
                "terminal/session_1/3",
                "terminal/session_1/4",
            ],
        );
    }

    #[test]
    fn child_index_is_deterministic_not_expand() {
        let records = vec![test_record(1)];
        assert_refs(
            navigate(&records, &[records[0].work_ref.whole()], &records, ">3").expect("child"),
            &["terminal/session_1/1/p2"],
        );
        assert!(navigate(&records, &[records[0].work_ref.whole()], &records, ">0").is_err());
    }

    #[test]
    fn navigation_preserves_named_scope() {
        let mut records = (1..=3).map(test_record).collect::<Vec<_>>();
        for record in &mut records {
            record.work_ref = record.work_ref.with_named_scope("desk");
        }
        let start = vec![records[1].work_ref.whole()];

        assert_refs(
            navigate(&records, &start, &records, "+1").expect("remote sibling"),
            &["desk:terminal/session_1/3"],
        );
        assert_refs(
            navigate(&records, &start, &records, "~").expect("remote session"),
            &[
                "desk:terminal/session_1/1",
                "desk:terminal/session_1/2",
                "desk:terminal/session_1/3",
            ],
        );
    }

    fn assert_refs(actual: Vec<WorkRef>, expected: &[&str]) {
        let actual = actual
            .into_iter()
            .map(|anchor| anchor.to_string())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    fn test_record(index: usize) -> WorkRecord {
        WorkRecord {
            schema_version: sivtr_core::record::RECORD_SCHEMA_VERSION,
            work_ref: WorkRef::terminal("session_1", index),
            kind: WorkRecordKind::TerminalCommand,
            source: WorkSource {
                channel: WorkChannel::Terminal,
                provider: None,
            },
            session: WorkSessionRef {
                id: "session_1".to_string(),
                canonical_id: Some("session_1".to_string()),
                path: None,
            },
            cwd: None,
            title: format!("record {index}"),
            time: WorkTime::from_components(None, None, None),
            status: None,
            parts: vec![
                crate::test_fixtures::message_part(1, MessageRole::User, &format!("cmd {index}")),
                crate::test_fixtures::message_part(
                    1,
                    MessageRole::Assistant,
                    &format!("out {index}.1"),
                ),
                crate::test_fixtures::message_part(
                    2,
                    MessageRole::Assistant,
                    &format!("out {index}.2"),
                ),
            ],
        }
    }
}
