//! Search query enums shared by the CLI, MCP, eval, and remote protocol.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::record::{WorkPart, WorkPartBody, WorkPartKind, WorkTarget};

/// Whether search results address whole records or individual parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterMode {
    #[default]
    Anchors,
    Parts,
}

/// Which field a match applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    #[default]
    Content,
    Title,
    Session,
    Input,
    Output,
    Command,
    All,
}

impl FromStr for Field {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "content" => Ok(Self::Content),
            "title" | "dialogue" | "dialog" => Ok(Self::Title),
            "session" => Ok(Self::Session),
            "input" => Ok(Self::Input),
            "output" => Ok(Self::Output),
            "command" | "cmd" => Ok(Self::Command),
            "all" => Ok(Self::All),
            _ => Err(format!(
                "unknown search field `{value}`; expected content, title, session, input, output, command, or all"
            )),
        }
    }
}

impl fmt::Display for Field {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Content => "content",
            Self::Title => "title",
            Self::Session => "session",
            Self::Input => "input",
            Self::Output => "output",
            Self::Command => "command",
            Self::All => "all",
        })
    }
}

/// How search results are ordered.
// kebab-case serde keeps the historical wire spelling (`duration-asc`, `exit-code`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Sort {
    #[default]
    Newest,
    Oldest,
    Duration,
    DurationAsc,
    ExitCode,
    ExitCodeAsc,
    /// BM25 relevance to the rank query.
    Relevance,
}

impl FromStr for Sort {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "newest" | "latest" | "time" | "time-desc" => Ok(Self::Newest),
            "oldest" | "time-asc" => Ok(Self::Oldest),
            "duration" | "duration-desc" | "longest" => Ok(Self::Duration),
            "duration-asc" | "shortest" => Ok(Self::DurationAsc),
            "exit-code" | "exit_code" | "exit" | "exit-desc" => Ok(Self::ExitCode),
            "exit-code-asc" | "exit_code_asc" | "exit-asc" => Ok(Self::ExitCodeAsc),
            "relevance" | "re" => Ok(Self::Relevance),
            _ => Err(format!(
                "unknown search sort `{value}`; expected newest, oldest, duration, duration-asc, exit-code, exit-code-asc, or relevance"
            )),
        }
    }
}

impl fmt::Display for Sort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Newest => "newest",
            Self::Oldest => "oldest",
            Self::Duration => "duration",
            Self::DurationAsc => "duration-asc",
            Self::ExitCode => "exit-code",
            Self::ExitCodeAsc => "exit-code-asc",
            Self::Relevance => "relevance",
        })
    }
}

/// Part filter for `--kind`. Roles and action targets are predicates over the
/// two stored part bodies, not additional storage variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartKind {
    Message,
    Action,
    User,
    Assistant,
    Reasoning,
    System,
    Shell,
    Tool,
}

impl PartKind {
    pub fn matches(self, part: &WorkPart) -> bool {
        use crate::record::MessageRole;
        match self {
            Self::Message => part.kind() == WorkPartKind::Message,
            Self::Action => part.kind() == WorkPartKind::Action,
            Self::User => part.message_role() == Some(MessageRole::User),
            Self::Assistant => part.message_role() == Some(MessageRole::Assistant),
            Self::Reasoning => part.message_role() == Some(MessageRole::Reasoning),
            Self::System => part.message_role() == Some(MessageRole::System),
            Self::Shell => matches!(
                &part.body,
                WorkPartBody::Action {
                    target: WorkTarget::Shell,
                    ..
                }
            ),
            Self::Tool => matches!(
                &part.body,
                WorkPartBody::Action {
                    target: WorkTarget::Tool { .. }
                        | WorkTarget::Mcp { .. }
                        | WorkTarget::Agent { .. },
                    ..
                }
            ),
        }
    }
}

impl FromStr for PartKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().replace('-', "_").as_str() {
            "message" => Ok(Self::Message),
            "action" => Ok(Self::Action),
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            "reasoning" => Ok(Self::Reasoning),
            "system" => Ok(Self::System),
            "shell" => Ok(Self::Shell),
            "tool" => Ok(Self::Tool),
            _ => Err(format!(
                "unknown part kind `{value}`; expected message, action, user, assistant, reasoning, system, shell, or tool"
            )),
        }
    }
}

impl fmt::Display for PartKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Message => "message",
            Self::Action => "action",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Reasoning => "reasoning",
            Self::System => "system",
            Self::Shell => "shell",
            Self::Tool => "tool",
        })
    }
}
