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

pub const RECORD_SCHEMA_VERSION: u32 = 6;

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
    Message,
    Action,
}

/// One view of a record's parts. Copy, search, show, export, publish, the
/// TUI, and the web API all consume a projection — none of them branch on
/// record kind or channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    /// What the user said and did: user messages, human actions.
    Input,
    /// What came back: assistant, reasoning, and system messages, agent
    /// actions, and the output of human actions.
    Output,
    /// Shell commands, whoever ran them.
    Commands,
    /// Every part in transcript order.
    Combined,
}

/// Which slice of a part one projection renders. An action carries its
/// whole lifecycle, so the input projection shows its command and the
/// output projection shows its result; messages always render whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionSlice {
    Whole,
    Input,
    Output,
}

impl Projection {
    /// The slice a part contributes to this projection. Parts outside the
    /// projection never reach a consumer, so only included parts are asked.
    pub fn slice_of(self, part: &WorkPart) -> ProjectionSlice {
        match (&part.body, self) {
            (_, Projection::Combined | Projection::Commands) => ProjectionSlice::Whole,
            (WorkPartBody::Message { .. }, _) => ProjectionSlice::Whole,
            (WorkPartBody::Action { .. }, Projection::Input) => ProjectionSlice::Input,
            (WorkPartBody::Action { actor, .. }, Projection::Output) => match actor {
                // Agent actions read as one flow (call then result); a human
                // action's command already went to the input view.
                WorkActor::Agent => ProjectionSlice::Whole,
                WorkActor::User => ProjectionSlice::Output,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Reasoning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkActor {
    User,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkTarget {
    Shell,
    Tool { name: Option<String> },
    Mcp { server: String, tool: String },
    Agent { name: String },
}

impl WorkTarget {
    pub fn label(&self) -> Option<&str> {
        match self {
            Self::Shell => None,
            Self::Tool { name } => name.as_deref(),
            Self::Agent { name } => Some(name),
            Self::Mcp { tool, .. } => Some(tool),
        }
    }

    pub fn is_shell(&self) -> bool {
        matches!(self, Self::Shell)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkActionStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkContent {
    Text {
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        ansi: Option<String>,
    },
    Json(serde_json::Value),
}

impl WorkContent {
    /// Renderable text of the content: text as-is, JSON compacted.
    pub fn text(&self) -> std::borrow::Cow<'_, str> {
        match self {
            Self::Text { content, .. } => std::borrow::Cow::Borrowed(content),
            Self::Json(value) => json_text(value),
        }
    }

    fn ansi(&self) -> Option<&str> {
        match self {
            Self::Text { ansi, .. } => ansi.as_deref(),
            Self::Json(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkContentBlock {
    pub content: WorkContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkPartBody {
    Message {
        role: MessageRole,
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        content: WorkContent,
    },
    Action {
        id: String,
        actor: WorkActor,
        target: WorkTarget,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        input: Option<WorkContent>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        output: Vec<WorkContentBlock>,
        status: WorkActionStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkPart {
    pub seq: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<String>,
    #[serde(flatten)]
    pub body: WorkPartBody,
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
        match self.body {
            WorkPartBody::Message { .. } => WorkPartKind::Message,
            WorkPartBody::Action { .. } => WorkPartKind::Action,
        }
    }

    pub fn text(&self) -> std::borrow::Cow<'_, str> {
        match &self.body {
            WorkPartBody::Message { content, .. } => content.text(),
            WorkPartBody::Action { input, output, .. } => {
                let text = output_blocks_text(output);
                (!text.is_empty())
                    .then_some(std::borrow::Cow::Owned(text))
                    .or_else(|| input.as_ref().map(WorkContent::text))
                    .unwrap_or_default()
            }
        }
    }

    pub fn label(&self) -> Option<&str> {
        match &self.body {
            WorkPartBody::Message { label, .. } => label.as_deref(),
            WorkPartBody::Action { target, .. } => target.label(),
        }
    }

    pub fn ansi(&self) -> Option<&str> {
        match &self.body {
            WorkPartBody::Message { content, .. } => content.ansi(),
            WorkPartBody::Action { output, .. } => {
                output.iter().find_map(|block| block.content.ansi())
            }
        }
    }

    pub fn message_role(&self) -> Option<MessageRole> {
        match self.body {
            WorkPartBody::Message { role, .. } => Some(role),
            WorkPartBody::Action { .. } => None,
        }
    }

    pub fn is_dialogue(&self) -> bool {
        matches!(
            self.message_role(),
            Some(MessageRole::User | MessageRole::Assistant)
        )
    }

    /// Agent-side evidence: agent actions and reasoning/system messages.
    /// Evidence folds into tags and stays out of dialogue; human content
    /// (user messages, human actions) reads as body text.
    pub fn is_structure(&self) -> bool {
        match &self.body {
            WorkPartBody::Message { role, .. } => {
                matches!(role, MessageRole::System | MessageRole::Reasoning)
            }
            WorkPartBody::Action { actor, .. } => matches!(actor, WorkActor::Agent),
        }
    }

    pub fn is_input(&self) -> bool {
        match &self.body {
            WorkPartBody::Message { role, .. } => matches!(role, MessageRole::User),
            WorkPartBody::Action { actor, .. } => matches!(actor, WorkActor::User),
        }
    }

    pub fn is_output(&self) -> bool {
        match &self.body {
            WorkPartBody::Message { role, .. } => {
                matches!(role, MessageRole::Assistant | MessageRole::Reasoning)
            }
            WorkPartBody::Action { .. } => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkRecord {
    pub schema_version: u32,
    pub work_ref: WorkRef,
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
    /// Stream discriminator derived from the ref: `None` = terminal, else
    /// the agent provider. The single source of truth — everything else
    /// (namespace, labels, cache keys) renders off this.
    pub fn provider(&self) -> Option<AgentProvider> {
        self.work_ref.provider()
    }

    pub fn is_terminal(&self) -> bool {
        self.provider().is_none()
    }

    pub fn is_agent(&self) -> bool {
        self.provider().is_some()
    }

    /// `"shell"` / `"ai"` — the kind label, derived from the ref.
    pub fn kind_label(&self) -> &'static str {
        if self.is_terminal() {
            "shell"
        } else {
            "ai"
        }
    }

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
                .filter(|part| part.message_role() == Some(MessageRole::User)),
        );
        let assistant = join_part_text(
            parts
                .iter()
                .filter(|part| part.message_role() == Some(MessageRole::Assistant)),
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
        let input = self.input_text().unwrap_or_default();
        let output = self.output_text().unwrap_or_default();
        match mode {
            RecordTextMode::Input => {
                let input = if include_prompt {
                    let prompt = prompt_override
                        .map(str::to_string)
                        .or_else(|| self.terminal_prompt());
                    prompt
                        .map(|prompt| render_prompt_override(&prompt, &input))
                        .unwrap_or(input)
                } else {
                    input
                };
                RecordText::plain(input)
            }
            RecordTextMode::Output => RecordText::plain(output),
            RecordTextMode::Command => self
                .parts
                .iter()
                .filter_map(action_input)
                .find(|(target, _)| target.is_shell())
                .map(|(_, input)| {
                    let plain = input.text().into_owned();
                    let ansi = input
                        .ansi()
                        .map(str::to_owned)
                        .unwrap_or_else(|| plain.clone());
                    RecordText::with_ansi(plain, ansi)
                })
                .unwrap_or_default(),
            RecordTextMode::Combined => {
                let mut plain = String::new();
                for part in &self.parts {
                    append_text_segment(&mut plain, &format_work_part(part));
                }
                if let Some(prompt) = prompt_override.filter(|_| !input.is_empty()) {
                    plain = render_prompt_override(prompt, &input);
                    if !output.is_empty() {
                        plain.push_str("\n\n");
                        plain.push_str(&output);
                    }
                }
                RecordText::plain(plain)
            }
        }
    }

    /// The prompt line of the record's shell command, when it kept one.
    fn terminal_prompt(&self) -> Option<String> {
        self.parts.iter().find_map(|part| match &part.body {
            WorkPartBody::Action {
                target: WorkTarget::Shell,
                title: Some(prompt),
                input: Some(_),
                ..
            } => Some(prompt.clone()),
            _ => None,
        })
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
        non_empty(self.projected_text(Projection::Input))
    }

    pub fn output_text(&self) -> Option<String> {
        non_empty(self.projected_text(Projection::Output))
    }

    /// The copy text of one projection: whole parts render in their display
    /// format, the sliced side of an action renders just that side.
    fn projected_text(&self, projection: Projection) -> String {
        let mut text = String::new();
        for part in self.project(projection) {
            append_text_segment(&mut text, &projected_part_text(part, projection));
        }
        text
    }

    pub fn content_for_at(&self, at: WorkAt) -> Option<String> {
        match at {
            WorkAt::Whole => Some(self.combined_text()).filter(|text| !text.is_empty()),
            WorkAt::Part(_) => self.part_for_at(at).map(|part| part.text().into_owned()),
        }
    }

    fn write_combined_text(&self, text: &mut String) {
        for part in &self.parts {
            append_text_segment(text, &format_work_part(part));
        }
    }

    pub fn part_for_at(&self, at: WorkAt) -> Option<&WorkPart> {
        let seq = match at {
            WorkAt::Part(seq) => seq,
            WorkAt::Whole => return None,
        };
        self.parts.iter().find(|part| part.seq == seq)
    }

    /// The parts one projection includes, in transcript order. An action
    /// included in both views (a human command: input view and output view)
    /// carries its whole lifecycle either way; consumers render the slice
    /// the projection names.
    pub fn project(&self, projection: Projection) -> Vec<&WorkPart> {
        self.parts
            .iter()
            .filter(|part| {
                match (&part.body, projection) {
                    (_, Projection::Combined) => true,
                    (WorkPartBody::Action { target, .. }, Projection::Commands) => {
                        target.is_shell()
                    }
                    (WorkPartBody::Message { .. }, Projection::Commands) => false,
                    (WorkPartBody::Message { role, .. }, Projection::Input) => {
                        matches!(role, MessageRole::User | MessageRole::System)
                    }
                    (WorkPartBody::Action { actor, .. }, Projection::Input) => {
                        matches!(actor, WorkActor::User)
                    }
                    (WorkPartBody::Message { role, .. }, Projection::Output) => {
                        matches!(role, MessageRole::Assistant | MessageRole::Reasoning)
                    }
                    (WorkPartBody::Action { actor, output, .. }, Projection::Output) => match actor
                    {
                        WorkActor::Agent => true,
                        // A human action's result belongs to the output view
                        // once it has one; a command without output is
                        // input-only.
                        WorkActor::User => !output.is_empty(),
                    },
                }
            })
            .collect()
    }
}

fn action_input(part: &WorkPart) -> Option<(&WorkTarget, &WorkContent)> {
    match &part.body {
        WorkPartBody::Action {
            target,
            input: Some(input),
            ..
        } => Some((target, input)),
        _ => None,
    }
}

/// The text one part contributes to a projection: whole parts render in
/// their display format; the sliced side of an action renders just that
/// side — the command as plain text, the result as its blocks' text.
fn projected_part_text(part: &WorkPart, projection: Projection) -> String {
    match projection.slice_of(part) {
        ProjectionSlice::Whole => format_work_part(part),
        ProjectionSlice::Input => match &part.body {
            WorkPartBody::Action {
                input: Some(input), ..
            } => input.text().into_owned(),
            _ => format_work_part(part),
        },
        ProjectionSlice::Output => match &part.body {
            WorkPartBody::Action { output, .. } if !output.is_empty() => output
                .iter()
                .map(|block| block.content.text().into_owned())
                .collect::<Vec<_>>()
                .join("\n\n"),
            _ => format_work_part(part),
        },
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
        if block.kind == AgentBlockKind::User {
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
            .filter(|part| part.message_role() == Some(MessageRole::User)),
    );
    if !user.trim().is_empty() {
        return preview(&user);
    }
    let assistant = join_part_text(
        parts
            .iter()
            .filter(|part| part.message_role() == Some(MessageRole::Assistant)),
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

/// Join text segments with blank lines, skipping empty ones — the shared
/// shape of multi-part record text and multi-block action output.
pub fn output_blocks_text<'a>(blocks: impl IntoIterator<Item = &'a WorkContentBlock>) -> String {
    let mut output = String::new();
    for block in blocks {
        append_text_segment(&mut output, &block.content.text());
    }
    output
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
    // One shell action carries the whole command lifecycle: the prompt line
    // is its context (`title`), the command its input, the terminal output
    // its result. A terminal command and an agent-run shell tool therefore
    // have the same shape.
    let prompt = entry.prompt.trim_end_matches(['\r', '\n']);
    let mut output_blocks = Vec::new();
    if !output.is_empty() {
        output_blocks.push(WorkContentBlock {
            content: WorkContent::Text {
                content: output.to_string(),
                ansi: entry.output_ansi.clone(),
            },
            start_line: None,
        });
    }
    vec![WorkPart {
        seq: 1,
        occurred_at: entry.ended_at.clone(),
        body: WorkPartBody::Action {
            id: "shell-1".to_string(),
            actor: WorkActor::User,
            target: WorkTarget::Shell,
            title: (!prompt.trim().is_empty()).then(|| prompt.to_string()),
            input: (!command.is_empty()).then(|| WorkContent::Text {
                content: command.to_string(),
                ansi: None,
            }),
            output: output_blocks,
            status: match entry.exit_code {
                Some(0) | None => WorkActionStatus::Completed,
                Some(_) => WorkActionStatus::Failed,
            },
            exit_code: entry.exit_code,
        },
    }]
}

/// Build record parts from parsed agent blocks — the single path from raw
/// provider events to typed parts (production parsing and test fixtures).
pub fn agent_parts(blocks: &[AgentBlock]) -> Vec<WorkPart> {
    let mut parts = Vec::new();
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
                    let body = match segment.kind {
                        TextSegmentKind::User => WorkPartBody::Message {
                            role: MessageRole::User,
                            label: None,
                            content: WorkContent::Text {
                                content: segment.text,
                                ansi: None,
                            },
                        },
                        TextSegmentKind::Assistant => WorkPartBody::Message {
                            role: MessageRole::Assistant,
                            label: None,
                            content: WorkContent::Text {
                                content: segment.text,
                                ansi: None,
                            },
                        },
                        TextSegmentKind::Skill => WorkPartBody::Message {
                            role: MessageRole::System,
                            label: segment.label,
                            content: WorkContent::Text {
                                content: segment.text,
                                ansi: None,
                            },
                        },
                    };
                    push_agent_part(&mut parts, block.timestamp.clone(), body);
                }
            }
            AgentBlockKind::ToolCall => {
                let id = block
                    .call_id
                    .clone()
                    .unwrap_or_else(|| format!("action-{}", parts.len() + 1));
                let value = tool_value(text);
                let body = if block.label.as_deref().is_some_and(shell_tool) {
                    // Shell tools normalize to the same shape as a terminal
                    // command: the command text is the input, the tool's
                    // description the context line.
                    WorkPartBody::Action {
                        id,
                        actor: WorkActor::Agent,
                        target: WorkTarget::Shell,
                        title: json_field_str(&value, "description"),
                        input: Some(shell_command_content(&value)),
                        output: Vec::new(),
                        status: WorkActionStatus::InProgress,
                        exit_code: None,
                    }
                } else {
                    WorkPartBody::Action {
                        id,
                        actor: WorkActor::Agent,
                        target: WorkTarget::Tool {
                            name: block.label.clone(),
                        },
                        title: None,
                        input: Some(WorkContent::Json(value)),
                        output: Vec::new(),
                        status: WorkActionStatus::InProgress,
                        exit_code: None,
                    }
                };
                push_agent_part(&mut parts, block.timestamp.clone(), body);
            }
            AgentBlockKind::ToolOutput => {
                let id = block
                    .call_id
                    .clone()
                    .unwrap_or_else(|| format!("action-{}", parts.len() + 1));
                let value = tool_value(text);
                let body = if block.label.as_deref().is_some_and(shell_tool) {
                    let (output, exit_code) = shell_output(&value);
                    WorkPartBody::Action {
                        id,
                        actor: WorkActor::Agent,
                        target: WorkTarget::Shell,
                        title: None,
                        input: None,
                        output,
                        status: match exit_code {
                            Some(0) | None => WorkActionStatus::Completed,
                            Some(_) => WorkActionStatus::Failed,
                        },
                        exit_code,
                    }
                } else {
                    WorkPartBody::Action {
                        id,
                        actor: WorkActor::Agent,
                        target: WorkTarget::Tool {
                            name: block.label.clone(),
                        },
                        title: None,
                        input: None,
                        output: vec![WorkContentBlock {
                            content: WorkContent::Json(value),
                            start_line: block.start_line,
                        }],
                        status: WorkActionStatus::Completed,
                        exit_code: None,
                    }
                };
                push_agent_part(&mut parts, block.timestamp.clone(), body);
            }
            AgentBlockKind::Skill => push_agent_part(
                &mut parts,
                block.timestamp.clone(),
                WorkPartBody::Message {
                    role: MessageRole::System,
                    label: block.label.clone(),
                    content: WorkContent::Text {
                        content: text.to_string(),
                        ansi: None,
                    },
                },
            ),
            AgentBlockKind::Thinking => push_agent_part(
                &mut parts,
                block.timestamp.clone(),
                WorkPartBody::Message {
                    role: MessageRole::Reasoning,
                    label: None,
                    content: WorkContent::Text {
                        content: text.to_string(),
                        ansi: None,
                    },
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

/// Append one parsed agent event. An event carrying a stable id folds into
/// its earlier action part: the input stays, output appends (streaming
/// chunks), and status, exit code, and context update from the event.
/// Events without a matching id stand alone — pairing is never guessed by
/// tool name or adjacency.
fn push_agent_part(parts: &mut Vec<WorkPart>, occurred_at: Option<String>, body: WorkPartBody) {
    let index = action_id(&body).and_then(|id| {
        parts
            .iter()
            .rposition(|part| action_id(&part.body) == Some(id))
    });
    let Some(index) = index else {
        parts.push(WorkPart {
            seq: parts.len() + 1,
            occurred_at,
            body,
        });
        return;
    };
    let WorkPartBody::Action {
        input: event_input,
        output: event_output,
        status: event_status,
        exit_code: event_exit,
        title: event_title,
        ..
    } = body
    else {
        unreachable!("matched an action above");
    };
    let WorkPartBody::Action {
        input,
        output,
        status,
        exit_code,
        title,
        ..
    } = &mut parts[index].body
    else {
        unreachable!("matched an action above");
    };
    if input.is_none() {
        *input = event_input;
    }
    output.extend(event_output);
    *status = event_status;
    if event_exit.is_some() {
        *exit_code = event_exit;
    }
    if title.is_none() {
        *title = event_title;
    }
}

/// Stable pairing id of an action body; messages have none.
fn action_id(body: &WorkPartBody) -> Option<&str> {
    match body {
        WorkPartBody::Action { id, .. } => Some(id),
        WorkPartBody::Message { .. } => None,
    }
}

/// Canonical identity of a shell-execution tool: the names providers give
/// their shell tool. `false` for tools that are not shell execution — those
/// stay `WorkTarget::Tool`.
fn shell_tool(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "exec"
            | "bash"
            | "shell"
            | "run_terminal_command"
            | "shell_command"
            | "run_command"
            | "run_command_or_subagent"
    )
}

/// A shell call's input as command text: the `command` string field, the
/// whole value when the provider already sent plain text.
fn shell_command_content(value: &serde_json::Value) -> WorkContent {
    let command = value
        .as_object()
        .and_then(|object| object.get("command"))
        .and_then(|command| command.as_str())
        .or_else(|| value.as_str())
        .map(str::trim)
        .unwrap_or_default();
    WorkContent::Text {
        content: command.to_string(),
        ansi: None,
    }
}

/// A shell result as output text blocks (`stdout`/`stderr`), plus the exit
/// code when the provider reported one. Values without the fields keep
/// their full shape as one text block, so no payload is lost.
fn shell_output(value: &serde_json::Value) -> (Vec<WorkContentBlock>, Option<i32>) {
    let object = value.as_object();
    let exit_code = object
        .and_then(|object| object.get("exit_code").or_else(|| object.get("exitCode")))
        .and_then(serde_json::Value::as_i64)
        .map(|code| code as i32);
    let text = |key: &str| {
        object
            .and_then(|object| object.get(key))
            .and_then(|value| value.as_str())
            .map(str::to_string)
    };
    let stdout = text("stdout");
    let stderr = text("stderr");
    let mut blocks = Vec::new();
    if let Some(stdout) = stdout {
        blocks.push(text_block(stdout));
    }
    if let Some(stderr) = stderr {
        blocks.push(text_block(stderr));
    }
    if blocks.is_empty() {
        let payload = match value.as_str() {
            Some(text) => text.to_string(),
            None => value.to_string(),
        };
        blocks.push(text_block(payload));
    }
    (blocks, exit_code)
}

fn text_block(content: String) -> WorkContentBlock {
    WorkContentBlock {
        content: WorkContent::Text {
            content,
            ansi: None,
        },
        start_line: None,
    }
}

/// One string field of a JSON tool payload, if present and non-empty.
fn json_field_str(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .as_object()
        .and_then(|object| object.get(key))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .map(str::to_string)
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
    match &part.body {
        WorkPartBody::Message { role, .. } => {
            let kind = match role {
                MessageRole::User => AgentBlockKind::User,
                MessageRole::Assistant => AgentBlockKind::Assistant,
                MessageRole::System => AgentBlockKind::Skill,
                MessageRole::Reasoning => AgentBlockKind::Thinking,
            };
            format_structured_block(kind, part.label(), &text)
        }
        // A shell action reads as the terminal transcript it is — the typed
        // command line (with its prompt context when kept), then the output.
        // Other actions keep their tool-evidence markers.
        WorkPartBody::Action { target, .. } if target.is_shell() => {
            format_shell_action(part, ProjectionSlice::Whole)
        }
        WorkPartBody::Action { output, .. } => {
            let kind = if output.is_empty() {
                AgentBlockKind::ToolCall
            } else {
                AgentBlockKind::ToolOutput
            };
            format_structured_block(kind, part.label(), &text)
        }
    }
}

/// Body of a shell action in the projection's slice: the typed command line
/// (with its prompt context when kept) unless the slice is output-only, then
/// the result blocks — the terminal transcript it is, whichever source it
/// came from. Shared by the TUI content pane and text export.
pub fn format_shell_action(part: &WorkPart, slice: ProjectionSlice) -> String {
    let WorkPartBody::Action {
        target,
        title,
        input,
        output,
        ..
    } = &part.body
    else {
        return String::new();
    };
    debug_assert!(target.is_shell(), "format_shell_action on a non-shell part");
    let mut rendered = String::new();
    if !matches!(slice, ProjectionSlice::Output) {
        if let Some(command) = input.as_ref().map(|input| input.text()) {
            let command = command.trim_end();
            if !command.is_empty() {
                rendered.push_str(&match title.as_deref() {
                    Some(prompt) => render_prompt_override(prompt, command),
                    None => format!("$ {command}"),
                });
            }
        }
    }
    if matches!(slice, ProjectionSlice::Whole | ProjectionSlice::Output) {
        for block in output {
            let line = block.content.text().trim_end().to_string();
            if line.is_empty() {
                continue;
            }
            if !rendered.is_empty() {
                rendered.push('\n');
            }
            rendered.push_str(&line);
        }
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::push_block;
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

        assert!(record.is_terminal());
        assert_eq!(record.kind_label(), "shell");
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
        // The bash call and its output are one shell action, rendered as
        // the command line then the result.
        assert!(output.contains("$ cargo test"));
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
    fn shell_tool_results_merge_into_the_call_and_normalize_to_shell() {
        let session = AgentSession {
            path: PathBuf::from("codex-session.jsonl"),
            id: Some("abcdef123456".to_string()),
            cwd: Some("D:\\sivtr".to_string()),
            title: None,
            blocks: vec![
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: None,
                    label: None,
                    call_id: None,
                    text: "run it".to_string(),
                    start_line: None,
                },
                AgentBlock {
                    kind: AgentBlockKind::ToolCall,
                    timestamp: None,
                    label: Some("Bash".to_string()),
                    call_id: Some("c1".to_string()),
                    text: "{\"command\":\"cargo test\",\"description\":\"run the suite\"}"
                        .to_string(),
                    start_line: None,
                },
                AgentBlock {
                    kind: AgentBlockKind::ToolOutput,
                    timestamp: None,
                    label: Some("Bash".to_string()),
                    call_id: Some("c1".to_string()),
                    text: "{\"stdout\":\"ok\",\"exit_code\":0}".to_string(),
                    start_line: None,
                },
            ],
        };

        let records = WorkRecord::chat_turns(AgentProvider::Codex, &session);
        assert_eq!(records.len(), 1);
        // The call and its result are one shell action with the full
        // lifecycle; the description is the context line.
        assert_eq!(records[0].parts.len(), 2);
        let Some(WorkPartBody::Action {
            target,
            title,
            input,
            output,
            status,
            exit_code,
            ..
        }) = records[0].parts.get(1).map(|part| &part.body)
        else {
            panic!("tool part is an action");
        };
        assert!(target.is_shell());
        assert_eq!(title.as_deref(), Some("run the suite"));
        assert_eq!(
            input.as_ref().map(|input| input.text().into_owned()),
            Some("cargo test".to_string())
        );
        assert_eq!(output.len(), 1);
        assert_eq!(*status, WorkActionStatus::Completed);
        assert_eq!(*exit_code, Some(0));
    }

    #[test]
    fn results_without_a_stable_id_stay_separate_parts() {
        // No stable id: no implicit pairing. An id-less call and its output
        // remain two actions; the reducer never guesses by tool name.
        let session = AgentSession {
            path: PathBuf::from("codex-session.jsonl"),
            id: Some("abcdef123456".to_string()),
            cwd: Some("D:\\sivtr".to_string()),
            title: None,
            blocks: vec![
                AgentBlock {
                    kind: AgentBlockKind::User,
                    timestamp: None,
                    label: None,
                    call_id: None,
                    text: "go".to_string(),
                    start_line: None,
                },
                AgentBlock {
                    kind: AgentBlockKind::ToolCall,
                    timestamp: None,
                    label: Some("Bash".to_string()),
                    call_id: None,
                    text: "{\"command\":\"ls\"}".to_string(),
                    start_line: None,
                },
                AgentBlock {
                    kind: AgentBlockKind::ToolOutput,
                    timestamp: None,
                    label: Some("Bash".to_string()),
                    call_id: None,
                    text: "{\"stdout\":\"ok\"}".to_string(),
                    start_line: None,
                },
            ],
        };

        let records = WorkRecord::chat_turns(AgentProvider::Codex, &session);
        assert_eq!(records[0].parts.len(), 3);
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
        assert!(records[0].is_agent());
        assert_eq!(records[0].kind_label(), "ai");
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
        assert!(records[0]
            .parts
            .iter()
            .any(|part| part.message_role() == Some(MessageRole::System)
                && part.label() == Some("sivtr-memory")));
        assert!(records[0]
            .parts
            .iter()
            .any(|part| part.message_role() == Some(MessageRole::User)
                && part.text().contains("real task")));
        let input = records[0].input_text().unwrap_or_default();
        assert!(input.contains("<:skill:sivtr-memory:>"));
        assert!(input.contains("long instructions"));
        assert!(input.contains("real task"));
        assert_eq!(records[0].output_text().as_deref(), Some("done"));
    }

    #[test]
    fn chat_turn_records_ignore_interrupted_tool_use_noise() {
        // The interrupted-turn noise is dropped at parse time (push_block
        // filters scaffolding), so post-parse this is a plain two-turn
        // session: an empty-scaffold start and a normal continuation.
        let mut session = AgentSession {
            path: PathBuf::from("claude-session.jsonl"),
            id: Some("abcdef123456".to_string()),
            cwd: Some("D:\\sivtr".to_string()),
            title: None,
            blocks: Vec::new(),
        };
        for (kind, at, text) in [
            (
                AgentBlockKind::User,
                "2026-05-23T12:00:00Z",
                "[Request interrupted by user for tool use]",
            ),
            (AgentBlockKind::Assistant, "2026-05-23T12:00:01Z", "partial"),
            (AgentBlockKind::User, "2026-05-23T12:01:00Z", "continue"),
            (AgentBlockKind::Assistant, "2026-05-23T12:02:00Z", "done"),
        ] {
            push_block(&mut session, kind, Some(at.to_string()), None, text);
        }

        // Scaffolding never reaches the block list.
        assert_eq!(session.blocks.len(), 3);

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
        let mut session = AgentSession {
            path: PathBuf::from("claude-session.jsonl"),
            id: Some("abcdef123456".to_string()),
            cwd: Some("D:\\sivtr".to_string()),
            title: None,
            blocks: Vec::new(),
        };
        for (kind, at, text) in [
            (
                AgentBlockKind::User,
                "2026-05-23T12:00:00Z",
                "  <system-reminder>\nhidden\n</system-reminder>",
            ),
            (
                AgentBlockKind::User,
                "2026-05-23T12:00:01Z",
                "<command-args>--foo</command-args>",
            ),
            (
                AgentBlockKind::User,
                "2026-05-23T12:00:02Z",
                "<ide_selection>main.rs</ide_selection>",
            ),
            (
                AgentBlockKind::User,
                "2026-05-23T12:01:00Z",
                "real question",
            ),
            (AgentBlockKind::Assistant, "2026-05-23T12:02:00Z", "answer"),
        ] {
            push_block(&mut session, kind, Some(at.to_string()), None, text);
        }

        // Scaffolding never reaches the block list.
        assert_eq!(session.blocks.len(), 2);

        let records = WorkRecord::chat_turns(AgentProvider::Claude, &session);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].input_text().as_deref(), Some("real question"));
        assert_eq!(records[0].parts[1].text(), "answer");
    }
}
