//! Agent session providers and shared parsing skeletons.
//!
//! - [`model`]: shared session/block types and trait
//! - [`jsonl`]: JSONL discovery/parsing helpers (Claude/Codex/Hermes/Grok/Pi)
//! - [`sqlite`]: readonly SQLite helpers (OpenCode/OpenClaw)
//! - per-provider modules keep only storage paths + schema mapping

use std::path::Path;

use crate::session_source::SessionSource;
use anyhow::{Context, Result};

pub mod claude;
pub mod codex;
pub mod cursor;
pub mod dsh;
pub mod gemini;
pub mod generic;
pub mod goose;
pub mod grok;
pub mod hermes;
pub mod jsonl;
pub mod model;
pub mod openclaw;
pub mod opencode;
pub mod pi;
pub mod qoder;
pub mod qwen;
pub mod sqlite;
pub mod zcode;

pub use jsonl::{
    jsonl_files, list_chat_recording_sessions, list_recent_jsonl_sessions, list_sessions_matching,
    parse_jsonl_meta, parse_jsonl_session,
};
pub use model::*;
pub use sqlite::{open_readonly_db, system_time_from_millis, system_time_from_unix_secs};

#[derive(Clone, Copy)]
pub struct AgentProviderSpec {
    pub provider: AgentProvider,
    pub name: &'static str,
    pub command_name: &'static str,
    pub current_transcript_env: Option<&'static str>,
    pub current_session_id_env: Option<&'static str>,
    factory: fn() -> Box<dyn AgentSessionProvider>,
}

const AGENT_PROVIDER_SPECS: &[AgentProviderSpec] = &[
    AgentProviderSpec {
        provider: AgentProvider::Codex,
        name: "Codex",
        command_name: "codex",
        current_transcript_env: None,
        current_session_id_env: Some("CODEX_THREAD_ID"),
        factory: codex_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Claude,
        name: "Claude",
        command_name: "claude",
        current_transcript_env: Some("CLAUDE_TRANSCRIPT_PATH"),
        current_session_id_env: Some("CLAUDE_SESSION_ID"),
        factory: claude_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Cursor,
        name: "Cursor",
        command_name: "cursor",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: cursor_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Dsh,
        name: "Dsh",
        command_name: "dsh",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: dsh_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::OpenCode,
        name: "OpenCode",
        command_name: "opencode",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: opencode_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::OpenClaw,
        name: "OpenClaw",
        command_name: "openclaw",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: openclaw_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Grok,
        name: "Grok",
        command_name: "grok",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: grok_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Hermes,
        name: "Hermes",
        command_name: "hermes",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: hermes_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Pi,
        name: "Pi",
        command_name: "pi",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: pi_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Qoder,
        name: "Qoder",
        command_name: "qoder",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: qoder_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::QoderCn,
        name: "Qoder-CN",
        command_name: "qoder-cn",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: qoder_cn_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Gemini,
        name: "Gemini",
        command_name: "gemini",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: gemini_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Goose,
        name: "Goose",
        command_name: "goose",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: goose_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Qwen,
        name: "Qwen",
        command_name: "qwen",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: qwen_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Zcode,
        name: "ZCode",
        command_name: "zcode",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: zcode_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::ClaudeAi,
        name: "Claude.ai",
        command_name: "claude-ai",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: claude_ai_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::ChatGpt,
        name: "ChatGPT",
        command_name: "chatgpt",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: chatgpt_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::GeminiApps,
        name: "Gemini Apps",
        command_name: "gemini-apps",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: gemini_apps_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Amp,
        name: "Amp",
        command_name: "amp",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: amp_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Aider,
        name: "Aider",
        command_name: "aider",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: aider_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Antigravity,
        name: "Antigravity",
        command_name: "antigravity",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: antigravity_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::AntigravityCli,
        name: "Antigravity CLI",
        command_name: "antigravity-cli",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: antigravity_cli_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::CommandCode,
        name: "Command Code",
        command_name: "commandcode",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: commandcode_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Copilot,
        name: "Copilot",
        command_name: "copilot",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: copilot_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::DeepSeekTui,
        name: "DeepSeek TUI",
        command_name: "deepseek-tui",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: deepseek_tui_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Forge,
        name: "Forge",
        command_name: "forge",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: forge_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Gptme,
        name: "gptme",
        command_name: "gptme",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: gptme_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Iflow,
        name: "iFlow",
        command_name: "iflow",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: iflow_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Kilo,
        name: "Kilo",
        command_name: "kilo",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: kilo_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Kimi,
        name: "Kimi",
        command_name: "kimi",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: kimi_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::KimiWork,
        name: "Kimi Work",
        command_name: "kimi-work",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: kimi_work_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Kiro,
        name: "Kiro",
        command_name: "kiro",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: kiro_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::OpenHands,
        name: "OpenHands",
        command_name: "openhands",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: openhands_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Poolside,
        name: "Poolside",
        command_name: "poolside",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: poolside_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::PositAssistant,
        name: "Posit Assistant",
        command_name: "posit-assistant",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: posit_assistant_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::QwenPaw,
        name: "QwenPaw",
        command_name: "qwenpaw",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: qwenpaw_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Reasonix,
        name: "Reasonix",
        command_name: "reasonix",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: reasonix_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::RooCode,
        name: "RooCode",
        command_name: "roocode",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: roocode_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Shelley,
        name: "Shelley",
        command_name: "shelley",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: shelley_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Trae,
        name: "Trae",
        command_name: "trae",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: trae_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::TraeX,
        name: "TraeX",
        command_name: "traex",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: traex_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Vibe,
        name: "Mistral Vibe",
        command_name: "vibe",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: vibe_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::VSCodeCopilot,
        name: "VSCode Copilot",
        command_name: "vscode-copilot",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: vscode_copilot_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Windsurf,
        name: "Windsurf",
        command_name: "windsurf",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: windsurf_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Zed,
        name: "Zed",
        command_name: "zed",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: zed_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Zencoder,
        name: "Zencoder",
        command_name: "zencoder",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: zencoder_provider,
    },
];

