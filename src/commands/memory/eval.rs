//! Retrieval quality benchmark: runs labeled queries through the real search
//! pipeline against a frozen snapshot of real records.
//!
//! Single-pipeline by design: [`filter::apply`] is the same search path
//! `sivtr search` uses. Nothing here reimplements matching or ranking.

use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use sivtr_core::record::{WorkRecord, WorkRef};
use sivtr_core::search::{
    evaluate_with_ranked, EvalReport, EvalSnapshot, Filter, GoldenQuery, Searcher, Sort,
};

use crate::cli::{EvalArgs, EvalMethod};
use crate::commands::memory::{filter, workset};

pub fn execute(args: &EvalArgs) -> Result<()> {
    if let Some(path) = args.create_snapshot.as_deref() {
        return create_snapshot(path);
    }

    let Some(path) = args.snapshot.as_deref() else {
        bail!("no snapshot; run `sivtr eval --create-snapshot <path>` first, then `sivtr eval --snapshot <path>`");
    };
    let snapshot = load_snapshot(path)?;
    if snapshot.queries.is_empty() {
        bail!(
            "snapshot {} has no queries; add labeled queries (`{{name, query, field, relevant}}`)",
            path.display()
        );
    }

    let k = args.k.max(1);
    let anchors: Vec<_> = snapshot
        .corpus
        .iter()
        .map(|record| record.work_ref.whole())
        .collect();
    let ranked = rank_all(
        &snapshot.queries,
        &snapshot.corpus,
        &anchors,
        args.sort,
        k,
        args.method,
    )?;
    if let Some(dir) = args.export.as_deref() {
        export_trec(dir, &snapshot.queries, &ranked)?;
    }

    let report = evaluate_with_ranked(&snapshot.queries, &ranked, k);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_table(&report, &args.sort);
    }
    Ok(())
}

/// Dump the current workspace records (terminal + agent) into a snapshot file.
/// Queries start empty so they can be labeled against the frozen corpus.
fn create_snapshot(path: &Path) -> Result<()> {
    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    let mut records = Vec::new();
    for source in ["terminal", "agent"] {
        let set = workset::query(source, filter::Filter::none(), Some(&cwd))?;
        records.extend(set.into_records());
    }
    let snapshot = EvalSnapshot {
        queries: Vec::new(),
        corpus: records,
    };
    let json = serde_json::to_string_pretty(&snapshot).context("Failed to serialize snapshot")?;
    fs::write(path, json)
        .with_context(|| format!("Failed to write snapshot: {}", path.display()))?;
    println!(
        "wrote {} records to {}; add labeled queries before evaluating",
        snapshot.corpus.len(),
        path.display()
    );
    Ok(())
}

fn load_snapshot(path: &Path) -> Result<EvalSnapshot> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("Failed to read snapshot: {}", path.display()))?;
    serde_json::from_str(&text).context("Invalid snapshot JSON")
}

/// One search implementation: the same `Searcher` path `sivtr search` runs.
/// Each golden query ranks the whole field-scoped corpus under `sort` and keeps
/// the top `k`, so the metrics measure ranking quality, not filter recall.
/// One `Searcher` is shared across queries so the BM25 index builds once.
/// Returns ranked WorkRef strings per query, aligned by index.
fn rank_all(
    queries: &[GoldenQuery],
    corpus: &[WorkRecord],
    anchors: &[WorkRef],
    sort: Sort,
    k: usize,
    method: EvalMethod,
) -> Result<Vec<Vec<String>>> {
    match method {
        EvalMethod::Bm25 => {
            let searcher = Searcher::new(corpus);
            queries
                .iter()
                .map(|query| {
                    let filter = Filter::eval(&query.query, query.field, sort, k);
                    searcher
                        .search(&filter, anchors, Path::new("."))
                        .map(|hits| hits.into_iter().map(|hit| hit.anchor.to_string()).collect())
                })
                .collect()
        }
        EvalMethod::Semantic | EvalMethod::Hybrid => queries
            .iter()
            .map(|query| {
                let mut ranked = crate::commands::memory::semantic::rank_records_for_eval(
                    corpus,
                    &query.query,
                    query.field,
                    matches!(method, EvalMethod::Hybrid),
                )?;
                ranked.truncate(k);
                Ok(ranked)
            })
            .collect(),
    }
}

