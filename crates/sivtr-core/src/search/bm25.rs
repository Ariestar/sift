//! BM25 relevance ranking over a record corpus, structured as passage
//! retrieval over typed work parts.
//!
//! Literature (see `docs/retrieval-literature.md`):
//! - **Passage retrieval** (Callan, SIGIR 1994; the same idea modern RAG calls
//!   chunking): the index unit is a *passage*, not a whole record. sivtr's
//!   `WorkPart` is a natural passage — agent turns already carry typed parts
//!   (user/assistant/thinking/output/tool_result/...). A term that sits in the
//!   middle of a long turn lives inside its own bounded part, so it is fully
//!   indexed instead of being dropped by a positional window.
//! - **Fielded weighting** (BM25F, Robertson et al., CIKM 2004): structure is
//!   signal — the record title and the `Command` part (the executed command
//!   string) are fields with weight [`TITLE_WEIGHT`]; every other part kind
//!   (output, error, tool result, user/assistant dialogue, thinking) is
//!   weighted 1.0 with its own per-kind length normalization. This is the
//!   structural replacement for both the old title-x3 hack and the head+tail
//!   window: boosting incidental keyword mentions inside short terminal-output
//!   passages (measured: it let unrelated terminal records drown provider
//!   sessions and Chinese content queries) hurt more than it helped.
//! - **BM25+ lower bound** (Lv & Zhai, CIKM 2011): every matching query term
//!   contributes at least `idf * δ`, so a document whose passage matches is
//!   scored strictly above one that does not match at all.
//!
//! Record score is the **max over its passages** (passage retrieval): a record
//! is as relevant as its best part. Summing across passages would let a long
//! chat turn that mentions common command words in many parts accumulate past
//! the short record that actually ran the command — max keeps the noise of
//! many-part mentions bounded while a single strong passage wins.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::record::{
    output_blocks_text, MessageRole, WorkActionStatus, WorkPart, WorkPartBody, WorkRecord, WorkRef,
};

/// Standard BM25 term-saturation and length-normalization constants. k1 sits
/// at the top of Robertson's classic [1.2, 2.0] range: less aggressive tf
/// saturation lets a relevant turn that mentions the keyword many times (tf
/// 30-90 in thinking/assistant parts) outscore an incidental mention in a
/// short terminal-output passage (tf 2-3).
const K1: f64 = 2.0;
const B: f64 = 0.75;

/// BM25+ lower bound added to every matching query term (Lv & Zhai, CIKM 2011).
const DELTA: f64 = 1.0;

/// Safety bound for a single passage: genuinely huge parts (rare) get a
/// head+tail window within the part so their contribution stays bounded. Most
/// parts (average 60-170 tokens) are fully indexed.
const MAX_PASSAGE_TOKENS: usize = 2_000;

/// Weight of the title field relative to the body field (BM25F-style).
/// Applied only to multi-token queries (see [`Bm25Index::rank_terms_with`]): a
/// single-token query like `grok` must not promote unrelated terminal records
/// over provider sessions.
pub const TITLE_WEIGHT: f64 = 3.0;

/// Passage kinds. `ToolCall` and `Skill` parts are excluded entirely
/// (structural noise, same as the default content filter). Only the record
/// title and the `Command` part carry a weight above 1.0 — that is the
/// structural signal. Every content kind (output, error, tool result,
/// dialogue, thinking) is weighted 1.0: they are all legitimate evidence, and
/// per-kind length normalization already accounts for their different scales.
///
/// Weights are measured per kind (agent eval, 2026-08-19): agent turns are
/// ~77% tool output by volume vs ~2.4% dialogue, so dialogue (user question
/// is the intent — it is also the fallback title) is up-weighted and tool
/// output down-weighted; thinking sits between. Error/output stay 1.0: they
/// only occur in terminal records and error queries are known-sensitive there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum PassageKind {
    Title = 0,
    Command = 1,
    Output = 2,
    Error = 3,
    ToolResult = 4,
    Assistant = 5,
    User = 6,
    Thinking = 7,
}

