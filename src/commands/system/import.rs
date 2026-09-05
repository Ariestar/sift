//! Import durable Claude.ai and ChatGPT conversation exports into archive.db.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sivtr_core::agents::{
    extract_content_text, push_block, AgentBlockKind, AgentProvider, AgentSession,
};
use sivtr_core::archive::store::{self, SessionUpsert};
use sivtr_core::cache;
use sivtr_core::record::WorkRecord;
use zip::ZipArchive;

use crate::cli::{ImportProvider, ImportSessionsArgs};

pub fn execute(args: &ImportSessionsArgs) -> Result<()> {
    let provider = match args.provider {
        ImportProvider::ClaudeAi => AgentProvider::ClaudeAi,
        ImportProvider::ChatGpt => AgentProvider::ChatGpt,
    };
    let text = read_export(&args.path)?;
    let sessions = parse_export(provider, &text)?;
    let stamp = cache::file_stamp(&args.path)
        .ok_or_else(|| anyhow::anyhow!("cannot stat import file {}", args.path.display()))?;
    let conn = sivtr_core::archive::open()?;
    let mut imported = 0usize;
    for session in sessions {
        let session_id = session
            .id
            .as_deref()
            .context("imported session has no stable id")?;
        let records = WorkRecord::chat_turns(provider, &session);
        if records.is_empty() {
            continue;
        }
        let source_path = PathBuf::from(format!("{}#{}", args.path.display(), session_id));
        store::upsert_session(
            &conn,
            &SessionUpsert {
                provider: provider.command_name(),
                session_id,
                source_path: &source_path,
                cwd: None,
                workspace_key: "",
                title: session.title.as_deref(),
                stamp,
                records: &records,
                usage_events: &[],
            },
        )?;
        imported += 1;
    }
    println!(
        "imported {imported} {} sessions from {}",
        provider.name(),
        args.path.display()
    );
    Ok(())
}

fn read_export(path: &Path) -> Result<String> {
    if path.extension().and_then(|ext| ext.to_str()) != Some("zip") {
        return std::fs::read_to_string(path)
            .with_context(|| format!("failed to read export {}", path.display()));
    }
    let file =
        File::open(path).with_context(|| format!("failed to open export {}", path.display()))?;
    let mut archive = ZipArchive::new(file).context("invalid export ZIP")?;
    let mut matches = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        if Path::new(entry.name())
            .file_name()
            .and_then(|name| name.to_str())
            == Some("conversations.json")
        {
            matches.push(index);
        }
    }
    let index = match matches.as_slice() {
        [index] => *index,
        [] => bail!("export ZIP does not contain conversations.json"),
        _ => bail!("export ZIP contains multiple conversations.json files"),
    };
    let mut entry = archive.by_index(index)?;
    let mut text = String::new();
    entry
        .read_to_string(&mut text)
        .context("export conversations.json is not UTF-8")?;
    Ok(text)
}

fn parse_export(provider: AgentProvider, text: &str) -> Result<Vec<AgentSession>> {
    let root: Value = serde_json::from_str(text).context("conversation export is not JSON")?;
    let conversations = root
        .as_array()
        .or_else(|| root.get("conversations").and_then(Value::as_array))
        .context("conversation export must contain a conversations array")?;
    match provider {
        AgentProvider::ClaudeAi => parse_claude(conversations),
        AgentProvider::ChatGpt => parse_chatgpt(conversations),
        _ => bail!("unsupported import provider {}", provider.command_name()),
    }
}

