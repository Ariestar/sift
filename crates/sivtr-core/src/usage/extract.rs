//! Per-provider usage extraction from native transcripts.
//!
//! One walker per provider whose transcript carries per-call token usage.
//! Walkers skip well-formed records without usage, but malformed transcript
//! data fails the source so accounting is never silently incomplete. The
//! normalized event shape mirrors the Anthropic billing model (`input`
//! excludes cache reads; the cached portion is billed as `cache_read`),
//! matching agentsview's normalization.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

use super::UsageEvent;

/// Extract usage events from one provider transcript. Returns events in
/// transcript order; duplicates carry identical `dedup_key`s and collapse
/// at insert time.
pub fn extract(provider: &str, path: &Path) -> Result<Vec<UsageEvent>> {
    match provider {
        "claude" => claude(path),
        "codex" | "traex" => codex(path),
        "amp" => amp(path),
        "gptme" => gptme(path),
        "vscode-copilot" | "trae" => vscode_copilot(path),
        "kimi" | "kimi-work" => kimi(path),
        "vibe" => vibe(path),
        "poolside" => poolside(path),
        // Providers without a reliable per-call usage record contribute no
        // events. Their transcript data is still archived normally.
        _ => Ok(Vec::new()),
    }
}

fn jsonl_lines(path: &Path) -> Result<Vec<String>> {
    let file = std::fs::File::open(path)?;
    BufReader::new(file)
        .lines()
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(Into::into)
}

/// Claude Code: one JSONL line per API response.
///
/// ```json
/// {"type":"assistant","timestamp":"…","message":{"id":"msg_…","model":"…",
///  "usage":{"input_tokens":4,"cache_creation_input_tokens":232,"cache_read_input_tokens":10000,"output_tokens":91}}}
/// ```
///
/// Streaming re-emits the same message id with growing usage — the last
/// line for an id wins.
fn claude(path: &Path) -> Result<Vec<UsageEvent>> {
    let mut by_id: HashMap<String, UsageEvent> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for (index, line) in jsonl_lines(path)?.into_iter().enumerate() {
        let value = parse_line(&line, path, index)?;
        let Some(message) = value.get("message") else {
            continue;
        };
        let Some(usage) = message.get("usage") else {
            continue;
        };
        if [
            "input_tokens",
            "output_tokens",
            "cache_creation_input_tokens",
            "cache_read_input_tokens",
        ]
        .iter()
        .all(|field| usage.get(field).is_none())
        {
            continue;
        }
        let id = message
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let key = if id.is_empty() {
            format!("line:{index}")
        } else {
            id
        };
        let event = UsageEvent {
            model: message
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            input_tokens: num(usage.get("input_tokens")),
            output_tokens: num(usage.get("output_tokens")),
            cache_creation_tokens: num(usage.get("cache_creation_input_tokens")),
            cache_read_tokens: num(usage.get("cache_read_input_tokens")),
            occurred_at: value
                .get("timestamp")
                .and_then(Value::as_str)
                .map(str::to_string),
            dedup_key: format!("claude:{key}"),
        };
        if !by_id.contains_key(&key) {
            order.push(key.clone());
        }
        by_id.insert(key, event);
    }
    Ok(order
        .into_iter()
        .filter_map(|key| by_id.remove(&key))
        .collect())
}

