use super::refs::{WorkAt, WorkRef};
use crate::agents::{
    format_structured_block, select_blocks, AgentBlock, AgentBlockKind, AgentProvider,
    AgentSelection, AgentSession,
};
use crate::session::SessionEntry;
use crate::time::{derive_ended_at, derive_started_at, duration_between_ms, normalize_timestamp};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordTextMode {
    Combined,
    Input,
    Output,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordText {
    pub plain: String,
    pub ansi: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkRecordCopyParts {
    pub input: RecordText,
    pub output: RecordText,
    pub block: RecordText,
    pub command: RecordText,
}

impl Default for RecordText {
    fn default() -> Self {
        Self::plain(String::new())
    }
}

impl RecordText {
    pub fn plain(plain: String) -> Self {
        Self { plain, ansi: None }
    }

    pub fn with_ansi(plain: String, ansi: String) -> Self {
        Self {
            plain,
            ansi: Some(ansi),
        }
    }

    pub fn rendered(&self, ansi: bool) -> &str {
        if ansi {
            self.ansi.as_deref().unwrap_or(&self.plain)
        } else {
            &self.plain
        }
    }
}

impl Default for WorkRecordCopyParts {
    fn default() -> Self {
        Self::from_block(RecordText::default())
    }
}

impl WorkRecordCopyParts {
    pub fn from_block(block: RecordText) -> Self {
        Self {
            input: block.clone(),
            output: block.clone(),
            block,
            command: RecordText::default(),
        }
    }
}

pub const RECORD_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkRecordKind {
    TerminalCommand,
    ChatTurn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkOutcome {
    Success,
    Failure,
    Unknown,
}

impl WorkOutcome {
    /// `--status` filter parse, including the CLI alias spellings.
    pub fn from_status_arg(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "success" | "succeeded" | "ok" | "passed" => Ok(Self::Success),
            "failure" | "failed" | "fail" | "error" => Ok(Self::Failure),
            "unknown" => Ok(Self::Unknown),
            _ => Err(format!(
                "unknown search status `{value}`; expected success, failure, or unknown"
            )),
        }
    }

    pub fn as_status_arg(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Unknown => "unknown",
        }
    }
}

impl std::str::FromStr for WorkOutcome {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::from_status_arg(value)
    }
}