fn parse_claude(conversations: &[Value]) -> Result<Vec<AgentSession>> {
    let mut sessions = Vec::new();
    for conversation in conversations {
        let id = conversation
            .get("uuid")
            .or_else(|| conversation.get("id"))
            .and_then(Value::as_str)
            .map(addressable_id)
            .filter(|id| !id.is_empty())
            .context("Claude.ai conversation has no uuid")?;
        let mut session = imported_session(id, conversation.get("name").and_then(Value::as_str));
        if let Some(messages) = conversation.get("chat_messages").and_then(Value::as_array) {
            for message in messages {
                let kind = match message.get("sender").and_then(Value::as_str) {
                    Some("human") | Some("user") => AgentBlockKind::User,
                    Some("assistant") => AgentBlockKind::Assistant,
                    _ => continue,
                };
                let content = message.get("text").or_else(|| message.get("content"));
                if let Some(content) = content {
                    push_block(
                        &mut session,
                        kind,
                        timestamp(message),
                        None,
                        extract_content_text(content),
                    );
                }
            }
        }
        sessions.push(session);
    }
    Ok(sessions)
}

fn parse_chatgpt(conversations: &[Value]) -> Result<Vec<AgentSession>> {
    let mut sessions = Vec::new();
    for conversation in conversations {
        let id = conversation
            .get("conversation_id")
            .or_else(|| conversation.get("id"))
            .and_then(Value::as_str)
            .map(addressable_id)
            .filter(|id| !id.is_empty())
            .context("ChatGPT conversation has no conversation_id")?;
        let mut session = imported_session(id, conversation.get("title").and_then(Value::as_str));
        let mapping = conversation
            .get("mapping")
            .and_then(Value::as_object)
            .context("ChatGPT conversation has no mapping object")?;
        let mut messages = mapping
            .values()
            .filter_map(|node| node.get("message"))
            .filter_map(|message| {
                let role = message.pointer("/author/role").and_then(Value::as_str)?;
                let kind = match role {
                    "user" => AgentBlockKind::User,
                    "assistant" => AgentBlockKind::Assistant,
                    "tool" => AgentBlockKind::ToolOutput,
                    _ => return None,
                };
                let content = message.get("content")?;
                let text = content
                    .get("parts")
                    .map(extract_content_text)
                    .unwrap_or_else(|| extract_content_text(content));
                Some((timestamp(message), kind, text))
            })
            .collect::<Vec<_>>();
        messages.sort_by(|left, right| left.0.cmp(&right.0));
        for (timestamp, kind, text) in messages {
            push_block(&mut session, kind, timestamp, None, text);
        }
        sessions.push(session);
    }
    Ok(sessions)
}

fn imported_session(id: String, title: Option<&str>) -> AgentSession {
    AgentSession {
        path: PathBuf::from(format!("imported#{id}")),
        id: Some(id),
        cwd: None,
        title: title.map(str::to_string),
        blocks: Vec::new(),
    }
}

fn timestamp(value: &Value) -> Option<String> {
    let value = value
        .get("created_at")
        .or_else(|| value.get("create_time"))
        .or_else(|| value.get("timestamp"))?;
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    let number = value.as_f64()?;
    let millis = if number.abs() >= 1_000_000_000_000.0 {
        number as i64
    } else {
        (number * 1_000.0) as i64
    };
    DateTime::<Utc>::from_timestamp_millis(millis).map(|time| time.to_rfc3339())
}

fn addressable_id(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chatgpt_mapping_in_timestamp_order() {
        let sessions = parse_export(
            AgentProvider::ChatGpt,
            r#"[{"conversation_id":"c1","title":"task","mapping":{
                "b":{"message":{"author":{"role":"assistant"},"content":{"parts":["answer"]},"create_time":2}},
                "a":{"message":{"author":{"role":"user"},"content":{"parts":["question"]},"create_time":1}}
            }}]"#,
        )
        .unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].blocks[0].text, "question");
        assert_eq!(sessions[0].blocks[1].text, "answer");
    }

    #[test]
    fn parses_claude_chat_messages() {
        let sessions = parse_export(
            AgentProvider::ClaudeAi,
            r#"[{"uuid":"c1","name":"task","chat_messages":[
                {"sender":"human","text":"question"},
                {"sender":"assistant","text":"answer"}
            ]}]"#,
        )
        .unwrap();
        assert_eq!(sessions[0].blocks.len(), 2);
        assert_eq!(sessions[0].title.as_deref(), Some("task"));
    }
}