/// Codex: `token_count` events carry `last_token_usage`; the model comes
/// from `turn_context` events. Codex reports `input_tokens` as the full
/// input count including the cached portion, so the cached part is moved
/// to `cache_read_tokens` to avoid double-billing at the full input rate.
fn codex(path: &Path) -> Result<Vec<UsageEvent>> {
    let mut model = String::new();
    let mut events: Vec<UsageEvent> = Vec::new();
    let mut seen_keys = std::collections::HashSet::new();
    for (index, line) in jsonl_lines(path)?.into_iter().enumerate() {
        let value = parse_line(&line, path, index)?;
        match value.get("type").and_then(Value::as_str) {
            Some("turn_context") => {
                if let Some(name) = value
                    .get("payload")
                    .and_then(|p| p.get("model"))
                    .and_then(Value::as_str)
                {
                    model = name.to_string();
                }
            }
            Some("event_msg") => {
                let Some(payload) = value.get("payload") else {
                    continue;
                };
                if payload.get("type").and_then(Value::as_str) != Some("token_count") {
                    continue;
                }
                let Some(usage) = payload
                    .get("info")
                    .and_then(|info| info.get("last_token_usage"))
                else {
                    continue;
                };
                let total_input = num(usage.get("input_tokens"));
                let cached = num(usage.get("cached_input_tokens"));
                let output = num(usage.get("output_tokens"));
                if total_input == 0 && cached == 0 && output == 0 {
                    continue;
                }
                let timestamp = value
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let dedup_key = format!("codex:{model}:{total_input}:{cached}:{output}");
                if !seen_keys.insert(dedup_key.clone()) {
                    continue;
                }
                events.push(UsageEvent {
                    model: model.clone(),
                    input_tokens: total_input.saturating_sub(cached),
                    output_tokens: output,
                    cache_read_tokens: cached,
                    cache_creation_tokens: 0,
                    occurred_at: Some(timestamp).filter(|ts| !ts.is_empty()),
                    dedup_key,
                });
            }
            _ => {}
        }
    }
    Ok(events)
}

fn amp(path: &Path) -> Result<Vec<UsageEvent>> {
    let document = read_json_document(path)?;
    let messages = document
        .get("messages")
        .and_then(Value::as_array)
        .context("Amp thread has no messages array")?;
    let mut events = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        let Some(usage) = message.get("usage") else {
            continue;
        };
        if !has_any(
            usage,
            &[
                "inputTokens",
                "outputTokens",
                "cacheReadInputTokens",
                "cacheCreationInputTokens",
            ],
        ) {
            continue;
        }
        let model = string_value(usage, "model").unwrap_or_default();
        let mut input = num(usage.get("inputTokens"));
        let mut cache_creation = num(usage.get("cacheCreationInputTokens"));
        let cache_read = num(usage.get("cacheReadInputTokens"));
        // Amp's OpenAI-family records put uncached prompt tokens in the
        // cache-creation bucket. That is a provider wire-format rule, not a
        // pricing guess, so normalize it before the shared cost engine.
        if model.to_ascii_lowercase().starts_with("gpt-") {
            input = input.saturating_add(cache_creation);
            cache_creation = 0;
        }
        events.push(UsageEvent {
            model,
            input_tokens: input,
            output_tokens: num(usage.get("outputTokens")),
            cache_read_tokens: cache_read,
            cache_creation_tokens: cache_creation,
            occurred_at: string_value(usage, "timestamp")
                .or_else(|| string_value(message, "timestamp")),
            dedup_key: format!("amp:{index}"),
        });
    }
    Ok(events)
}

fn gptme(path: &Path) -> Result<Vec<UsageEvent>> {
    let mut events = Vec::new();
    for (index, line) in jsonl_lines(path)?.into_iter().enumerate() {
        let value = parse_line(&line, path, index)?;
        if value.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(usage) = value.pointer("/metadata/usage") else {
            continue;
        };
        if !has_any(
            usage,
            &[
                "input_tokens",
                "output_tokens",
                "cache_read_tokens",
                "cache_creation_tokens",
            ],
        ) {
            continue;
        }
        events.push(UsageEvent {
            model: value
                .pointer("/metadata/model")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            input_tokens: num(usage.get("input_tokens")),
            output_tokens: num(usage.get("output_tokens")),
            cache_read_tokens: num(usage.get("cache_read_tokens")),
            cache_creation_tokens: num(usage.get("cache_creation_tokens")),
            occurred_at: string_value(&value, "timestamp"),
            dedup_key: format!("gptme:{index}"),
        });
    }
    Ok(events)
}