impl PassageKind {
    fn weight(self) -> f64 {
        match self {
            Self::Title => 3.0,
            Self::Command => 3.0,
            Self::Output => 1.0,
            Self::Error => 1.0,
            Self::ToolResult => 0.3,
            Self::Assistant => 2.0,
            Self::User => 2.5,
            Self::Thinking => 0.5,
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    const COUNT: usize = 8;
}

fn passage_kind_by_index(index: usize) -> PassageKind {
    match index {
        0 => PassageKind::Title,
        1 => PassageKind::Command,
        2 => PassageKind::Output,
        3 => PassageKind::Error,
        4 => PassageKind::ToolResult,
        5 => PassageKind::Assistant,
        6 => PassageKind::User,
        _ => PassageKind::Thinking,
    }
}

/// Part → indexed passages: `(kind, text)` pairs. A shell action contributes
/// its command (weighted like the title — it is the structural signal) and
/// its output as separate passages; dialogue and reasoning index whole; tool
/// results index their payload; input-only tool calls are structural noise
/// and stay out, same as the default content filter.
fn part_passages(part: &WorkPart) -> Vec<(PassageKind, String)> {
    match &part.body {
        WorkPartBody::Message { role, content, .. } => {
            let kind = match role {
                MessageRole::User => PassageKind::User,
                MessageRole::Assistant => PassageKind::Assistant,
                MessageRole::Reasoning => PassageKind::Thinking,
                MessageRole::System => return Vec::new(),
            };
            vec![(kind, content.text().into_owned())]
        }
        WorkPartBody::Action {
            target,
            input,
            output,
            status,
            ..
        } => {
            let mut passages = Vec::new();
            if target.is_shell() {
                if let Some(input) = input {
                    passages.push((PassageKind::Command, input.text().into_owned()));
                }
                if !output.is_empty() {
                    let kind = match status {
                        WorkActionStatus::Failed | WorkActionStatus::Cancelled => {
                            PassageKind::Error
                        }
                        _ => PassageKind::Output,
                    };
                    passages.push((kind, output_blocks_text(output)));
                }
            } else if !output.is_empty() {
                passages.push((PassageKind::ToolResult, output_blocks_text(output)));
            }
            passages
        }
    }
}

/// Splits on non-alphanumeric runs, lowercases Latin/digit runs (so `Error[E`
/// and `error[E0428]` share the token `error`), emits overlapping CJK bigrams
/// (Lucene CJKAnalyzer style), and drops single-character tokens (Lucene's
/// default minimum token length — a degenerate query token like `e` in
/// `error[E` would otherwise dominate scoring with its near-zero document
/// frequency).
pub struct SimpleTokenizer;

impl SimpleTokenizer {
    pub fn tokenize(&self, text: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut chars = text.chars().peekable();
        while let Some(&ch) = chars.peek() {
            if is_cjk(ch) {
                let mut run = String::new();
                while let Some(&c) = chars.peek() {
                    if !is_cjk(c) {
                        break;
                    }
                    run.push(c);
                    chars.next();
                }
                let run_chars: Vec<char> = run.chars().collect();
                if run_chars.len() == 1 {
                    tokens.push(run);
                } else {
                    for bigram in run_chars.windows(2) {
                        tokens.push(bigram.iter().collect());
                    }
                }
            } else if ch.is_alphanumeric() {
                let mut run = String::new();
                while let Some(&c) = chars.peek() {
                    if is_cjk(c) || !c.is_alphanumeric() {
                        break;
                    }
                    run.push(c);
                    chars.next();
                }
                if !run.is_empty() {
                    tokens.push(run.to_lowercase());
                }
            } else {
                chars.next();
            }
        }
        tokens.retain(|token| token.len() >= 2);
        tokens
    }
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x3400..=0x4DBF     // CJK Unified Ideographs Extension A
        | 0x4E00..=0x9FFF   // CJK Unified Ideographs
        | 0xF900..=0xFAFF   // CJK Compatibility Ideographs
        | 0x20000..=0x2FA1F // CJK Extension B..F + Compatibility Supplement
    )
}

/// Head+tail window within a single oversized passage (safety bound for rare
/// giant parts): first and last `MAX_PASSAGE_TOKENS` tokens stay indexed.
fn window_tokens(tokens: Vec<String>) -> Vec<String> {
    if tokens.len() <= 2 * MAX_PASSAGE_TOKENS {
        return tokens;
    }
    tokens[..MAX_PASSAGE_TOKENS]
        .iter()
        .cloned()
        .chain(tokens[tokens.len() - MAX_PASSAGE_TOKENS..].iter().cloned())
        .collect()
}

/// An in-memory passage-based BM25 index over a record corpus. Build once,
/// query many times. Serialized (MessagePack) into the cache keyed by the
/// corpus fingerprint, so a fresh process reuses the index instead of
/// re-tokenizing every passage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bm25Index {
    n: usize,
    /// Average passage length per passage kind.
    avgdl: [f64; PassageKind::COUNT],
    /// Token -> number of records containing it (record-level document
    /// frequency for idf).
    df: HashMap<String, f64>,
    /// Token -> [(record id, passage id, passage kind index, term frequency)].
    /// The passage id is unique per record (title = 0, then each indexed part
    /// in order), so multiple parts of the same kind stay distinct passages.
    postings: HashMap<String, Vec<(usize, usize, usize, f64)>>,
    /// Passage length per (record, passage id).
    passage_len: HashMap<(usize, usize), f64>,
    refs: Vec<WorkRef>,
}