fn print_table(report: &EvalReport, sort: &Sort) {
    let k = report.k;
    println!("retrieval eval: k={k}, sort={sort}");
    println!(
        "{:<28} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "query",
        "relevant",
        "retrieved",
        format!("recall@{k}"),
        format!("prec@{k}"),
        "mrr",
        format!("ndcg@{k}"),
    );
    for query in &report.queries {
        println!(
            "{:<28} {:>9} {:>9} {:>9.3} {:>9.3} {:>9.3} {:>9.3}",
            query.name,
            query.relevant,
            query.retrieved,
            query.recall_at_k,
            query.precision_at_k,
            query.mrr,
            query.ndcg_at_k,
        );
    }
    let aggregate = &report.aggregate;
    println!(
        "{:<28} {:>9} {:>9} {:>9.3} {:>9.3} {:>9.3} {:>9.3}",
        "mean",
        "",
        "",
        aggregate.mean_recall_at_k,
        aggregate.mean_precision_at_k,
        aggregate.mean_mrr,
        aggregate.mean_ndcg_at_k,
    );
}

/// Write qrels.txt and results.txt in trec_eval format so the same snapshot can
/// be cross-checked with standard tools (`trec_eval`, `ir_measures`).
fn export_trec(dir: &Path, queries: &[GoldenQuery], ranked: &[Vec<String>]) -> Result<()> {
    fs::create_dir_all(dir)
        .with_context(|| format!("Failed to create export dir: {}", dir.display()))?;

    let mut qrels = String::new();
    for (query_index, query) in queries.iter().enumerate() {
        for ref_ in &query.relevant {
            qrels.push_str(&format!("{} 0 {} 1\n", query_id(query_index), ref_));
        }
    }
    let qrels_path = dir.join("qrels.txt");
    fs::write(&qrels_path, qrels)
        .with_context(|| format!("Failed to write {}", qrels_path.display()))?;

    let mut results = String::new();
    for (query_index, ranked) in ranked.iter().enumerate() {
        let total = ranked.len().max(1);
        for (rank, ref_) in ranked.iter().enumerate() {
            // Score is rank-derived so trec_eval orders identically to the pipeline.
            let score = (total - rank) as f64;
            results.push_str(&format!(
                "{} Q0 {} {} {score:.3} sivtr\n",
                query_id(query_index),
                ref_,
                rank + 1
            ));
        }
    }
    let results_path = dir.join("results.txt");
    fs::write(&results_path, results)
        .with_context(|| format!("Failed to write {}", results_path.display()))?;
    Ok(())
}

/// trec_eval ids are whitespace-free; query names may contain spaces.
fn query_id(index: usize) -> String {
    format!("q{index}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::search::Field;

    #[test]
    fn query_ids_are_whitespace_free() {
        assert_eq!(query_id(0), "q0");
        assert_eq!(query_id(12), "q12");
    }

    #[test]
    fn export_writes_trec_formats() {
        let queries = vec![GoldenQuery {
            name: "panic".into(),
            query: "panic".into(),
            field: Field::Content,
            relevant: vec!["terminal/dev/2".into(), "codex/abc123/1".into()],
        }];
        let ranked = vec![vec![
            "terminal/dev/2".to_string(),
            "terminal/dev/3".to_string(),
        ]];
        let dir = std::env::temp_dir().join(format!("sivtr-eval-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        export_trec(&dir, &queries, &ranked).expect("export");
        let qrels = fs::read_to_string(dir.join("qrels.txt")).expect("qrels");
        let results = fs::read_to_string(dir.join("results.txt")).expect("results");
        assert!(qrels.contains("q0 0 terminal/dev/2 1\n"));
        assert!(results.contains("q0 Q0 terminal/dev/2 1 2.000 sivtr\n"));
        assert!(results.contains("q0 Q0 terminal/dev/3 2 1.000 sivtr\n"));
        let _ = fs::remove_dir_all(&dir);
    }
}
