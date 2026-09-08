//! Semantic and hybrid ranking over the archive-backed WorkSet corpus.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::Value;
use sivtr_core::archive::semantic as embedding_store;
use sivtr_core::config::SivtrConfig;
use sivtr_core::record::{WorkRecord, WorkRef};
use sivtr_core::search::{Field, Filter, Searcher, Sort};
use sivtr_core::workset::WorkSet;

use crate::cli::SearchArgs;
use crate::commands::memory::{filter, workset};

const DEFAULT_RRF_K: f64 = 60.0;
const EMBEDDING_RESPONSE_LIMIT: u64 = 64 * 1024 * 1024;

pub fn run(args: &SearchArgs) -> Result<WorkSet> {
    if args.semantic && args.hybrid {
        bail!("semantic and hybrid search are mutually exclusive");
    }
    let query = args
        .query
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .context("semantic and hybrid search require a non-empty QUERY")?;

    let mut corpus_filter = filter::from_search_args(args)?;
    let result_limit = corpus_filter.limit.or(corpus_filter.latest).unwrap_or(5);
    let field = corpus_filter.in_field;
    corpus_filter.rank = None;
    corpus_filter.sort = Sort::Newest;
    corpus_filter.latest = None;
    corpus_filter.limit = None;

    let set = workset::query(&args.source, corpus_filter.clone(), args.cwd.as_deref())?;
    let cwd_string = set.cwd().to_string();
    let cwd = Path::new(&cwd_string);
    let (records, anchors) = set.into_parts();
    if anchors.iter().any(|anchor| !anchor.is_local()) {
        bail!("semantic and hybrid search require a local source");
    }

    let client = EmbeddingClient::from_config()?;
    let mut query_vectors = client.embed(&[query.to_string()])?;
    let query_vector = query_vectors
        .pop()
        .context("embedding service returned no query vector")?;
    let conn = sivtr_core::archive::open()?;
    embedding_store::ensure_index(&conn, &client.model, query_vector.len())?;
    let mut embeddings = embedding_store::load_embeddings(&conn)?;

    let mut missing = Vec::new();
    for record in &records {
        let reference = record.work_ref.whole().to_string();
        let text = embedding_text(record, field);
        if text.trim().is_empty() {
            continue;
        }
        let hash = embedding_store::text_hash(&text);
        if embeddings
            .get(&reference)
            .is_some_and(|(stored_hash, _)| stored_hash == &hash)
        {
            continue;
        }
        missing.push((reference, hash, text));
    }

    if !missing.is_empty() {
        let texts: Vec<String> = missing.iter().map(|(_, _, text)| text.clone()).collect();
        let vectors = client.embed(&texts)?;
        if vectors.len() != missing.len() {
            bail!(
                "embedding service returned {} vectors for {} records",
                vectors.len(),
                missing.len()
            );
        }
        let rows: Vec<_> = missing
            .into_iter()
            .zip(vectors)
            .map(|((reference, hash, _), vector)| (reference, hash, vector))
            .collect();
        embedding_store::upsert_embeddings(&conn, &rows)?;
        embeddings = embedding_store::load_embeddings(&conn)?;
    }

    let semantic_rank = rank_semantic(&records, &anchors, &embeddings, &query_vector)?;
    let ranked = if args.hybrid {
        let mut bm25_filter = corpus_filter;
        bm25_filter.rank = Some(query.to_string());
        bm25_filter.sort = Sort::Relevance;
        let bm25 = Searcher::new(&records).search(&bm25_filter, &anchors, cwd)?;
        fuse_rrf(
            &bm25.into_iter().map(|hit| hit.anchor).collect::<Vec<_>>(),
            &semantic_rank,
        )
    } else {
        semantic_rank
            .into_iter()
            .map(|(reference, _)| reference)
            .collect()
    };

    let ranked = ranked.into_iter().take(result_limit).collect();
    Ok(WorkSet::from_parts(cwd_string, records, ranked))
}