fn vscode_copilot(path: &Path) -> Result<Vec<UsageEvent>> {
    let documents = if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
        vec![read_json_document(path)?]
    } else {
        jsonl_lines(path)?
            .into_iter()
            .enumerate()
            .map(|(index, line)| parse_line(&line, path, index))
            .collect::<Result<Vec<_>>>()?
    };
    let mut events = Vec::new();
    for document in documents {
        let Some(requests) = document.get("requests").and_then(Value::as_array) else {
            continue;
        };
        for (index, request) in requests.iter().enumerate() {
            let Some(metadata) = request
                .pointer("/result/metadata")
                .or_else(|| request.get("metadata"))
            else {
                continue;
            };
            let Some(input) = first_num(metadata, &["promptTokens", "inputTokens"]) else {
                continue;
            };
            let Some(output) = first_num(metadata, &["outputTokens"]) else {
                continue;
            };
            events.push(UsageEvent {
                model: string_value(metadata, "resolvedModel")
                    .or_else(|| string_value(request, "modelId"))
                    .unwrap_or_default(),
                input_tokens: input,
                output_tokens: output,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                occurred_at: string_value(request, "timestamp"),
                dedup_key: format!(
                    "vscode-copilot:{}",
                    string_value(request, "requestId").unwrap_or_else(|| index.to_string())
                ),
            });
        }
    }
    Ok(events)
}

fn kimi(path: &Path) -> Result<Vec<UsageEvent>> {
    let mut events = Vec::new();
    for (index, line) in jsonl_lines(path)?.into_iter().enumerate() {
        let value = parse_line(&line, path, index)?;
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let usage = match kind {
            "usage.record" => value.get("usage"),
            "step.end" => value.get("usage"),
            _ => None,
        };
        let Some(usage) = usage else { continue };
        if !has_any(
            usage,
            &[
                "inputOther",
                "output",
                "inputCacheRead",
                "inputCacheCreation",
            ],
        ) {
            continue;
        }
        events.push(UsageEvent {
            model: string_value(&value, "model").unwrap_or_default(),
            input_tokens: num(usage.get("inputOther")),
            output_tokens: num(usage.get("output")),
            cache_read_tokens: num(usage.get("inputCacheRead")),
            cache_creation_tokens: num(usage.get("inputCacheCreation")),
            occurred_at: json_timestamp(&value),
            dedup_key: format!("kimi:{index}"),
        });
    }
    Ok(events)
}