/// Bump when the BM25 index layout or scoring changes, to invalidate cached
/// indexes built by older code.
pub const INDEX_CACHE_VERSION: u32 = 1;

impl Bm25Index {
    pub fn build(records: &[WorkRecord]) -> Self {
        let mut df = HashMap::new();
        let mut postings: HashMap<String, Vec<(usize, usize, usize, f64)>> = HashMap::new();
        let mut passage_len: HashMap<(usize, usize), f64> = HashMap::new();
        let refs = records
            .iter()
            .map(|record| record.work_ref.whole())
            .collect::<Vec<_>>();

        for (doc_id, record) in records.iter().enumerate() {
            let mut record_tokens: HashSet<String> = HashSet::new();
            let mut next_passage_id = 0usize;
            let push_passage =
                |kind: PassageKind,
                 text: &str,
                 passage_id: usize,
                 passage_len: &mut HashMap<(usize, usize), f64>,
                 postings: &mut HashMap<String, Vec<(usize, usize, usize, f64)>>,
                 record_tokens: &mut HashSet<String>| {
                    let mut tf: HashMap<String, usize> = HashMap::new();
                    for token in window_tokens(SimpleTokenizer.tokenize(text)) {
                        *tf.entry(token).or_insert(0) += 1;
                    }
                    passage_len.insert((doc_id, passage_id), tf.values().sum::<usize>() as f64);
                    for (token, count) in tf {
                        record_tokens.insert(token.clone());
                        postings.entry(token).or_default().push((
                            doc_id,
                            passage_id,
                            kind.index(),
                            count as f64,
                        ));
                    }
                };
            push_passage(
                PassageKind::Title,
                &record.title,
                next_passage_id,
                &mut passage_len,
                &mut postings,
                &mut record_tokens,
            );
            next_passage_id += 1;
            for part in &record.parts {
                for (kind, text) in part_passages(part) {
                    push_passage(
                        kind,
                        &text,
                        next_passage_id,
                        &mut passage_len,
                        &mut postings,
                        &mut record_tokens,
                    );
                    next_passage_id += 1;
                }
            }
            for token in record_tokens {
                *df.entry(token).or_insert(0.0) += 1.0;
            }
        }

        let n = records.len();
        let avg = |kind: PassageKind| -> f64 {
            let values = passage_len
                .iter()
                .filter(|((_, kind_idx), _)| *kind_idx == kind.index())
                .map(|(_, len)| *len)
                .collect::<Vec<_>>();
            if values.is_empty() {
                1.0
            } else {
                values.iter().sum::<f64>() / values.len() as f64
            }
        };
        let avgdl = [
            avg(PassageKind::Title),
            avg(PassageKind::Command),
            avg(PassageKind::Output),
            avg(PassageKind::Error),
            avg(PassageKind::ToolResult),
            avg(PassageKind::Assistant),
            avg(PassageKind::User),
            avg(PassageKind::Thinking),
        ];
        Self {
            n,
            avgdl,
            df,
            postings,
            passage_len,
            refs,
        }
    }