impl std::fmt::Display for WorkOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_status_arg())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkChannel {
    Terminal,
    Chat,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkSource {
    pub channel: WorkChannel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkSessionRef {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl WorkSessionRef {
    pub fn matches_id(&self, value: &str) -> bool {
        self.id == value || self.canonical_id.as_deref() == Some(value)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkTime {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

impl WorkTime {
    pub fn from_components(
        started_at: Option<String>,
        ended_at: Option<String>,
        duration_ms: Option<u64>,
    ) -> Self {
        let started_at = started_at.or_else(|| derive_started_at(ended_at.as_deref(), duration_ms));
        let ended_at = ended_at.or_else(|| derive_ended_at(started_at.as_deref(), duration_ms));
        let duration_ms =
            duration_ms.or_else(|| duration_between_ms(started_at.as_deref(), ended_at.as_deref()));

        Self {
            started_at,
            ended_at,
            duration_ms,
        }
    }

    pub fn primary_at(&self) -> Option<&str> {
        self.ended_at.as_deref().or(self.started_at.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkStatus {
    pub outcome: WorkOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkPartKind {
    Prompt,
    Command,
    User,
    Assistant,
    ToolCall,
    ToolResult,
    Skill,
    Thinking,
    Output,
    Error,
}

impl WorkPartKind {
    pub fn is_dialogue(self) -> bool {
        matches!(self, Self::User | Self::Assistant)
    }

    /// Structural evidence stays in records but is excluded from default content search.
    pub fn is_structure(self) -> bool {
        matches!(
            self,
            Self::ToolCall | Self::ToolResult | Self::Skill | Self::Thinking
        )
    }

    pub fn is_input(self) -> bool {
        matches!(
            self,
            Self::Prompt | Self::Command | Self::User | Self::Skill
        )
    }

    pub fn is_output(self) -> bool {
        matches!(
            self,
            Self::Assistant
                | Self::ToolCall
                | Self::ToolResult
                | Self::Thinking
                | Self::Output
                | Self::Error
        )
    }

    /// Map chat-related kinds to agent block kinds for shared formatting.
    pub fn as_agent_block_kind(self) -> Option<crate::agents::AgentBlockKind> {
        use crate::agents::AgentBlockKind;
        Some(match self {
            Self::User => AgentBlockKind::User,
            Self::Assistant => AgentBlockKind::Assistant,
            Self::ToolCall => AgentBlockKind::ToolCall,
            Self::ToolResult => AgentBlockKind::ToolOutput,
            Self::Skill => AgentBlockKind::Skill,
            Self::Thinking => AgentBlockKind::Thinking,
            Self::Prompt | Self::Command | Self::Output | Self::Error => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkPartData {
    Prompt {
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        ansi: Option<String>,
    },
    Command {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        content: String,
    },
    ToolCall {
        #[serde(skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool: Option<String>,
        input: serde_json::Value,
    },
    ToolResult {
        #[serde(skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
        /// Tool name when the provider included it on the result event.
        #[serde(skip_serializing_if = "Option::is_none")]
        tool: Option<String>,
        output: serde_json::Value,
        /// First file line of a read-result body when the provider numbered
        /// it (`775→ …`); display shifts the code gutter to real file lines.
        #[serde(skip_serializing_if = "Option::is_none")]
        start_line: Option<u64>,
    },
    Skill {
        #[serde(skip_serializing_if = "Option::is_none")]
        skill: Option<String>,
        content: String,
    },
    Thinking {
        content: String,
    },
    Output {
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        ansi: Option<String>,
    },
    Error {
        content: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkPart {
    pub seq: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<String>,
    #[serde(flatten)]
    pub data: WorkPartData,
}

fn tool_value(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|_| serde_json::Value::String(text.to_owned()))
}

fn json_text(value: &serde_json::Value) -> std::borrow::Cow<'_, str> {
    match value {
        serde_json::Value::String(text) => std::borrow::Cow::Borrowed(text),
        _ => std::borrow::Cow::Owned(value.to_string()),
    }
}

impl WorkPart {
    pub fn kind(&self) -> WorkPartKind {
        match self.data {
            WorkPartData::Prompt { .. } => WorkPartKind::Prompt,
            WorkPartData::Command { .. } => WorkPartKind::Command,
            WorkPartData::User { .. } => WorkPartKind::User,
            WorkPartData::Assistant { .. } => WorkPartKind::Assistant,
            WorkPartData::ToolCall { .. } => WorkPartKind::ToolCall,
            WorkPartData::ToolResult { .. } => WorkPartKind::ToolResult,
            WorkPartData::Skill { .. } => WorkPartKind::Skill,
            WorkPartData::Thinking { .. } => WorkPartKind::Thinking,
            WorkPartData::Output { .. } => WorkPartKind::Output,
            WorkPartData::Error { .. } => WorkPartKind::Error,
        }
    }

    pub fn text(&self) -> std::borrow::Cow<'_, str> {
        match &self.data {
            WorkPartData::Prompt { content, .. }
            | WorkPartData::Command { content }
            | WorkPartData::User { content }
            | WorkPartData::Assistant { content }
            | WorkPartData::Skill { content, .. }
            | WorkPartData::Thinking { content }
            | WorkPartData::Output { content, .. }
            | WorkPartData::Error { content } => std::borrow::Cow::Borrowed(content),
            WorkPartData::ToolCall { input, .. } => json_text(input),
            WorkPartData::ToolResult { output, .. } => json_text(output),
        }
    }

    pub fn label(&self) -> Option<&str> {
        match &self.data {
            WorkPartData::ToolCall { tool, .. } | WorkPartData::ToolResult { tool, .. } => {
                tool.as_deref()
            }
            WorkPartData::Skill { skill, .. } => skill.as_deref(),
            _ => None,
        }
    }

    pub fn ansi(&self) -> Option<&str> {
        match &self.data {
            WorkPartData::Prompt { ansi, .. } | WorkPartData::Output { ansi, .. } => {
                ansi.as_deref()
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkRecord {
    pub schema_version: u32,
    pub work_ref: WorkRef,
    pub kind: WorkRecordKind,
    pub source: WorkSource,
    pub session: WorkSessionRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub time: WorkTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<WorkStatus>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<WorkPart>,
}

impl WorkRecord {
    pub fn terminal(entry: &SessionEntry, session_path: &Path, index: usize) -> Option<Self> {
        let command = entry.command.trim().to_string();
        let output = entry.output.trim().to_string();
        if command.is_empty() && output.is_empty() {
            return None;
        }

        let session_id = session_path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("current")
            .to_string();
        let work_ref = WorkRef::terminal(session_id.clone(), index + 1);
        let title = if command.is_empty() {
            preview(&output)
        } else {
            preview(&command)
        };
        let outcome = match entry.exit_code {
            Some(0) => WorkOutcome::Success,
            Some(_) => WorkOutcome::Failure,
            None => WorkOutcome::Unknown,
        };

        Some(Self {
            schema_version: RECORD_SCHEMA_VERSION,
            work_ref,
            kind: WorkRecordKind::TerminalCommand,
            source: WorkSource {
                channel: WorkChannel::Terminal,
                provider: None,
            },
            session: WorkSessionRef {
                id: session_id.clone(),
                canonical_id: Some(session_id.clone()),
                path: Some(session_path.display().to_string()),
            },
            cwd: non_empty(entry.cwd.clone().unwrap_or_default()),
            time: WorkTime::from_components(None, entry.ended_at.clone(), entry.duration_ms),
            status: Some(WorkStatus {
                outcome,
                exit_code: entry.exit_code,
            }),
            title,
            parts: terminal_parts(entry, &command, &output),
        })
    }

    pub fn chat_turns(provider: AgentProvider, session: &AgentSession) -> Vec<Self> {
        chat_turn_ranges(&session.blocks)
            .into_iter()
            .enumerate()
            .filter_map(|(index, (start, end))| {
                Self::chat_turn(provider, session, index, &session.blocks[start..end])
            })
            .collect()
    }

    fn chat_turn(
        provider: AgentProvider,
        session: &AgentSession,
        index: usize,
        blocks: &[AgentBlock],
    ) -> Option<Self> {
        let parts = agent_parts(blocks);
        let user = join_part_text(
            parts
                .iter()
                .filter(|part| matches!(part.kind(), WorkPartKind::User)),
        );
        let assistant = join_part_text(
            parts
                .iter()
                .filter(|part| matches!(part.kind(), WorkPartKind::Assistant)),
        );
        if user.trim().is_empty() && assistant.trim().is_empty() {
            return None;
        }
        if assistant.trim().is_empty() && !last_user_is_followed_only_by_tools(blocks) {
            return None;
        }

        let session_ref_id = agent_session_ref_id(session.id.as_deref(), &session.path);
        let canonical_id = agent_session_canonical_id(session.id.as_deref(), &session.path);
        let work_ref = WorkRef::agent(provider, session_ref_id.clone(), index + 1);
        let title = title_from_parts(&parts);
        let started_at =
            first_timestamp(blocks).and_then(|timestamp| normalize_timestamp(&timestamp));
        let ended_at = last_timestamp(blocks).and_then(|timestamp| normalize_timestamp(&timestamp));

        Some(Self {
            schema_version: RECORD_SCHEMA_VERSION,
            work_ref,
            kind: WorkRecordKind::ChatTurn,
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some(provider.command_name().to_string()),
            },
            session: WorkSessionRef {
                id: session_ref_id,
                canonical_id,
                path: Some(session.path.display().to_string()),
            },
            cwd: non_empty(session.cwd.clone().unwrap_or_default()),
            time: WorkTime::from_components(started_at, ended_at, None),
            status: None,
            title,
            parts,
        })
    }

    pub fn selected_chat_records(
        provider: AgentProvider,
        session: &AgentSession,
        selection: AgentSelection,
    ) -> Vec<Self> {
        match selection {
            AgentSelection::LastTurn => Self::chat_turns(provider, session),
            AgentSelection::LastAssistant => selected_block_records(provider, session, selection),
            AgentSelection::LastUser => selected_block_records(provider, session, selection),
            AgentSelection::LastTool => selected_block_records(provider, session, selection),
            AgentSelection::LastBlocks(_) | AgentSelection::All => {
                selected_group_record(provider, session, selection)
            }
        }
    }

    pub fn copy_text(
        &self,
        mode: RecordTextMode,
        include_prompt: bool,
        prompt_override: Option<&str>,
    ) -> RecordText {
        match self.kind {
            WorkRecordKind::TerminalCommand => terminal_record_text(
                mode,
                TerminalTextContext {
                    parts: &self.parts,
                    include_prompt,
                    prompt_override,
                },
            ),
            WorkRecordKind::ChatTurn => chat_record_text(self, mode),
        }
    }

    pub fn copy_parts(&self, include_prompt: bool) -> WorkRecordCopyParts {
        WorkRecordCopyParts {
            input: self.copy_text(RecordTextMode::Input, include_prompt, None),
            output: self.copy_text(RecordTextMode::Output, include_prompt, None),
            block: self.copy_text(RecordTextMode::Combined, include_prompt, None),
            command: self.copy_text(RecordTextMode::Command, false, None),
        }
    }

    pub fn combined_text(&self) -> String {
        let mut text = String::new();
        self.write_combined_text(&mut text);
        text
    }

    pub fn input_text(&self) -> Option<String> {
        match self.kind {
            WorkRecordKind::TerminalCommand => non_empty(join_part_text(
                self.parts
                    .iter()
                    .filter(|part| matches!(part.kind(), WorkPartKind::Command)),
            )),
            WorkRecordKind::ChatTurn => non_empty(join_parts_structured(
                self.parts.iter().filter(|part| part.kind().is_input()),
            )),
        }
    }

    pub fn output_text(&self) -> Option<String> {
        match self.kind {
            WorkRecordKind::TerminalCommand => non_empty(join_part_text(
                self.parts
                    .iter()
                    .filter(|part| matches!(part.kind(), WorkPartKind::Output)),
            )),
            WorkRecordKind::ChatTurn => non_empty(join_parts_structured(
                self.parts.iter().filter(|part| part.kind().is_output()),
            )),
        }
    }

    pub fn content_for_at(&self, at: WorkAt) -> Option<String> {
        match at {
            WorkAt::Whole => Some(self.combined_text()).filter(|text| !text.is_empty()),
            WorkAt::Part(_) => self.part_for_at(at).map(|part| part.text().into_owned()),
        }
    }

    fn write_combined_text(&self, text: &mut String) {
        match self.kind {
            WorkRecordKind::TerminalCommand => {
                for part in self.parts.iter().filter(|part| {
                    matches!(part.kind(), WorkPartKind::Command | WorkPartKind::Output)
                }) {
                    append_text_segment(text, &part.text());
                }
            }
            WorkRecordKind::ChatTurn => {
                for part in &self.parts {
                    if part.kind().is_dialogue() || part.kind().is_structure() {
                        append_text_segment(text, &format_work_part(part));
                    }
                }
            }
        }
    }

    pub fn part_for_at(&self, at: WorkAt) -> Option<&WorkPart> {
        let seq = match at {
            WorkAt::Part(seq) => seq,
            WorkAt::Whole => return None,
        };
        self.parts.iter().find(|part| part.seq == seq)
    }
}

struct TerminalTextContext<'a> {
    parts: &'a [WorkPart],
    include_prompt: bool,
    prompt_override: Option<&'a str>,
}

fn terminal_record_text(mode: RecordTextMode, context: TerminalTextContext<'_>) -> RecordText {
    let prompt_part = first_part(context.parts, WorkPartKind::Prompt);
    let command_part = first_part(context.parts, WorkPartKind::Command);
    let output_part = first_part(context.parts, WorkPartKind::Output);
    let prompt = prompt_part.map(WorkPart::text).unwrap_or_default();
    let command = command_part.map(WorkPart::text).unwrap_or_default();
    let output = output_part.map(WorkPart::text).unwrap_or_default();
    let prompt_ansi = prompt_part.and_then(WorkPart::ansi);
    let output_ansi = output_part.and_then(WorkPart::ansi);

    match mode {
        RecordTextMode::Combined => {
            let input = terminal_input_text(
                &prompt,
                &command,
                prompt_ansi,
                context.include_prompt,
                context.prompt_override,
            );
            let output_ansi = output_ansi.unwrap_or(output.as_ref()).to_string();
            RecordText::with_ansi(
                join_terminal_input_output(&input.plain, &output),
                join_terminal_input_output(input.rendered(true), &output_ansi),
            )
        }
        RecordTextMode::Input => terminal_input_text(
            &prompt,
            &command,
            prompt_ansi,
            context.include_prompt,
            context.prompt_override,
        ),
        RecordTextMode::Output => RecordText::with_ansi(
            output.to_string(),
            output_ansi.unwrap_or(output.as_ref()).to_string(),
        ),
        RecordTextMode::Command => RecordText::plain(command.to_string()),
    }
}

fn first_part(parts: &[WorkPart], kind: WorkPartKind) -> Option<&WorkPart> {
    parts.iter().find(|part| part.kind() == kind)
}

fn join_terminal_input_output(input: &str, output: &str) -> String {
    match (input.is_empty(), output.is_empty()) {
        (false, false) => format!("{input}\n{output}"),
        (false, true) => input.to_string(),
        (true, false) => output.to_string(),
        (true, true) => String::new(),
    }
}

fn terminal_input_text(
    prompt: &str,
    command: &str,
    prompt_ansi: Option<&str>,
    include_prompt: bool,
    prompt_override: Option<&str>,
) -> RecordText {
    if !include_prompt {
        return RecordText::plain(command.to_string());
    }

    let plain_prompt = render_input(prompt, command);
    let ansi_prompt = render_input(prompt_ansi.unwrap_or(prompt), command);
    // The override only rewrites the prompt when there is a command to attach
    // it to; otherwise both branches fall back to the recorded prompt, keeping
    // the ANSI rendering intact.
    let plain = match prompt_override {
        Some(prompt_override) if !command.is_empty() => {
            render_prompt_override(prompt_override, command)
        }
        _ => plain_prompt,
    };
    let ansi = match prompt_override {
        Some(prompt_override) if !command.is_empty() => {
            render_prompt_override(prompt_override, command)
        }
        _ => ansi_prompt,
    };
    RecordText::with_ansi(plain, ansi)
}

fn render_input(prompt: &str, command: &str) -> String {
    match (prompt.trim_end().is_empty(), command.is_empty()) {
        (false, false) => render_prompt_override(prompt, command),
        (false, true) => prompt.trim_end_matches(['\r', '\n']).to_string(),
        (true, false) => command.to_string(),
        (true, true) => String::new(),
    }
}

fn render_prompt_override(prompt: &str, command: &str) -> String {
    let prompt = prompt.trim_end_matches(['\r', '\n']);
    if prompt.is_empty() {
        return command.to_string();
    }

    if prompt.ends_with(' ') || prompt.ends_with('\t') {
        format!("{prompt}{command}")
    } else {
        format!("{prompt} {command}")
    }
}

fn chat_record_text(record: &WorkRecord, mode: RecordTextMode) -> RecordText {
    let text = match mode {
        RecordTextMode::Combined => record.combined_text(),
        RecordTextMode::Input => record.input_text().unwrap_or_default(),
        RecordTextMode::Output => record.output_text().unwrap_or_default(),
        RecordTextMode::Command => String::new(),
    };
    RecordText::plain(text)
}

fn selected_block_records(
    provider: AgentProvider,
    session: &AgentSession,
    selection: AgentSelection,
) -> Vec<WorkRecord> {
    select_blocks(session, selection)
        .into_iter()
        .enumerate()
        .filter(|(_, block)| !block.text.trim().is_empty())
        .map(|(index, block)| selected_block_record(provider, session, index, block))
        .collect()
}

fn selected_block_record(
    provider: AgentProvider,
    session: &AgentSession,
    index: usize,
    block: AgentBlock,
) -> WorkRecord {
    let parts = agent_parts(std::slice::from_ref(&block));
    let title = title_from_parts(&parts);
    let session_ref_id = agent_session_ref_id(session.id.as_deref(), &session.path);
    let canonical_id = agent_session_canonical_id(session.id.as_deref(), &session.path);
    let work_ref = WorkRef::agent(provider, session_ref_id.clone(), index + 1);
    let block_timestamp = block.timestamp.as_deref().and_then(normalize_timestamp);
    WorkRecord {
        schema_version: RECORD_SCHEMA_VERSION,
        work_ref,
        kind: WorkRecordKind::ChatTurn,
        source: WorkSource {
            channel: WorkChannel::Chat,
            provider: Some(provider.command_name().to_string()),
        },
        session: WorkSessionRef {
            id: session_ref_id,
            canonical_id,
            path: Some(session.path.display().to_string()),
        },
        cwd: non_empty(session.cwd.clone().unwrap_or_default()),
        time: WorkTime::from_components(block_timestamp.clone(), None, None),
        status: None,
        title,
        parts,
    }
}

fn selected_group_record(
    provider: AgentProvider,
    session: &AgentSession,
    selection: AgentSelection,
) -> Vec<WorkRecord> {
    let blocks = select_blocks(session, selection);
    let parts = agent_parts(&blocks);
    if parts.is_empty() {
        return Vec::new();
    }

    let session_ref_id = agent_session_ref_id(session.id.as_deref(), &session.path);
    let canonical_id = agent_session_canonical_id(session.id.as_deref(), &session.path);
    let work_ref = WorkRef::agent(provider, session_ref_id.clone(), 1);
    let started_at = first_timestamp(&blocks).and_then(|timestamp| normalize_timestamp(&timestamp));
    let ended_at = last_timestamp(&blocks).and_then(|timestamp| normalize_timestamp(&timestamp));
    vec![WorkRecord {
        schema_version: RECORD_SCHEMA_VERSION,
        work_ref,
        kind: WorkRecordKind::ChatTurn,
        source: WorkSource {
            channel: WorkChannel::Chat,
            provider: Some(provider.command_name().to_string()),
        },
        session: WorkSessionRef {
            id: session_ref_id,
            canonical_id,
            path: Some(session.path.display().to_string()),
        },
        cwd: non_empty(session.cwd.clone().unwrap_or_default()),
        time: WorkTime::from_components(started_at, ended_at, None),
        status: None,
        title: title_from_parts(&parts),
        parts,
    }]
}

pub fn chat_turn_ranges(blocks: &[AgentBlock]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = None;
    let mut has_assistant = false;

    for (idx, block) in blocks.iter().enumerate() {
        if block.kind == AgentBlockKind::User && is_real_user_block(block) {
            if let Some(start) = start {
                if has_assistant {
                    ranges.push((start, idx));
                }
            }
            start = Some(idx);
            has_assistant = false;
        } else if start.is_some() && block.kind == AgentBlockKind::Assistant {
            has_assistant = true;
        }
    }

    if let Some(start) = start {
        if has_assistant {
            ranges.push((start, blocks.len()));
        } else if let Some(previous_user_only_turn) =
            user_only_turn_before_trailing_tools(blocks, start)
        {
            ranges.push(previous_user_only_turn);
        }
    }

    ranges
}

fn user_only_turn_before_trailing_tools(
    blocks: &[AgentBlock],
    start: usize,
) -> Option<(usize, usize)> {
    blocks
        .get(start + 1..)?
        .iter()
        .all(|block| block.kind.is_structure())
        .then_some((start, blocks.len()))
}

pub fn is_real_user_block(block: &AgentBlock) -> bool {
    if block.kind != AgentBlockKind::User {
        return false;
    }

    let text = block.text.trim_start();
    !is_agent_startup_user_text(text)
}

fn is_agent_startup_user_text(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with("# AGENTS.md instructions for")
        || text.starts_with("<environment_context>")
        || text.starts_with("<turn_aborted>")
        || text.starts_with("<local-command-caveat>")
        || text.starts_with("<local-command-stdout>")
        || text.starts_with("<command-message>")
        || text.starts_with("<command-name>")
        || text.starts_with("<command-args>")
        || text.starts_with("<ide_opened_file>")
        || text.starts_with("<ide_selection>")
        || text.starts_with("<system-reminder>")
        || text.starts_with("<system_reminder>")
        // Covers plain interrupt and "…for tool use" variants.
        || text.starts_with("[Request interrupted by user")
}

fn agent_session_ref_id(id: Option<&str>, path: &Path) -> String {
    id.map(short_id)
        .filter(|id| !id.is_empty())
        .or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .map(short_id)
                .filter(|id| !id.is_empty())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn agent_session_canonical_id(id: Option<&str>, path: &Path) -> Option<String> {
    id.filter(|id| !id.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_string)
                .filter(|id| !id.trim().is_empty())
        })
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn first_timestamp(blocks: &[AgentBlock]) -> Option<String> {
    blocks.iter().find_map(|block| block.timestamp.clone())
}

fn last_timestamp(blocks: &[AgentBlock]) -> Option<String> {
    blocks
        .iter()
        .rev()
        .find_map(|block| block.timestamp.clone())
}

fn last_user_is_followed_only_by_tools(blocks: &[AgentBlock]) -> bool {
    let Some(user_idx) = blocks
        .iter()
        .rposition(|block| block.kind == AgentBlockKind::User)
    else {
        return false;
    };

    blocks[user_idx + 1..]
        .iter()
        .all(|block| block.kind.is_structure())
}

fn title_from_parts(parts: &[WorkPart]) -> String {
    let user = join_part_text(
        parts
            .iter()
            .filter(|part| matches!(part.kind(), WorkPartKind::User)),
    );
    if !user.trim().is_empty() {
        return preview(&user);
    }
    let assistant = join_part_text(
        parts
            .iter()
            .filter(|part| matches!(part.kind(), WorkPartKind::Assistant)),
    );
    if !assistant.trim().is_empty() {
        return preview(&assistant);
    }
    preview(&join_part_text(parts))
}

fn skill_attribute(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')?;
    Some(tag[start..start + end].to_string())
}

fn preview(text: &str) -> String {
    preview_from_lines(text.lines())
}

fn preview_from_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> String {
    lines
        .into_iter()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("## ") && !line.starts_with("<:"))
        .unwrap_or("<empty>")
        .chars()
        .take(80)
        .collect()
}

fn append_text_segment(output: &mut String, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    if !output.is_empty() {
        output.push_str("\n\n");
    }
    output.push_str(text);
}

fn join_part_text<'a>(parts: impl IntoIterator<Item = &'a WorkPart>) -> String {
    let mut output = String::new();
    for part in parts {
        append_text_segment(&mut output, &part.text());
    }
    output
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn terminal_parts(entry: &SessionEntry, command: &str, output: &str) -> Vec<WorkPart> {
    let mut parts = Vec::new();
    let prompt = entry.prompt.trim_end_matches(['\r', '\n']);
    if !prompt.trim().is_empty() {
        parts.push(WorkPart {
            seq: parts.len() + 1,
            occurred_at: entry.ended_at.clone(),
            data: WorkPartData::Prompt {
                content: prompt.to_string(),
                ansi: entry.prompt_ansi.clone(),
            },
        });
    }
    if !command.is_empty() {
        parts.push(WorkPart {
            seq: parts.len() + 1,
            occurred_at: entry.ended_at.clone(),
            data: WorkPartData::Command {
                content: command.to_string(),
            },
        });
    }
    if !output.is_empty() {
        parts.push(WorkPart {
            seq: parts.len() + 1,
            occurred_at: entry.ended_at.clone(),
            data: WorkPartData::Output {
                content: output.to_string(),
                ansi: entry.output_ansi.clone(),
            },
        });
    }
    parts
}

fn agent_parts(blocks: &[AgentBlock]) -> Vec<WorkPart> {
    let mut parts = Vec::new();
    // Tool name for results that omit it: borrowed from the matching call —
    // same call id when both carry one, else the nearest preceding call
    // (the block model's adjacency pairing rule).
    let mut last_call_name: Option<String> = None;
    let mut last_call_id: Option<Option<String>> = None;
    for block in blocks {
        let text = block.text.trim();
        if text.is_empty() {
            continue;
        }

        match block.kind {
            AgentBlockKind::User | AgentBlockKind::Assistant => {
                let dialogue_kind = if block.kind == AgentBlockKind::User {
                    TextSegmentKind::User
                } else {
                    TextSegmentKind::Assistant
                };
                for segment in split_skill_segments(text, dialogue_kind) {
                    let data = match segment.kind {
                        TextSegmentKind::User => WorkPartData::User {
                            content: segment.text,
                        },
                        TextSegmentKind::Assistant => WorkPartData::Assistant {
                            content: segment.text,
                        },
                        TextSegmentKind::Skill => WorkPartData::Skill {
                            skill: segment.label,
                            content: segment.text,
                        },
                    };
                    push_agent_part(&mut parts, block.timestamp.clone(), data);
                }
            }
            AgentBlockKind::ToolCall => {
                last_call_name = block.label.clone();
                last_call_id = Some(block.call_id.clone());
                push_agent_part(
                    &mut parts,
                    block.timestamp.clone(),
                    WorkPartData::ToolCall {
                        call_id: block.call_id.clone(),
                        tool: block.label.clone(),
                        input: tool_value(text),
                    },
                );
            }
            AgentBlockKind::ToolOutput => {
                let tool = block.label.clone().or_else(|| {
                    if last_call_id.as_ref() == Some(&block.call_id) {
                        last_call_name.clone()
                    } else {
                        None
                    }
                });
                push_agent_part(
                    &mut parts,
                    block.timestamp.clone(),
                    WorkPartData::ToolResult {
                        call_id: block.call_id.clone(),
                        tool,
                        output: tool_value(text),
                        start_line: block.start_line,
                    },
                );
            }
            AgentBlockKind::Skill => push_agent_part(
                &mut parts,
                block.timestamp.clone(),
                WorkPartData::Skill {
                    skill: block.label.clone(),
                    content: text.to_string(),
                },
            ),
            AgentBlockKind::Thinking => push_agent_part(
                &mut parts,
                block.timestamp.clone(),
                WorkPartData::Thinking {
                    content: text.to_string(),
                },
            ),
        }
    }
    parts
}

#[derive(Clone, Copy)]
enum TextSegmentKind {
    User,
    Assistant,
    Skill,
}

struct TextSegment {
    kind: TextSegmentKind,
    label: Option<String>,
    text: String,
}

fn push_agent_part(parts: &mut Vec<WorkPart>, occurred_at: Option<String>, data: WorkPartData) {
    parts.push(WorkPart {
        seq: parts.len() + 1,
        occurred_at,
        data,
    });
}

/// Split dialogue text into plain dialogue + structured skill segments (never drop skills).
fn split_skill_segments(text: &str, dialogue_kind: TextSegmentKind) -> Vec<TextSegment> {
    let mut segments = Vec::new();
    let mut rest = text;
    let mut plain = String::new();
    while let Some(start) = rest.find("<skill ") {
        plain.push_str(&rest[..start]);
        let candidate = &rest[start..];
        let Some(open_end) = candidate.find('>') else {
            plain.push_str(candidate);
            rest = "";
            break;
        };
        let opening = &candidate[..=open_end];
        let Some(name) = skill_attribute(opening, "name") else {
            plain.push_str(&candidate[..=open_end]);
            rest = &candidate[open_end + 1..];
            continue;
        };
        let after_open = &candidate[open_end + 1..];
        let Some(close_start) = after_open.find("</skill>") else {
            plain.push_str(candidate);
            rest = "";
            break;
        };
        let body = after_open[..close_start].trim();
        if !plain.trim().is_empty() {
            segments.push(TextSegment {
                kind: dialogue_kind,
                label: None,
                text: plain.trim().to_string(),
            });
            plain.clear();
        }
        segments.push(TextSegment {
            kind: TextSegmentKind::Skill,
            label: Some(name),
            text: body.to_string(),
        });
        rest = &after_open[close_start + "</skill>".len()..];
    }
    plain.push_str(rest);
    if !plain.trim().is_empty() {
        segments.push(TextSegment {
            kind: dialogue_kind,
            label: None,
            text: plain.trim().to_string(),
        });
    }
    // Empty skill body is still a structure hit (name-only skill use).
    if segments.is_empty() {
        segments.push(TextSegment {
            kind: dialogue_kind,
            label: None,
            text: text.trim().to_string(),
        });
    }
    segments
}

pub fn format_work_part(part: &WorkPart) -> String {
    let text = part.text();
    match part.kind().as_agent_block_kind() {
        Some(agent_kind) => format_structured_block(agent_kind, part.label(), &text),
        None => text.into_owned(),
    }
}

fn join_parts_structured<'a>(parts: impl IntoIterator<Item = &'a WorkPart>) -> String {
    let mut output = String::new();
    for part in parts {
        append_text_segment(&mut output, &format_work_part(part));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::parse_timestamp;
    use std::path::PathBuf;

    #[test]
    fn terminal_record_maps_metadata_and_status() {
        let entry = SessionEntry::new("repo> ", "cargo test", "failed").with_metadata(
            Some("D:\\sivtr".to_string()),
            Some("2026-05-23T12:00:00Z".to_string()),
            Some(42),
            Some(101),
        );

        let record = WorkRecord::terminal(&entry, Path::new("session_123.log"), 0).unwrap();

        assert_eq!(record.kind, WorkRecordKind::TerminalCommand);
        assert_eq!(record.cwd.as_deref(), Some("D:\\sivtr"));
        assert_eq!(
            record.time.ended_at.as_deref().and_then(parse_timestamp),
            parse_timestamp("2026-05-23T12:00:00Z")
        );
        assert_eq!(
            record.time.started_at.as_deref().and_then(parse_timestamp),
            parse_timestamp("2026-05-23T11:59:59.958Z")
        );
        assert_eq!(record.time.duration_ms, Some(42));
        assert_eq!(
            record.status.as_ref().map(|status| status.outcome),
            Some(WorkOutcome::Failure)
        );
        assert_eq!(
            record.status.as_ref().and_then(|status| status.exit_code),
            Some(101)
        );
        assert_eq!(record.input_text().as_deref(), Some("cargo test"));
        assert_eq!(record.output_text().as_deref(), Some("failed"));
    }

    #[test]
    fn chat_turn_records_keep_interrupted_user_tool_turn() {
        let session = AgentSession {
            path: PathBuf::from("pi-session.jsonl"),
            id: Some("abcdef123456".to_string()),
            cwd: Some("D:\\sivtr".to_string()),
            title: None,
            blocks: vec![
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: Some("2026-05-23T12:01:00Z".to_string()),
                    label: None,
                    call_id: None,
                    text: "fix latest terminal error".to_string(),
                    start_line: None,
                },
                AgentBlock {
                    kind: AgentBlockKind::ToolCall,
                    timestamp: Some("2026-05-23T12:02:00Z".to_string()),
                    label: Some("bash".to_string()),
                    call_id: None,
                    text: "{\"command\":\"cargo test\"}".to_string(),
                    start_line: None,
                },
                AgentBlock {
                    kind: AgentBlockKind::ToolOutput,
                    timestamp: Some("2026-05-23T12:03:00Z".to_string()),
                    label: Some("bash".to_string()),
                    call_id: None,
                    text: "failed".to_string(),
                    start_line: None,
                },
            ],
        };

        let records = WorkRecord::chat_turns(AgentProvider::Pi, &session);

        assert_eq!(records.len(), 1);
        let input = records[0].input_text().unwrap_or_default();
        assert!(input.contains("fix latest terminal error"));
        let output = records[0].output_text().unwrap_or_default();
        assert!(output.contains("<:tool:bash call:>"));
        assert!(output.contains("cargo test"));
        assert!(output.contains("<:tool:bash result:>"));
        assert!(output.contains("failed"));
        assert_eq!(
            records[0]
                .time
                .started_at
                .as_deref()
                .and_then(parse_timestamp),
            parse_timestamp("2026-05-23T12:01:00Z")
        );
        assert_eq!(
            records[0]
                .time
                .ended_at
                .as_deref()
                .and_then(parse_timestamp),
            parse_timestamp("2026-05-23T12:03:00Z")
        );
        assert_eq!(records[0].time.duration_ms, Some(120_000));
    }

    #[test]
    fn tool_results_without_a_label_borrow_the_call_tool_name() {
        let session = AgentSession {
            path: PathBuf::from("pi-session.jsonl"),
            id: Some("abcdef123456".to_string()),
            cwd: Some("D:\\sivtr".to_string()),
            title: None,
            blocks: vec![
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: None,
                    label: None,
                    call_id: None,
                    text: "read a.rs".to_string(),
                    start_line: None,
                },
                AgentBlock {
                    kind: AgentBlockKind::ToolCall,
                    timestamp: None,
                    label: Some("read".to_string()),
                    call_id: Some("c1".to_string()),
                    text: "{\"file_path\":\"a.rs\"}".to_string(),
                    start_line: None,
                },
                AgentBlock {
                    kind: AgentBlockKind::ToolOutput,
                    timestamp: None,
                    label: None,
                    call_id: Some("c1".to_string()),
                    text: "content".to_string(),
                    start_line: None,
                },
            ],
        };

        let records = WorkRecord::chat_turns(AgentProvider::Pi, &session);
        let output = records[0].output_text().unwrap_or_default();
        // The result carries the call's tool name instead of "unknown".
        assert!(output.contains("<:tool:read result:>"));
    }

    #[test]
    fn chat_turn_records_ignore_startup_user_blocks() {
        let session = AgentSession {
            path: PathBuf::from("pi-session.jsonl"),
            id: Some("abcdef123456".to_string()),
            cwd: Some("D:\\sivtr".to_string()),
            title: None,
            blocks: vec![
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: Some("2026-05-23T12:00:00Z".to_string()),
                    label: None,
                    call_id: None,
                    text: "<environment_context>".to_string(),
                    start_line: None,
                },
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: Some("2026-05-23T12:01:00Z".to_string()),
                    label: None,
                    call_id: None,
                    text: "implement this".to_string(),
                    start_line: None,
                },
                AgentBlock {
                    kind: AgentBlockKind::Assistant,
                    timestamp: Some("2026-05-23T12:02:00Z".to_string()),
                    label: None,
                    call_id: None,
                    text: "done".to_string(),
                    start_line: None,
                },
            ],
        };

        let records = WorkRecord::chat_turns(AgentProvider::Pi, &session);

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, WorkRecordKind::ChatTurn);
        assert_eq!(records[0].work_ref.provider(), Some(AgentProvider::Pi));
        assert_eq!(records[0].input_text().as_deref(), Some("implement this"));
        assert_eq!(records[0].output_text().as_deref(), Some("done"));
        assert_eq!(
            records[0]
                .time
                .started_at
                .as_deref()
                .and_then(parse_timestamp),
            parse_timestamp("2026-05-23T12:01:00Z")
        );
        assert_eq!(
            records[0]
                .time
                .ended_at
                .as_deref()
                .and_then(parse_timestamp),
            parse_timestamp("2026-05-23T12:02:00Z")
        );
        assert_eq!(records[0].time.duration_ms, Some(60_000));
    }

    #[test]
    fn chat_turn_records_compact_skill_blocks() {
        let session = AgentSession {
            path: PathBuf::from("pi-session.jsonl"),
            id: Some("abcdef123456".to_string()),
            cwd: Some("D:\\sivtr".to_string()),
            title: None,
            blocks: vec![
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: Some("2026-05-23T12:01:00Z".to_string()),
                    label: None,
                    call_id: None,
                    text: "<skill name=\"sivtr-memory\" location=\"C:\\x\\SKILL.md\">\nlong instructions\n</skill>\n\nreal task".to_string(),
                    start_line: None,
                },
                AgentBlock {
                    kind: AgentBlockKind::Assistant,
                    timestamp: Some("2026-05-23T12:02:00Z".to_string()),
                    label: None,
                    call_id: None,
                    text: "done".to_string(),
                    start_line: None,
                },
            ],
        };

        let records = WorkRecord::chat_turns(AgentProvider::Pi, &session);

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].title, "real task");
        assert!(
            records[0]
                .parts
                .iter()
                .any(|part| part.kind() == WorkPartKind::Skill
                    && part.label() == Some("sivtr-memory"))
        );
        assert!(records[0]
            .parts
            .iter()
            .any(|part| part.kind() == WorkPartKind::User && part.text().contains("real task")));
        let input = records[0].input_text().unwrap_or_default();
        assert!(input.contains("<:skill:sivtr-memory:>"));
        assert!(input.contains("long instructions"));
        assert!(input.contains("real task"));
        assert_eq!(records[0].output_text().as_deref(), Some("done"));
    }

    #[test]
    fn chat_turn_records_ignore_interrupted_tool_use_noise() {
        let session = AgentSession {
            path: PathBuf::from("claude-session.jsonl"),
            id: Some("abcdef123456".to_string()),
            cwd: Some("D:\\sivtr".to_string()),
            title: None,
            blocks: vec![
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: Some("2026-05-23T12:00:00Z".to_string()),
                    label: None,
                    call_id: None,
                    text: "[Request interrupted by user for tool use]".to_string(),
                    start_line: None,
                },
                AgentBlock {
                    kind: AgentBlockKind::Assistant,
                    timestamp: Some("2026-05-23T12:00:01Z".to_string()),
                    label: None,
                    call_id: None,
                    text: "partial".to_string(),
                    start_line: None,
                },
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: Some("2026-05-23T12:01:00Z".to_string()),
                    label: None,
                    call_id: None,
                    text: "continue".to_string(),
                    start_line: None,
                },
                AgentBlock {
                    kind: AgentBlockKind::Assistant,
                    timestamp: Some("2026-05-23T12:02:00Z".to_string()),
                    label: None,
                    call_id: None,
                    text: "done".to_string(),
                    start_line: None,
                },
            ],
        };

        let records = WorkRecord::chat_turns(AgentProvider::Claude, &session);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].input_text().as_deref(), Some("continue"));
        assert_eq!(records[0].output_text().as_deref(), Some("done"));
        // Orphan assistant after filtered interrupt is not kept as its own turn.
        assert!(!records
            .iter()
            .any(|record| record.output_text().as_deref() == Some("partial")));
    }

    #[test]
    fn chat_turn_records_ignore_system_and_command_noise() {
        let session = AgentSession {
            path: PathBuf::from("claude-session.jsonl"),
            id: Some("abcdef123456".to_string()),
            cwd: Some("D:\\sivtr".to_string()),
            title: None,
            blocks: vec![
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: Some("2026-05-23T12:00:00Z".to_string()),
                    label: None,
                    call_id: None,
                    text: "  <system-reminder>\nhidden\n</system-reminder>".to_string(),
                    start_line: None,
                },
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: Some("2026-05-23T12:00:01Z".to_string()),
                    label: None,
                    call_id: None,
                    text: "<command-args>--foo</command-args>".to_string(),
                    start_line: None,
                },
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: Some("2026-05-23T12:00:02Z".to_string()),
                    label: None,
                    call_id: None,
                    text: "<ide_selection>main.rs</ide_selection>".to_string(),
                    start_line: None,
                },
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: Some("2026-05-23T12:01:00Z".to_string()),
                    label: None,
                    call_id: None,
                    text: "real question".to_string(),
                    start_line: None,
                },
                AgentBlock {
                    kind: AgentBlockKind::Assistant,
                    timestamp: Some("2026-05-23T12:02:00Z".to_string()),
                    label: None,
                    call_id: None,
                    text: "answer".to_string(),
                    start_line: None,
                },
            ],
        };

        let records = WorkRecord::chat_turns(AgentProvider::Claude, &session);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].input_text().as_deref(), Some("real question"));
        assert_eq!(records[0].parts[1].text(), "answer");
    }
}