fn vibe(path: &Path) -> Result<Vec<UsageEvent>> {
    let metadata_path = path
        .parent()
        .map(|parent| parent.join("meta.json"))
        .context("Vibe transcript has no session directory")?;
    let document: Value = serde_json::from_str(
        &std::fs::read_to_string(&metadata_path)
            .with_context(|| format!("failed to read {}", metadata_path.display()))?,
    )
    .with_context(|| format!("failed to parse {}", metadata_path.display()))?;
    let stats = document
        .get("stats")
        .context("Vibe metadata has no stats")?;
    let input = num(stats.get("session_prompt_tokens"));
    let output = num(stats.get("session_completion_tokens"));
    let cache_read = num(stats.get("session_cached_tokens"));
    if input == 0 && output == 0 && cache_read == 0 {
        return Ok(Vec::new());
    }
    Ok(vec![UsageEvent {
        model: string_value(&document, "model")
            .or_else(|| {
                document
                    .pointer("/config/active_model")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_default(),
        input_tokens: input.saturating_sub(cache_read),
        output_tokens: output,
        cache_read_tokens: cache_read,
        cache_creation_tokens: 0,
        occurred_at: string_value(&document, "end_time"),
        dedup_key: format!("vibe:{}", path.display()),
    }])
}

fn poolside(path: &Path) -> Result<Vec<UsageEvent>> {
    let mut models = HashMap::new();
    let mut events = Vec::new();
    for (index, line) in jsonl_lines(path)?.into_iter().enumerate() {
        let value = parse_line(&line, path, index)?;
        match value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "tool_call.inference.start" => {
                if let Some(model) = value
                    .pointer("/tool_call_inference_start/chat_completion_request/model")
                    .and_then(Value::as_str)
                    .or_else(|| value.get("model").and_then(Value::as_str))
                {
                    if let Some(step) = value.get("step_id").and_then(Value::as_str) {
                        models.insert(step.to_string(), model.to_string());
                    }
                }
            }
            "tool_call.inference.end" => {
                let input = num(value.pointer("/tool_call_inference_end/input_tokens"));
                let output = num(value.pointer("/tool_call_inference_end/output_tokens"));
                let cache_read =
                    num(value.pointer("/tool_call_inference_end/cache_read_input_tokens"));
                let cache_creation =
                    num(value.pointer("/tool_call_inference_end/cache_write_input_tokens"));
                if input == 0 && output == 0 && cache_read == 0 && cache_creation == 0 {
                    continue;
                }
                let model = value
                    .get("step_id")
                    .and_then(Value::as_str)
                    .and_then(|step| models.get(step))
                    .cloned()
                    .unwrap_or_default();
                events.push(UsageEvent {
                    model,
                    input_tokens: input,
                    output_tokens: output,
                    cache_read_tokens: cache_read,
                    cache_creation_tokens: cache_creation,
                    occurred_at: json_timestamp(&value),
                    dedup_key: format!("poolside:{index}"),
                });
            }
            _ => {}
        }
    }
    Ok(events)
}

fn read_json_document(path: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read JSON transcript {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("failed to parse JSON transcript {}", path.display()))
}

fn parse_line(line: &str, path: &Path, index: usize) -> Result<Value> {
    serde_json::from_str(line)
        .with_context(|| format!("failed to parse {} line {}", path.display(), index + 1))
}

fn has_any(value: &Value, fields: &[&str]) -> bool {
    fields.iter().any(|field| value.get(*field).is_some())
}

fn string_value(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(|value| {
        value
            .as_str()
            .map(str::to_string)
            .or_else(|| value.as_i64().map(|number| number.to_string()))
            .or_else(|| value.as_f64().map(|number| number.to_string()))
    })
}

fn first_num(value: &Value, fields: &[&str]) -> Option<u64> {
    fields
        .iter()
        .find_map(|field| value.get(*field).and_then(Value::as_u64))
}

fn json_timestamp(value: &Value) -> Option<String> {
    value
        .get("timestamp")
        .or_else(|| value.get("createdAt"))
        .or_else(|| value.get("created_at"))
        .and_then(|timestamp| {
            timestamp
                .as_str()
                .map(str::to_string)
                .or_else(|| timestamp.as_i64().map(|number| number.to_string()))
        })
}

fn num(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_claude_usage_last_write_wins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"summary","summary":"ignored"}
{"type":"assistant","timestamp":"2026-08-30T10:00:00Z","message":{"id":"msg_1","model":"claude-sonnet-4-5-20250929","usage":{"input_tokens":4,"cache_creation_input_tokens":232,"cache_read_input_tokens":10000,"output_tokens":10}}}
{"type":"assistant","timestamp":"2026-08-30T10:00:05Z","message":{"id":"msg_1","model":"claude-sonnet-4-5-20250929","usage":{"input_tokens":4,"cache_creation_input_tokens":232,"cache_read_input_tokens":10000,"output_tokens":91}}}
{"type":"assistant","timestamp":"2026-08-30T10:00:10Z","message":{"id":"msg_2","model":"claude-sonnet-4-5-20250929","usage":{"input_tokens":9,"cache_creation_input_tokens":0,"cache_read_input_tokens":11000,"output_tokens":50}}}
"#,
        )
        .unwrap();

        let events = extract("claude", &path).unwrap();

        assert_eq!(events.len(), 2, "streaming re-emit of msg_1 collapses");
        assert_eq!(events[0].dedup_key, "claude:msg_1");
        assert_eq!(events[0].output_tokens, 91, "last write wins");
        assert_eq!(events[0].input_tokens, 4);
        assert_eq!(events[0].cache_read_tokens, 10000);
        assert_eq!(events[0].model, "claude-sonnet-4-5-20250929");
        assert_eq!(events[1].dedup_key, "claude:msg_2");
    }

    #[test]
    fn extracts_codex_usage_with_model_context() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(
            &path,
            r#"{"timestamp":"2026-08-30T10:00:00Z","type":"session_meta","payload":{"id":"abc"}}
{"timestamp":"2026-08-30T10:00:01Z","type":"turn_context","payload":{"model":"gpt-5-codex"}}
{"timestamp":"2026-08-30T10:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":1200,"cached_input_tokens":1000,"output_tokens":300}}}}
{"timestamp":"2026-08-30T10:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":1200,"cached_input_tokens":1000,"output_tokens":300}}}}
{"timestamp":"2026-08-30T10:00:04Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":1500,"cached_input_tokens":1000,"output_tokens":120}}}}
"#,
        )
        .unwrap();

        let events = extract("codex", &path).unwrap();

        assert_eq!(events.len(), 2, "identical usage payload dedups");
        assert_eq!(events[0].model, "gpt-5-codex");
        assert_eq!(
            events[0].input_tokens, 200,
            "cached portion moved out of input"
        );
        assert_eq!(events[0].cache_read_tokens, 1000);
        assert_eq!(events[0].output_tokens, 300);
        assert_eq!(events[1].input_tokens, 500);
    }

    #[test]
    fn unknown_provider_yields_no_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.jsonl");
        std::fs::write(&path, "{}").unwrap();
        assert!(extract("gemini", &path).unwrap().is_empty());
    }

    #[test]
    fn extracts_amp_usage_and_normalizes_openai_cache_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("thread.json");
        std::fs::write(
            &path,
            r#"{"messages":[{"usage":{"model":"gpt-5","inputTokens":2,"cacheCreationInputTokens":4,"cacheReadInputTokens":3,"outputTokens":5}}]}"#,
        )
        .unwrap();
        let events = extract("amp", &path).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].input_tokens, 6);
        assert_eq!(events[0].cache_creation_tokens, 0);
    }

    #[test]
    fn extracts_gptme_usage_from_assistant_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("conversation.jsonl");
        std::fs::write(
            &path,
            r#"{"role":"assistant","timestamp":"2026-08-31T00:00:00Z","metadata":{"model":"gpt-5","usage":{"input_tokens":2,"output_tokens":3,"cache_read_tokens":4,"cache_creation_tokens":5}}}
"#,
        )
        .unwrap();
        let events = extract("gptme", &path).unwrap();
        assert_eq!(events[0].model, "gpt-5");
        assert_eq!(events[0].cache_read_tokens, 4);
    }

    #[test]
    fn extracts_vscode_copilot_request_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        std::fs::write(
            &path,
            r#"{"requests":[{"requestId":"r1","timestamp":1775000000000,"modelId":"gpt-5","result":{"metadata":{"promptTokens":12,"outputTokens":4,"resolvedModel":"gpt-5.1"}}}]}"#,
        )
        .unwrap();
        let events = extract("vscode-copilot", &path).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].model, "gpt-5.1");
        assert_eq!(events[0].input_tokens, 12);
    }

    #[test]
    fn extracts_kimi_native_usage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wire.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"usage.record","model":"kimi-k2.5","usage":{"inputOther":10,"output":3,"inputCacheRead":2,"inputCacheCreation":1}}
"#,
        )
        .unwrap();
        let events = extract("kimi", &path).unwrap();
        assert_eq!(events[0].input_tokens, 10);
        assert_eq!(events[0].cache_creation_tokens, 1);
    }
}