    /// Document-frequency ratio of a token (0.0 when absent). Used by the PRF
    /// difficulty gate: expansion is suppressed for common-word queries.
    pub fn df_ratio(&self, token: &str) -> f64 {
        self.df.get(token).copied().unwrap_or(0.0) / self.n.max(1) as f64
    }

    /// Rank with explicit per-term weights and a configurable title-passage
    /// weight. Single-token queries pass 0.0 so a lone keyword is not boosted
    /// into unrelated terminal records; multi-token command queries keep the
    /// full [`TITLE_WEIGHT`].
    pub fn rank_terms_with(
        &self,
        terms: &[(String, f64)],
        title_weight: f64,
    ) -> Vec<(WorkRef, f32)> {
        self.score_terms(terms, title_weight)
            .into_iter()
            .map(|(doc_id, score)| (self.refs[doc_id].clone(), score as f32))
            .collect()
    }

    fn score_terms(&self, terms: &[(String, f64)], title_weight: f64) -> Vec<(usize, f64)> {
        // (record, passage) -> accumulated score; reduced to per-record max at
        // the end (passage retrieval: a record is as relevant as its best part).
        let mut passage_scores: HashMap<(usize, usize), f64> = HashMap::new();
        for (token, weight) in terms {
            let Some(&df) = self.df.get(token) else {
                continue;
            };
            let Some(postings) = self.postings.get(token) else {
                continue;
            };
            // Robertson idf; non-negative for every token frequency.
            let idf = ((self.n as f64 - df + 0.5) / (df + 0.5) + 1.0).ln();
            for &(doc_id, passage_id, kind_idx, tf) in postings {
                let kind = passage_kind_by_index(kind_idx);
                // The command-field gate (see [`Self::rank_terms_with`]) applies
                // to the title AND the Command passage: a single-token query
                // like `grok` must not promote unrelated terminal records whose
                // Command part is exactly that keyword.
                let kind_weight = if matches!(kind, PassageKind::Title | PassageKind::Command) {
                    title_weight
                } else {
                    kind.weight()
                };
                let avgdl = self.avgdl[kind_idx].max(1.0);
                let len = self
                    .passage_len
                    .get(&(doc_id, passage_id))
                    .copied()
                    .unwrap_or(0.0);
                let sat = tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * len / avgdl));
                let contribution = idf * (kind_weight * sat + DELTA);
                let entry = passage_scores.entry((doc_id, passage_id)).or_insert(0.0);
                *entry += weight * contribution;
            }
        }

        // Passage retrieval: record score = max over its passages.
        let mut record_scores: HashMap<usize, f64> = HashMap::new();
        for ((doc_id, _passage), score) in passage_scores {
            let entry = record_scores.entry(doc_id).or_insert(0.0);
            if score > *entry {
                *entry = score;
            }
        }

        let mut ranked: Vec<(usize, f64)> = record_scores.into_iter().collect();
        // Deterministic order: score desc, then reference asc (HashMap iteration
        // order is randomized per process, so ties must be broken explicitly).
        ranked.sort_by(|a, b| {
            b.1.total_cmp(&a.1)
                .then_with(|| self.refs[a.0].to_string().cmp(&self.refs[b.0].to_string()))
        });
        ranked
    }
}