fn codex_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::codex::CodexProvider)
}

fn claude_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::claude::ClaudeProvider)
}

fn cursor_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::cursor::CursorProvider)
}

fn dsh_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::dsh::DshProvider)
}

fn opencode_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::opencode::OpenCodeProvider::default())
}

fn openclaw_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::openclaw::OpenClawProvider)
}

fn grok_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::grok::GrokProvider)
}

fn hermes_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::hermes::HermesProvider)
}

fn pi_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::pi::PiProvider)
}

fn qoder_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::qoder::QoderProvider)
}

fn qoder_cn_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::qoder::QoderCnProvider)
}

fn gemini_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::gemini::GeminiProvider)
}

fn goose_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::goose::GooseProvider)
}

fn qwen_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::qwen::QwenProvider)
}

fn zcode_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::zcode::ZcodeProvider)
}

macro_rules! generic_factory {
    ($name:ident, $provider:ident) => {
        fn $name() -> Box<dyn AgentSessionProvider> {
            Box::new(crate::agents::generic::GenericProvider::new(
                AgentProvider::$provider,
            ))
        }
    };
}

generic_factory!(amp_provider, Amp);
generic_factory!(aider_provider, Aider);
generic_factory!(antigravity_provider, Antigravity);
generic_factory!(antigravity_cli_provider, AntigravityCli);
generic_factory!(commandcode_provider, CommandCode);
generic_factory!(copilot_provider, Copilot);
generic_factory!(deepseek_tui_provider, DeepSeekTui);
generic_factory!(forge_provider, Forge);
generic_factory!(gptme_provider, Gptme);
generic_factory!(iflow_provider, Iflow);
generic_factory!(kilo_provider, Kilo);
generic_factory!(kimi_provider, Kimi);
generic_factory!(kimi_work_provider, KimiWork);
generic_factory!(kiro_provider, Kiro);
generic_factory!(openhands_provider, OpenHands);
generic_factory!(poolside_provider, Poolside);
generic_factory!(posit_assistant_provider, PositAssistant);
generic_factory!(qwenpaw_provider, QwenPaw);
generic_factory!(reasonix_provider, Reasonix);
generic_factory!(roocode_provider, RooCode);
generic_factory!(shelley_provider, Shelley);
generic_factory!(trae_provider, Trae);
generic_factory!(traex_provider, TraeX);
generic_factory!(vibe_provider, Vibe);
generic_factory!(vscode_copilot_provider, VSCodeCopilot);
generic_factory!(windsurf_provider, Windsurf);
generic_factory!(zed_provider, Zed);
generic_factory!(zencoder_provider, Zencoder);
generic_factory!(claude_ai_provider, ClaudeAi);
generic_factory!(chatgpt_provider, ChatGpt);
generic_factory!(gemini_apps_provider, GeminiApps);

