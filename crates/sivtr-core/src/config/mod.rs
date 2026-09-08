use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Top-level configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SivtrConfig {
    /// Editor settings.
    pub editor: EditorConfig,
    /// History settings.
    pub history: HistoryConfig,
    /// Unified archive sync settings.
    pub sync: SyncConfig,
    /// Global hotkey settings.
    pub hotkey: HotkeyConfig,
    /// TUI theme settings.
    pub theme: ThemeConfig,
    /// MCP stdio server settings.
    pub mcp: McpConfig,
    /// Browser publication service settings.
    pub publish: PublishConfig,
}

/// Editor configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorConfig {
    /// Editor command. If empty, auto-detect from PATH.
    /// Examples: "hx", "nvim", "vim", "code --wait"
    pub command: String,
}

/// History storage settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryConfig {
    /// Whether to automatically save captured output to history.
    pub auto_save: bool,
    /// Maximum number of history entries to keep (0 = unlimited).
    pub max_entries: usize,
}

/// Unified archive sync settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncConfig {
    /// How stale the archive may be (seconds since the last sync pass)
    /// before a query triggers an incremental re-sync. `0` re-lists on
    /// every query; raise it to trade freshness for latency.
    pub max_age_secs: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self { max_age_secs: 15 }
    }
}

/// Global hotkey configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    /// Hotkey chord used by `sivtr hotkey start`.
    pub chord: String,
}

/// TUI color scheme preference.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    /// Detect light/dark and truecolor support from the terminal environment.
    #[default]
    Auto,
    /// Always use the dark palette.
    Dark,
    /// Always use the light palette.
    Light,
}

/// TUI theme configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeConfig {
    /// Color scheme: `"auto"` (default), `"dark"`, or `"light"`.
    pub mode: ThemeMode,
}

/// MCP stdio server settings (shared by every agent host registration —
/// hosts all run plain `sivtr mcp serve`, behavior is configured here once).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    /// Seconds of no tool calls after which the stdio MCP server exits.
    /// Default 60 (idle exit on); set 0 to keep the server alive until the
    /// host closes stdin. Hosts respawn the server on the next tool use, so
    /// an idle server never lingers.
    pub idle_exit_secs: u64,
}

/// Endpoint for encrypted browser publications.  The endpoint is deliberately
/// the only configurable publication integration point in v1; override
/// `endpoint` in `config.toml` to use a self-hosted or staging service.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PublishConfig {
    pub endpoint: String,
}

// --- Defaults ---

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            auto_save: true,
            max_entries: 0, // unlimited
        }
    }
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            chord: "alt+y".to_string(),
        }
    }
}

impl Default for McpConfig {
    fn default() -> Self {
        Self { idle_exit_secs: 60 }
    }
}

impl Default for PublishConfig {
    fn default() -> Self {
        Self {
            endpoint: "https://share.hnnulwh.cn".to_string(),
        }
    }
}

// --- Loading / Saving ---

impl SivtrConfig {
    /// Load config from the default config file.
    /// If the file doesn't exist, return defaults.
    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        let config: SivtrConfig = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;
        Ok(config)
    }

    /// Save config to the default config file.
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write config file: {}", path.display()))?;
        Ok(())
    }

    /// Generate the default config file if it doesn't exist.
    /// Returns the path to the config file.
    pub fn init_default() -> Result<PathBuf> {
        let path = Self::config_path()?;
        if !path.exists() {
            let config = Self::default();
            config.save()?;
        }
        Ok(path)
    }

    /// Get the config file path.
    /// Windows: %APPDATA%/sivtr/config.toml
    /// macOS:   ~/Library/Application Support/sivtr/config.toml
    /// Linux:   ~/.config/sivtr/config.toml
    pub fn config_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("Cannot determine config directory"))?;
        Ok(config_dir.join("sivtr").join("config.toml"))
    }
}

/// Serialize a SivtrConfig to a pretty TOML string.
pub fn to_toml_string(config: &SivtrConfig) -> Result<String> {
    toml::to_string_pretty(config).context("Failed to serialize config to TOML")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_hotkey_config() {
        let config = SivtrConfig {
            hotkey: HotkeyConfig {
                chord: "alt+y".to_string(),
            },
            ..SivtrConfig::default()
        };

        let toml = to_toml_string(&config).expect("serialize hotkey config");

        assert!(toml.contains("[hotkey]"));
        assert!(toml.contains("chord = \"alt+y\""));
    }

    #[test]
    fn serializes_sync_config() {
        let config = SivtrConfig {
            sync: SyncConfig { max_age_secs: 120 },
            ..SivtrConfig::default()
        };

        let toml = to_toml_string(&config).expect("serialize sync config");

        assert!(toml.contains("[sync]"));
        assert!(toml.contains("max_age_secs = 120"));
        assert_eq!(SivtrConfig::default().sync.max_age_secs, 15);
    }

    #[test]
    fn serializes_mcp_idle_exit_config() {
        let config = SivtrConfig {
            mcp: McpConfig { idle_exit_secs: 60 },
            ..SivtrConfig::default()
        };

        let toml = to_toml_string(&config).expect("serialize MCP config");

        assert!(toml.contains("[mcp]"));
        assert!(toml.contains("idle_exit_secs = 60"));
        assert_eq!(SivtrConfig::default().mcp.idle_exit_secs, 60);
    }

    #[test]
    fn serializes_publish_endpoint() {
        let toml = to_toml_string(&SivtrConfig::default()).expect("serialize publish config");
        assert!(toml.contains("[publish]"));
        assert!(toml.contains("endpoint = \"https://share.hnnulwh.cn\""));

        assert!(
            toml::from_str::<SivtrConfig>("[publish]\nendpont = \"https://example.com\"\n")
                .is_err()
        );
    }

    #[test]
    fn theme_config_round_trips_and_rejects_typos() {
        let config = SivtrConfig {
            theme: ThemeConfig {
                mode: ThemeMode::Light,
            },
            ..SivtrConfig::default()
        };

        let toml = to_toml_string(&config).expect("serialize theme config");
        assert!(toml.contains("[theme]"));
        assert!(toml.contains("mode = \"light\""));

        // A typo such as `mode = "ligth"` must fail loudly instead of silently
        // falling back to auto (which made the setting look ignored).
        assert!(toml::from_str::<SivtrConfig>("[theme]\nmode = \"ligth\"\n").is_err());
        assert!(toml::from_str::<SivtrConfig>("[theme]\nmode = \"light\"\n").is_ok());

        // A misspelled key (`mod` instead of `mode`) is rejected too; serde
        // would otherwise ignore the unknown field and keep `mode` at auto.
        assert!(toml::from_str::<SivtrConfig>("[theme]\nmod = \"light\"\n").is_err());
    }
}