/// Body text for BM25: every searchable message and action representation.
/// Used by the PRF expansion to re-tokenize the top-ranked documents.
pub fn body_text(record: &WorkRecord) -> String {
    let mut text = String::new();
    for part in &record.parts {
        text.push_str(&part.text());
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{AgentBlock, AgentBlockKind};
    use crate::record::{
        agent_parts, WorkActionStatus, WorkActor, WorkChannel, WorkContent, WorkContentBlock,
        WorkOutcome, WorkPart, WorkPartBody, WorkRecord, WorkRecordKind, WorkSessionRef,
        WorkSource, WorkStatus, WorkTarget, WorkTime, RECORD_SCHEMA_VERSION,
    };

    /// Rank a plain query string (test convenience over [`rank_terms_with`]).
    fn rank_query(index: &Bm25Index, query: &str) -> Vec<(WorkRef, f32)> {
        let terms = SimpleTokenizer
            .tokenize(query)
            .into_iter()
            .map(|token| (token, 1.0))
            .collect::<Vec<_>>();
        index.rank_terms_with(&terms, TITLE_WEIGHT)
    }

    fn text_content(text: &str) -> WorkContent {
        WorkContent::Text {
            content: text.to_string(),
            ansi: None,
        }
    }

    fn shell_action(
        seq: usize,
        command: Option<&str>,
        output: Option<&str>,
        failed: bool,
    ) -> WorkPart {
        WorkPart {
            seq,
            occurred_at: None,
            body: WorkPartBody::Action {
                id: format!("shell-{seq}"),
                actor: WorkActor::User,
                target: WorkTarget::Shell,
                title: None,
                input: command.map(text_content),
                output: output
                    .map(|text| {
                        vec![WorkContentBlock {
                            content: text_content(text),
                            start_line: None,
                        }]
                    })
                    .unwrap_or_default(),
                status: if failed {
                    WorkActionStatus::Failed
                } else {
                    WorkActionStatus::Completed
                },
                exit_code: failed.then_some(1),
            },
        }
    }

    fn record(session: &str, index: usize, title: &str, text: &str) -> WorkRecord {
        WorkRecord {
            schema_version: RECORD_SCHEMA_VERSION,
            work_ref: WorkRef::terminal(session, index),
            kind: WorkRecordKind::TerminalCommand,
            source: WorkSource {
                channel: WorkChannel::Terminal,
                provider: None,
            },
            session: WorkSessionRef {
                id: session.to_string(),
                canonical_id: None,
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: Some(WorkStatus {
                outcome: WorkOutcome::Success,
                exit_code: Some(0),
            }),
            title: title.to_string(),
            parts: vec![shell_action(1, None, Some(text), false)],
        }
    }

    /// Chat turn with typed parts: `(kind, text)` pairs. Built through the
    /// real reducer (`agent_parts` over `AgentBlock`s), so the passage shapes
    /// match what production parsing produces: a shell call folds its output
    /// blocks into one action, giving Command and Output/Error passages.
    /// An `error` part carries a parseable shell result JSON with a nonzero
    /// exit code, matching what providers actually emit for failures.
    fn chat_record(session: &str, index: usize, title: &str, parts: &[(&str, &str)]) -> WorkRecord {
        let kind = |name: &str| match name {
            "command" => AgentBlockKind::ToolCall,
            "output" | "error" => AgentBlockKind::ToolOutput,
            "user" => AgentBlockKind::User,
            "assistant" => AgentBlockKind::Assistant,
            _ => AgentBlockKind::Thinking,
        };
        let shell = |k: &str| matches!(k, "command" | "output" | "error");
        let mut blocks = Vec::new();
        for (k, text) in parts {
            blocks.push(AgentBlock {
                kind: kind(k),
                timestamp: None,
                label: shell(k).then(|| "bash".to_string()),
                call_id: shell(k).then(|| format!("call-{index}")),
                start_line: None,
                text: (*text).to_string(),
            });
        }
        let mut rec = record(session, index, title, "");
        rec.parts.clear();
        rec.parts = agent_parts(&blocks);
        rec
    }

    #[test]
    fn tokenizer_splits_and_lowercases() {
        let tokens = SimpleTokenizer.tokenize("Deploy-web: Foo.Bar_1");
        assert_eq!(tokens, vec!["deploy", "web", "foo", "bar"]);
    }

    #[test]
    fn tokenizer_drops_single_char_tokens() {
        // Lucene-style minimum token length: a degenerate `e` from `error[E`
        // must not survive into the query (near-zero df would dominate idf).
        let tokens = SimpleTokenizer.tokenize("error[E");
        assert_eq!(tokens, vec!["error"]);
        assert!(!tokens.contains(&"e".to_string()));
    }

    #[test]
    fn tokenizer_normalizes_case_for_error_codes() {
        // Query `error[E` must share the `error` token with `Error[E0428]`.
        let doc_tokens = SimpleTokenizer.tokenize("thread panicked: Error[E0428]");
        let query_tokens = SimpleTokenizer.tokenize("error[E");
        assert!(doc_tokens.contains(&"error".to_string()));
        assert!(query_tokens.contains(&"error".to_string()));
        assert!(doc_tokens.contains(&"e0428".to_string()));
        assert_eq!(query_tokens, vec!["error"]);
    }

    #[test]
    fn tokenizer_emits_cjk_bigrams() {
        assert_eq!(
            SimpleTokenizer.tokenize("重构逻辑"),
            vec!["重构", "构逻", "逻辑"]
        );
        assert_eq!(SimpleTokenizer.tokenize("重构"), vec!["重构"]);
        assert_eq!(SimpleTokenizer.tokenize("重构cargo"), vec!["重构", "cargo"]);
    }

    #[test]
    fn chinese_query_matches_documents_containing_the_words() {
        let corpus = vec![
            record("dev", 1, "重构下", "重构下，保证逻辑是最干净的"),
            record("dev", 2, "cargo build", "Finished dev profile"),
        ];
        let index = Bm25Index::build(&corpus);
        let ranked = rank_query(&index, "重构");
        assert_eq!(ranked[0].0.to_string(), "terminal/dev/1");
    }

    #[test]
    fn query_surfaces_matching_records_first() {
        let corpus = vec![
            record(
                "dev",
                1,
                "sqlite schema",
                "CREATE TABLE records (id TEXT PRIMARY KEY)",
            ),
            record(
                "dev",
                2,
                "rollback procedure",
                "kubectl rollout undo deploy/web",
            ),
            record("dev", 3, "git log", "7227bd8 refactor tui"),
        ];
        let index = Bm25Index::build(&corpus);
        let ranked = rank_query(&index, "rollback");
        assert_eq!(ranked[0].0.to_string(), "terminal/dev/2");
    }

    #[test]
    fn unmatched_query_scores_nothing() {
        let corpus = vec![record("dev", 1, "cargo build", "Finished dev profile")];
        let index = Bm25Index::build(&corpus);
        assert!(rank_query(&index, "zzzznothing").is_empty());
    }

    #[test]
    fn long_noise_document_does_not_outrank_short_match() {
        let long_noise =
            "command command command command command command command command command command \
            setup tooling docs and miscellaneous text";
        let corpus = vec![
            record("dev", 1, "command error", "command not found: get-item"),
            record("dev", 2, "unrelated", "the found command ran fine"),
            record("dev", 3, "setup log", long_noise),
        ];
        let index = Bm25Index::build(&corpus);
        let ranked = rank_query(&index, "command not found");
        assert_eq!(ranked[0].0.to_string(), "terminal/dev/1");
    }

    #[test]
    fn multi_word_query_prefers_documents_matching_all_terms() {
        let corpus = vec![
            record("dev", 1, "command error", "command not found: get-item"),
            record("dev", 2, "unrelated", "the found command ran fine"),
        ];
        let index = Bm25Index::build(&corpus);
        let ranked = rank_query(&index, "command not found");
        assert_eq!(ranked[0].0.to_string(), "terminal/dev/1");
    }

    #[test]
    fn late_term_in_long_document_is_still_indexed_and_ranked() {
        let mut noise = String::new();
        for i in 0..2_000 {
            noise.push_str(&format!("padding token {i} "));
        }
        let long_turn = format!("{noise}panicked at src/main.rs:42");
        let corpus = vec![
            record("dev", 1, "long agent turn", &long_turn),
            record("dev", 2, "short chatter", "no signal here at all"),
        ];
        let index = Bm25Index::build(&corpus);
        let ranked = rank_query(&index, "panicked");
        assert_eq!(ranked[0].0.to_string(), "terminal/dev/1");
    }

    #[test]
    fn matching_long_document_outranks_nonmatching_short_document() {
        // BM25+ lower bound: a long document that does match the query is
        // scored strictly above a short document that does not match at all.
        let mut noise = String::new();
        for i in 0..3_000 {
            noise.push_str(&format!("filler token {i} "));
        }
        let long_match = format!("{noise}kubectl gotcha at the very end");
        let corpus = vec![
            record("dev", 1, "long session", &long_match),
            record("dev", 2, "short unrelated", "just a short line"),
        ];
        let index = Bm25Index::build(&corpus);
        let ranked = rank_query(&index, "kubectl");
        assert_eq!(ranked[0].0.to_string(), "terminal/dev/1");
        assert_eq!(ranked.len(), 1, "non-matching doc must not score");
    }

    #[test]
    fn title_field_boost_beats_body_only_mention() {
        let corpus = vec![
            record("dev", 1, "sivtr serve status", "ok"),
            record(
                "dev",
                2,
                "chatter",
                "user asked about sivtr serve status and remote list today",
            ),
        ];
        let index = Bm25Index::build(&corpus);
        let ranked = rank_query(&index, "sivtr serve status");
        assert_eq!(ranked[0].0.to_string(), "terminal/dev/1");
    }

    #[test]
    fn rank_terms_applies_per_term_weights() {
        let corpus = vec![
            record("dev", 1, "fix", "cargo run panicked"),
            record("dev", 2, "other", "cargo build passed"),
        ];
        let index = Bm25Index::build(&corpus);
        let terms = vec![("cargo".to_string(), 1.0), ("panicked".to_string(), 1.0)];
        let ranked = index.rank_terms_with(&terms, TITLE_WEIGHT);
        assert_eq!(ranked[0].0.to_string(), "terminal/dev/1");
    }

    #[test]
    fn mid_conversation_term_in_own_part_is_indexed() {
        // Passage retrieval: the query term sits in the middle of a long turn,
        // inside its own bounded part. A flat concatenation with a positional
        // window would drop it; per-part indexing must surface it.
        let mut noise = String::new();
        for i in 0..2_500 {
            noise.push_str(&format!("noise token {i} "));
        }
        let corpus = vec![
            record("dev", 1, "first turn", &noise),
            chat_record(
                "dev",
                2,
                "long turn",
                &[
                    ("user", "start the session and keep going"),
                    (
                        "assistant",
                        &format!("{noise}kubectl connection refused at 1333"),
                    ),
                    ("assistant", "and later discussion continues"),
                ],
            ),
        ];
        let index = Bm25Index::build(&corpus);
        let ranked = rank_query(&index, "kubectl");
        assert_eq!(
            ranked[0].0.to_string(),
            "terminal/dev/2",
            "mid-turn term in its own part must rank first"
        );
    }

    #[test]
    fn command_part_wins_over_body_chatter() {
        // The Command part is the executed command: it outranks a long chat
        // turn that merely mentions the same words many times across parts
        // (max passage aggregation keeps the noise bounded).
        let mut chatter = String::new();
        for _ in 0..50 {
            chatter.push_str("sivtr serve status and check the daemon ");
        }
        let corpus = vec![
            chat_record(
                "dev",
                1,
                "server run",
                &[
                    ("command", "sivtr serve status"),
                    ("output", "daemon running on 127.0.0.1:17421"),
                ],
            ),
            chat_record(
                "dev",
                2,
                "discussion",
                &[
                    ("user", "how does serve work"),
                    ("assistant", &chatter),
                    ("assistant", &chatter),
                ],
            ),
        ];
        let index = Bm25Index::build(&corpus);
        let ranked = rank_query(&index, "sivtr serve status");
        assert_eq!(ranked[0].0.to_string(), "terminal/dev/1");
    }

    #[test]
    fn short_strong_passage_outranks_long_mention() {
        // Content kinds are weighted equally; a short passage holding the full
        // query (the error text) beats a longer reasoning passage that only
        // mentions the words — per-passage length normalization.
        let corpus = vec![
            chat_record(
                "dev",
                1,
                "error surface",
                &[(
                    "error",
                    r#"{"stderr":"connection refused: no route to host","exit_code":1}"#,
                )],
            ),
            chat_record(
                "dev",
                2,
                "reasoning",
                &[(
                    "thinking",
                    "connection refused might mean the port is closed",
                )],
            ),
        ];
        let index = Bm25Index::build(&corpus);
        let ranked = rank_query(&index, "connection refused");
        assert_eq!(ranked[0].0.to_string(), "terminal/dev/1");
    }
}