impl AgentProvider {
    pub fn all() -> &'static [AgentProviderSpec] {
        AGENT_PROVIDER_SPECS
    }

    pub fn from_command_name(value: &str) -> Option<Self> {
        Self::all()
            .iter()
            .find(|spec| spec.command_name.eq_ignore_ascii_case(value))
            .map(|spec| spec.provider)
    }

    pub fn spec(self) -> &'static AgentProviderSpec {
        Self::all()
            .iter()
            .find(|spec| spec.provider == self)
            .expect("agent provider registry must contain every AgentProvider variant")
    }

    pub fn name(self) -> &'static str {
        self.spec().name
    }

    pub fn command_name(self) -> &'static str {
        self.spec().command_name
    }

    pub fn current_transcript_env(self) -> Option<&'static str> {
        self.spec().current_transcript_env
    }

    pub fn current_session_id_env(self) -> Option<&'static str> {
        self.spec().current_session_id_env
    }

    /// Whether this provider has a native local source to sync. Web exports
    /// are imported explicitly and therefore belong to the archive namespace
    /// without participating in native-source reconciliation.
    pub fn has_native_source(self) -> bool {
        !matches!(self, Self::ClaudeAi | Self::ChatGpt | Self::GeminiApps)
    }

    pub fn session_provider(self) -> Box<dyn AgentSessionProvider> {
        (self.spec().factory)()
    }

    /// Registered provider CLI names (`codex`, `claude`, …), registry order.
    pub fn command_names() -> impl Iterator<Item = &'static str> {
        Self::all().iter().map(|spec| spec.command_name)
    }

    /// Comma-separated registered provider CLI names for help and errors.
    pub fn command_names_csv() -> String {
        Self::command_names().collect::<Vec<_>>().join(", ")
    }
}

use crate::record::WorkRecord;

impl SessionSource for AgentProvider {
    fn namespace(&self) -> &'static str {
        self.command_name()
    }

    fn list_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        self.session_provider()
            .list_recent_sessions(cwd)
            .with_context(|| format!("Failed to list {} sessions", self.name()))
    }

    fn parse_file(&self, path: &Path) -> Result<Vec<WorkRecord>> {
        let session = self
            .session_provider()
            .parse_session_file(path)
            .with_context(|| {
                format!("Failed to parse {} session {}", self.name(), path.display())
            })?;
        Ok(WorkRecord::chat_turns(*self, &session))
    }
}

#[cfg(test)]
mod tests {
    use super::AgentProvider;
    use std::collections::HashSet;

    #[test]
    fn registry_has_unique_expanded_provider_names() {
        let names: HashSet<_> = AgentProvider::command_names().collect();
        assert!(names.len() >= 40);
        assert_eq!(names.len(), AgentProvider::all().len());
        for spec in AgentProvider::all() {
            assert_eq!(
                AgentProvider::from_command_name(spec.command_name),
                Some(spec.provider)
            );
        }
    }
}