pub fn rank_records_for_eval(
    records: &[WorkRecord],
    query: &str,
    field: Field,
    hybrid: bool,
) -> Result<Vec<String>> {
    let client = EmbeddingClient::from_config()?;
    let mut query_vectors = client.embed(&[query.to_string()])?;
    let query_vector = query_vectors
        .pop()
        .context("embedding service returned no query vector")?;
    let anchors: Vec<WorkRef> = records
        .iter()
        .map(|record| record.work_ref.whole())
        .collect();
    let mut inputs = Vec::new();
    for record in records {
        let text = embedding_text(record, field);
        if !text.trim().is_empty() {
            inputs.push((record.work_ref.whole().to_string(), text));
        }
    }
    let texts: Vec<String> = inputs.iter().map(|(_, text)| text.clone()).collect();
    let vectors = client.embed(&texts)?;
    if vectors.len() != inputs.len() {
        bail!("embedding service returned an incomplete evaluation result");
    }
    let embeddings = inputs
        .into_iter()
        .zip(vectors)
        .map(|((reference, _), vector)| (reference, (String::new(), vector)))
        .collect::<HashMap<_, _>>();
    let semantic = rank_semantic(records, &anchors, &embeddings, &query_vector)?;
    if !hybrid {
        return Ok(semantic
            .into_iter()
            .map(|(reference, _)| reference.to_string())
            .collect());
    }
    let bm25_filter = Filter {
        rank: Some(query.to_string()),
        in_field: field,
        sort: Sort::Relevance,
        ..Filter::none()
    };
    let bm25 = Searcher::new(records)
        .search(&bm25_filter, &anchors, Path::new("."))?
        .into_iter()
        .map(|hit| hit.anchor)
        .collect::<Vec<_>>();
    Ok(fuse_rrf(&bm25, &semantic)
        .into_iter()
        .map(|reference| reference.to_string())
        .collect())
}

fn embedding_text(record: &WorkRecord, field: Field) -> String {
    match field {
        Field::Content => record.combined_text(),
        Field::Title => record.title.clone(),
        Field::Session => record.session.id.clone(),
        Field::Input | Field::Command => record.input_text().unwrap_or_default(),
        Field::Output => record.output_text().unwrap_or_default(),
        Field::All => format!(
            "{}\n{}\n{}\n{}",
            record.title,
            record.session.id,
            record.input_text().unwrap_or_default(),
            record.output_text().unwrap_or_default()
        ),
    }
}

fn rank_semantic(
    records: &[WorkRecord],
    anchors: &[WorkRef],
    embeddings: &HashMap<String, (String, Vec<f32>)>,
    query: &[f32],
) -> Result<Vec<(WorkRef, f32)>> {
    let mut ranked = Vec::new();
    for anchor in anchors {
        let reference = anchor.whole().to_string();
        let Some((_, vector)) = embeddings.get(&reference) else {
            continue;
        };
        ranked.push((
            anchor.clone(),
            embedding_store::cosine_similarity(vector, query)?,
        ));
    }
    ranked.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.to_string().cmp(&right.0.to_string()))
    });
    if ranked.is_empty() && !records.is_empty() {
        bail!("no record has a usable embedding");
    }
    Ok(ranked)
}

fn fuse_rrf(bm25: &[WorkRef], semantic: &[(WorkRef, f32)]) -> Vec<WorkRef> {
    let mut scores: HashMap<WorkRef, f64> = HashMap::new();
    for (rank, reference) in bm25.iter().enumerate() {
        *scores.entry(reference.clone()).or_default() += 1.0 / (DEFAULT_RRF_K + rank as f64 + 1.0);
    }
    for (rank, (reference, _)) in semantic.iter().enumerate() {
        *scores.entry(reference.clone()).or_default() += 1.0 / (DEFAULT_RRF_K + rank as f64 + 1.0);
    }
    let mut ranked: Vec<_> = scores.into_iter().collect();
    ranked.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.to_string().cmp(&right.0.to_string()))
    });
    ranked.into_iter().map(|(reference, _)| reference).collect()
}

struct EmbeddingClient {
    endpoint: String,
    model: String,
    api_key: Option<String>,
    batch_size: usize,
    agent: ureq::Agent,
}

impl EmbeddingClient {
    fn from_config() -> Result<Self> {
        let config = SivtrConfig::load()?.embedding;
        let endpoint = config.endpoint.trim().to_string();
        if endpoint.is_empty() {
            bail!("semantic search requires [embedding].endpoint in config.toml");
        }
        if !endpoint.starts_with("https://") && !is_loopback_http(&endpoint) {
            bail!("[embedding].endpoint must use https:// (http:// is allowed only for loopback)");
        }
        if config.model.trim().is_empty() {
            bail!("[embedding].model must not be empty");
        }
        if config.batch_size == 0 {
            bail!("[embedding].batch_size must be greater than zero");
        }
        let api_key = if config.api_key_env.trim().is_empty() {
            None
        } else {
            let key = std::env::var(&config.api_key_env).with_context(|| {
                format!(
                    "embedding API key environment variable `{}` is not set",
                    config.api_key_env
                )
            })?;
            if key.trim().is_empty() {
                bail!(
                    "embedding API key environment variable `{}` is empty",
                    config.api_key_env
                );
            }
            Some(key)
        };
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .build()
            .new_agent();
        Ok(Self {
            endpoint,
            model: config.model,
            api_key,
            batch_size: config.batch_size,
            agent,
        })
    }

    fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut all = Vec::with_capacity(inputs.len());
        for chunk in inputs.chunks(self.batch_size) {
            let body = serde_json::to_vec(&serde_json::json!({
                "model": self.model,
                "input": chunk,
            }))?;
            let mut request = self
                .agent
                .post(&self.endpoint)
                .header("Content-Type", "application/json");
            if let Some(api_key) = &self.api_key {
                request = request.header("Authorization", &format!("Bearer {api_key}"));
            }
            let mut response = request
                .send(body)
                .with_context(|| format!("embedding request failed: {}", self.endpoint))?;
            if !response.status().is_success() {
                bail!("embedding service returned HTTP {}", response.status());
            }
            let text = response
                .body_mut()
                .with_config()
                .limit(EMBEDDING_RESPONSE_LIMIT)
                .read_to_string()
                .context("failed to read embedding response")?;
            all.extend(parse_embedding_response(&text)?);
        }
        Ok(all)
    }
}

fn parse_embedding_response(text: &str) -> Result<Vec<Vec<f32>>> {
    let value: Value = serde_json::from_str(text).context("embedding response is not JSON")?;
    let mut indexes = std::collections::HashSet::new();
    let mut entries = value
        .get("data")
        .and_then(Value::as_array)
        .context("embedding response has no data array")?
        .iter()
        .enumerate()
        .map(|(position, entry)| {
            let index = entry
                .get("index")
                .and_then(Value::as_u64)
                .with_context(|| format!("embedding response item {position} has no index"))?;
            if !indexes.insert(index) {
                bail!("embedding response contains duplicate index {index}");
            }
            let embedding = entry
                .get("embedding")
                .and_then(Value::as_array)
                .context("embedding response item has no embedding array")?
                .iter()
                .map(|value| {
                    let value = value.as_f64().context("embedding value is not a number")?;
                    if !value.is_finite() {
                        bail!("embedding value is not finite");
                    }
                    let value = value as f32;
                    if !value.is_finite() {
                        bail!("embedding value exceeds f32 range");
                    }
                    Ok(value)
                })
                .collect::<Result<Vec<_>>>()?;
            Ok((index, embedding))
        })
        .collect::<Result<Vec<_>>>()?;
    entries.sort_by_key(|(index, _)| *index);
    for (expected, (index, _)) in entries.iter().enumerate() {
        if *index != expected as u64 {
            bail!("embedding response indexes must be contiguous from zero");
        }
    }
    Ok(entries
        .into_iter()
        .map(|(_, embedding)| embedding)
        .collect())
}

fn is_loopback_http(endpoint: &str) -> bool {
    let Some(authority) = endpoint
        .strip_prefix("http://")
        .and_then(|value| value.split('/').next())
    else {
        return false;
    };
    let host = authority
        .rsplit_once(':')
        .map(|(host, _)| host)
        .unwrap_or(authority)
        .trim_matches(['[', ']']);
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_orders_embedding_response_by_index() {
        let value =
            r#"{"data":[{"index":1,"embedding":[0.0,1.0]},{"index":0,"embedding":[1.0,0.0]}]}"#;
        assert_eq!(
            parse_embedding_response(value).unwrap(),
            vec![vec![1.0, 0.0], vec![0.0, 1.0]]
        );
    }

    #[test]
    fn rejects_non_contiguous_embedding_indexes() {
        let value = r#"{"data":[{"index":1,"embedding":[1.0,0.0]}]}"#;
        assert!(parse_embedding_response(value).is_err());
    }

    #[test]
    fn rrf_keeps_deterministic_union() {
        let a: WorkRef = "codex/a/1".parse().unwrap();
        let b: WorkRef = "codex/b/1".parse().unwrap();
        let ranked = fuse_rrf(&[b.clone(), a.clone()], &[(a.clone(), 0.9)]);
        assert_eq!(ranked, vec![a, b]);
    }
}
