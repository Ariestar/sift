use anyhow::Result;
use serde::Serialize;
use sivtr_core::config::SivtrConfig;
use sivtr_core::workspace;
use std::path::{Path, PathBuf};

use crate::cli::DoctorArgs;
use crate::commands::interactive;
use crate::commands::system::skill;
use crate::output;

pub fn execute(args: DoctorArgs) -> Result<()> {
    if args.fix && !args.json {
        return execute_interactive_fix();
    }

    let mut report = Report::default();
    report.run_checks(args.fix);

    if args.json {
        print_json(&report);
    } else {
        print_human(&report);
    }
    Ok(())
}

fn execute_interactive_fix() -> Result<()> {
    let mut report = Report::default();
    report.run_checks(false);

    print_human(&report);

    let fixable: Vec<&Check> = report
        .checks
        .iter()
        .filter(|c| c.status == Status::Fail)
        .collect();

    if fixable.is_empty() {
        output::blank();
        output::success("nothing to fix");
        return Ok(());
    }

    output::blank();
    output::info(format!(
        "{} issue(s) can be fixed automatically",
        fixable.len()
    ));
    for check in &fixable {
        output::detail(check.label, &check.detail);
    }

    if !interactive::confirm("Fix all issues?", true)? {
        output::plain("skipped fixes");
        return Ok(());
    }

    output::blank();
    let mut fixed_report = Report::default();
    fixed_report.run_checks(true);
    print_human(&fixed_report);
    Ok(())
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Status {
    Pass,
    Fail,
    Fixed,
    Manual,
}

#[derive(Debug, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub label: &'static str,
    pub status: Status,
    pub detail: String,
    pub hint: Option<String>,
}

impl Report {
    fn run_checks(&mut self, fix: bool) {
        self.check_binary();
        self.check_config(fix);
        self.check_session_dir();
        self.check_shell_hooks(fix);
        self.check_workspace_keys(fix);
        self.check_agent_hosts();
        self.check_mcp_registration(fix);
        self.check_skill(fix);
        self.check_providers();
        self.check_clipboard();
        self.check_update();
    }

    fn add(&mut self, check: Check) {
        self.checks.push(check);
    }

    fn check_binary(&mut self) {
        self.add(Check {
            name: "binary",
            label: "binary version",
            status: Status::Pass,
            detail: format!("sivtr {}", env!("CARGO_PKG_VERSION")),
            hint: None,
        });
    }

    fn check_config(&mut self, fix: bool) {
        let path = config_path();
        if path.exists() {
            self.add(Check {
                name: "config",
                label: "config file",
                status: Status::Pass,
                detail: path.display().to_string(),
                hint: None,
            });
        } else if fix {
            match SivtrConfig::init_default() {
                Ok(created) => self.add(Check {
                    name: "config",
                    label: "config file",
                    status: Status::Fixed,
                    detail: format!("created {}", created.display()),
                    hint: None,
                }),
                Err(e) => self.add(Check {
                    name: "config",
                    label: "config file",
                    status: Status::Manual,
                    detail: format!("failed to create: {e}"),
                    hint: Some("run `sivtr config init`".to_string()),
                }),
            }
        } else {
            self.add(Check {
                name: "config",
                label: "config file",
                status: Status::Fail,
                detail: "missing".to_string(),
                hint: Some("run `sivtr config init`".to_string()),
            });
        }
    }

    fn check_session_dir(&mut self) {
        // Terminal sessions live under data_dir()/workspaces/*/terminals/*.jsonl,
        // not under dirs::state_dir(). The latter is a false missing path on Windows.
        let base = workspace::data_dir().join("workspaces");
        if !base.exists() {
            self.add(Check {
                name: "session_dir",
                label: "terminal session storage",
                status: Status::Manual,
                detail: format!("no workspaces under {}", base.display()),
                hint: Some(
                    "run `sivtr init <shell>`, restart the shell, then run a command in a git repo"
                        .to_string(),
                ),
            });
            return;
        }

        let mut workspace_count = 0usize;
        let mut terminal_logs = 0usize;
        if let Ok(entries) = std::fs::read_dir(&base) {
            for entry in entries.flatten() {
                let terminals = entry.path().join("terminals");
                if !terminals.is_dir() {
                    continue;
                }
                workspace_count += 1;
                if let Ok(files) = std::fs::read_dir(&terminals) {
                    terminal_logs += files
                        .flatten()
                        .filter(|f| f.path().extension().and_then(|e| e.to_str()) == Some("jsonl"))
                        .count();
                }
            }
        }

        if terminal_logs > 0 {
            self.add(Check {
                name: "session_dir",
                label: "terminal session storage",
                status: Status::Pass,
                detail: format!(
                    "{} ({} workspace(s), {} terminal log(s))",
                    base.display(),
                    workspace_count,
                    terminal_logs
                ),
                hint: None,
            });
        } else {
            self.add(Check {
                name: "session_dir",
                label: "terminal session storage",
                status: Status::Manual,
                detail: format!(
                    "{} ({} workspace(s), 0 terminal logs)",
                    base.display(),
                    workspace_count
                ),
                hint: Some(
                    "hooks are installed, but no terminal logs yet — restart shell and run a command in a git repo"
                        .to_string(),
                ),
            });
        }
    }

    fn check_shell_hooks(&mut self, fix: bool) {
        let installed = detect_installed_shells();
        if !installed.is_empty() {
            self.add(Check {
                name: "shell_hooks",
                label: "shell hooks",
                status: Status::Pass,
                detail: installed.join(", "),
                hint: None,
            });
        } else if fix {
            let shell = detect_current_shell();
            match crate::commands::terminal::init::execute(&shell) {
                Ok(()) => self.add(Check {
                    name: "shell_hooks",
                    label: "shell hooks",
                    status: Status::Fixed,
                    detail: format!("installed for {shell}"),
                    hint: Some("restart your shell for hooks to take effect".to_string()),
                }),
                Err(e) => self.add(Check {
                    name: "shell_hooks",
                    label: "shell hooks",
                    status: Status::Manual,
                    detail: format!("auto-install failed: {e}"),
                    hint: Some("run `sivtr init <shell>`".to_string()),
                }),
            }
        } else {
            self.add(Check {
                name: "shell_hooks",
                label: "shell hooks",
                status: Status::Fail,
                detail: "none installed".to_string(),
                hint: Some(
                    "run `sivtr init bash` or `sivtr init zsh|powershell|nushell`".to_string(),
                ),
            });
        }
    }

    /// Workspaces whose stored roots predate the commondir key scheme become
    /// unreachable (their captured sessions vanish from queries). `--fix`
    /// re-keys and merges them; without it the check reports what needs it.
    fn check_workspace_keys(&mut self, fix: bool) {
        let result = if fix {
            workspace::migrate_workspace_keys()
        } else {
            workspace::inspect_workspace_keys()
        };
        let report = match result {
            Ok(report) => report,
            Err(e) => {
                self.add(Check {
                    name: "workspace_keys",
                    label: "workspace keys",
                    status: Status::Manual,
                    detail: format!("migration check failed: {e}"),
                    hint: None,
                });
                return;
            }
        };
        self.add(workspace_keys_check(report, fix));
    }

    fn check_skill(&mut self, fix: bool) {
        if skill::is_installed() {
            self.add(Check {
                name: "skill",
                label: "sivtr-memory skill",
                status: Status::Pass,
                detail: "installed".to_string(),
                hint: None,
            });
            return;
        }

        if fix {
            match skill::ensure_installed() {
                Ok(detail) => self.add(Check {
                    name: "skill",
                    label: "sivtr-memory skill",
                    status: Status::Fixed,
                    detail,
                    hint: None,
                }),
                Err(e) => self.add(Check {
                    name: "skill",
                    label: "sivtr-memory skill",
                    status: Status::Manual,
                    detail: format!("auto-install failed: {e}"),
                    hint: Some(skill::install_hint()),
                }),
            }
        } else {
            self.add(Check {
                name: "skill",
                label: "sivtr-memory skill",
                status: Status::Fail,
                detail: "missing".to_string(),
                hint: Some(skill::install_hint()),
            });
        }
    }

    fn check_agent_hosts(&mut self) {
        let hosts = crate::commands::system::mcp::detected_hosts();
        if hosts.is_empty() {
            self.add(Check {
                name: "agent_hosts",
                label: "agent hosts",
                status: Status::Manual,
                detail: "none detected".to_string(),
                hint: Some(
                    "install Claude Code, Cursor, Codex, or another supported host, then re-run doctor"
                        .to_string(),
                ),
            });
            return;
        }
        let names: Vec<&str> = hosts.iter().map(|p| p.name()).collect();
        self.add(Check {
            name: "agent_hosts",
            label: "agent hosts",
            status: Status::Pass,
            detail: names.join(", "),
            hint: None,
        });
    }

    fn check_mcp_registration(&mut self, fix: bool) {
        let registered = crate::commands::system::mcp::registered_targets();
        if !registered.is_empty() {
            let names: Vec<&str> = registered.iter().map(|p| p.name()).collect();
            self.add(Check {
                name: "mcp",
                label: "MCP registration",
                status: Status::Pass,
                detail: format!("sivtr entry present for {}", names.join(", ")),
                hint: None,
            });
            return;
        }

        if fix {
            let mcp_args = crate::cli::McpInstallArgs {
                providers: Vec::new(), // detect installed hosts
                location: crate::cli::McpLocation::Global,
                yes: true,
            };
            match crate::commands::system::mcp::install(&mcp_args) {
                Ok(()) => {
                    let after = crate::commands::system::mcp::registered_targets();
                    if after.is_empty() {
                        self.add(Check {
                            name: "mcp",
                            label: "MCP registration",
                            status: Status::Manual,
                            detail: "install ran but no sivtr MCP entry was found".to_string(),
                            hint: Some(
                                "run `sivtr mcp install -p all` and verify host config paths"
                                    .to_string(),
                            ),
                        });
                    } else {
                        let names: Vec<&str> = after.iter().map(|p| p.name()).collect();
                        self.add(Check {
                            name: "mcp",
                            label: "MCP registration",
                            status: Status::Fixed,
                            detail: format!("installed sivtr MCP for {}", names.join(", ")),
                            hint: None,
                        });
                    }
                }
                Err(e) => self.add(Check {
                    name: "mcp",
                    label: "MCP registration",
                    status: Status::Manual,
                    detail: format!("auto-install failed: {e}"),
                    hint: Some("run `sivtr mcp install` or `sivtr mcp install -p all`".to_string()),
                }),
            }
            return;
        }

        self.add(Check {
            name: "mcp",
            label: "MCP registration",
            status: Status::Fail,
            detail: "sivtr MCP entry missing in host configs".to_string(),
            hint: Some(
                "run `sivtr mcp install` or `sivtr mcp install -p claude,cursor`".to_string(),
            ),
        });
    }

    fn check_providers(&mut self) {
        match sivtr_core::archive::sync::provider_status() {
            Ok(statuses) => {
                let errors = statuses
                    .iter()
                    .filter(|status| status.error.is_some())
                    .count();
                let detail = statuses
                    .iter()
                    .map(|status| match &status.error {
                        Some(error) => format!("{}: error ({error})", status.name),
                        None => format!("{}: {}", status.name, status.sessions),
                    })
                    .collect::<Vec<_>>()
                    .join("  ");
                self.add(Check {
                    name: "providers",
                    label: "provider sessions",
                    status: if errors == 0 {
                        Status::Pass
                    } else {
                        Status::Manual
                    },
                    detail,
                    hint: (errors > 0)
                        .then(|| "one or more providers failed during archive sync".to_string()),
                });
            }
            Err(error) => self.add(Check {
                name: "providers",
                label: "provider sessions",
                status: Status::Manual,
                detail: format!("archive status failed: {error:#}"),
                hint: Some("run `sivtr sync --full` and inspect the report".to_string()),
            }),
        }
    }

    fn check_clipboard(&mut self) {
        if arboard::Clipboard::new().is_ok() {
            self.add(Check {
                name: "clipboard",
                label: "clipboard access",
                status: Status::Pass,
                detail: "available".to_string(),
                hint: None,
            });
        } else {
            self.add(Check {
                name: "clipboard",
                label: "clipboard access",
                status: Status::Manual,
                detail: "unavailable".to_string(),
                hint: Some("copy commands may not work in this environment".to_string()),
            });
        }
    }

    fn check_update(&mut self) {
        let current = env!("CARGO_PKG_VERSION");
        match super::update::latest_version() {
            Ok(latest) if latest.trim_start_matches('v') == current => self.add(Check {
                name: "update",
                label: "latest version",
                status: Status::Pass,
                detail: format!("sivtr {current} is up to date"),
                hint: None,
            }),
            Ok(latest) => self.add(Check {
                name: "update",
                label: "latest version",
                status: Status::Manual,
                detail: format!(
                    "new release available: sivtr {} (current {current})",
                    latest.trim_start_matches('v')
                ),
                hint: Some("run `sivtr update`".to_string()),
            }),
            Err(e) => self.add(Check {
                name: "update",
                label: "latest version",
                status: Status::Manual,
                detail: format!("could not check: {e}"),
                hint: None,
            }),
        }
    }
}

/// `"N workspace(s) could not be migrated (dir: reason; …)"` from a migration
/// report's skipped entries, sampling the first three reasons.
fn skipped_summary(skipped: &[(PathBuf, String)]) -> String {
    let reasons: Vec<String> = skipped
        .iter()
        .take(3)
        .map(|(path, reason)| {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("workspace");
            format!("{name}: {reason}")
        })
        .collect();
    let more = if skipped.len() > 3 {
        format!(", +{} more", skipped.len() - 3)
    } else {
        String::new()
    };
    format!(
        "{} workspace(s) could not be migrated ({}{more})",
        skipped.len(),
        reasons.join("; ")
    )
}

/// Map a migration report onto the `workspace_keys` check. Partial success
/// keeps the migrated/merged counts but must never report `Fixed` while any
/// workspace was skipped — its sessions stay unreachable.
fn workspace_keys_check(report: workspace::WorkspaceMigration, fix: bool) -> Check {
    let mut pending = Vec::new();
    if !report.migrated.is_empty() {
        pending.push(format!("{} to re-key", report.migrated.len()));
    }
    if !report.merged.is_empty() {
        pending.push(format!("{} to merge", report.merged.len()));
    }
    if !pending.is_empty() {
        let mut detail = pending.join(", ");
        if !report.skipped.is_empty() {
            detail.push_str("; ");
            detail.push_str(&skipped_summary(&report.skipped));
        }
        return Check {
            name: "workspace_keys",
            label: "workspace keys",
            status: if fix && report.skipped.is_empty() {
                Status::Fixed
            } else {
                Status::Fail
            },
            detail,
            hint: if fix {
                None
            } else {
                Some("run `sivtr doctor --fix`".to_string())
            },
        };
    }
    if !report.skipped.is_empty() {
        return Check {
            name: "workspace_keys",
            label: "workspace keys",
            status: Status::Manual,
            detail: skipped_summary(&report.skipped),
            hint: None,
        };
    }
    Check {
        name: "workspace_keys",
        label: "workspace keys",
        status: Status::Pass,
        detail: format!("{} workspace(s) on current scheme", report.current),
        hint: None,
    }
}

fn print_human(report: &Report) {
    let total = report.checks.len();
    let mut passed = 0;
    let mut fixed = 0;
    for check in &report.checks {
        output::info(format!("checking {}", check.label));
        match check.status {
            Status::Pass => {
                output::detail("ok", &check.detail);
                passed += 1;
            }
            Status::Fixed => {
                output::success(&check.detail);
                fixed += 1;
            }
            Status::Fail => {
                output::warning(&check.detail);
                if let Some(hint) = &check.hint {
                    output::hint(hint);
                }
            }
            Status::Manual => {
                output::detail("manual", &check.detail);
                if let Some(hint) = &check.hint {
                    output::hint(hint);
                }
            }
        }
    }
    output::blank();
    let ok = passed + fixed;
    let failed = count_status(report, Status::Fail);
    let manual = count_status(report, Status::Manual);
    if ok == total {
        output::success(format!("all {total} checks passed"));
    } else {
        output::warning(format!(
            "{ok}/{total} checks passed ({fixed} fixed, {failed} failed, {manual} manual)"
        ));
    }
}

fn count_status(report: &Report, status: Status) -> usize {
    report.checks.iter().filter(|c| c.status == status).count()
}

fn print_json(report: &Report) {
    let json = serde_json::to_string_pretty(report).unwrap_or_default();
    println!("{json}");
}

fn config_path() -> PathBuf {
    SivtrConfig::config_path().unwrap_or_else(|_| {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("sivtr")
            .join("config.toml")
    })
}

pub fn detect_installed_shells() -> Vec<String> {
    let mut installed = Vec::new();
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return installed,
    };

    for (name, rel_path, marker) in [
        ("bash", ".bashrc", "# >>> sivtr shell integration >>>"),
        ("zsh", ".zshrc", "# >>> sivtr shell integration >>>"),
    ] {
        let path = home.join(rel_path);
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if content.contains(marker) {
                    installed.push(name.to_string());
                }
            }
        }
    }

    if let Some(config_dir) = dirs::config_dir() {
        let nu_config = config_dir.join("nushell").join("config.nu");
        if nu_config.exists() {
            if let Ok(content) = std::fs::read_to_string(&nu_config) {
                if content.contains("# >>> sivtr shell integration >>>") {
                    installed.push("nushell".to_string());
                }
            }
        }
    }

    for cmd in &["pwsh", "powershell"] {
        if let Ok(Some(profile)) = crate::commands::terminal::init::shell_printed_path(
            cmd,
            &["-NoProfile", "-Command", "Write-Output $PROFILE"],
        ) {
            let path = Path::new(&profile);
            if path.exists() {
                if let Ok(content) = std::fs::read_to_string(path) {
                    if content.contains("# >>> sivtr shell integration >>>") {
                        installed.push("powershell".to_string());
                        break;
                    }
                }
            }
        }
    }

    installed
}

pub fn detect_current_shell() -> String {
    if let Ok(shell) = std::env::var("SHELL") {
        if shell.contains("zsh") {
            return "zsh".to_string();
        }
        if shell.contains("bash") {
            return "bash".to_string();
        }
        if shell.contains("nu") {
            return "nushell".to_string();
        }
    }
    if cfg!(windows) {
        "powershell".to_string()
    } else {
        "bash".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::workspace::WorkspaceMigration;

    fn partial_report() -> WorkspaceMigration {
        WorkspaceMigration {
            migrated: vec![("old-a".into(), "new-a".into())],
            merged: Vec::new(),
            skipped: vec![(PathBuf::from("old-b"), "rename failed: boom".into())],
            current: 1,
        }
    }

    #[test]
    fn partial_fix_never_reports_fixed_and_lists_skipped() {
        let check = workspace_keys_check(partial_report(), true);
        assert_eq!(check.status, Status::Fail);
        assert!(check.detail.contains("1 to re-key"));
        assert!(check
            .detail
            .contains("1 workspace(s) could not be migrated"));
        assert!(check.detail.contains("old-b: rename failed: boom"));
    }

    #[test]
    fn clean_fix_reports_fixed_without_skipped_noise() {
        let report = WorkspaceMigration {
            migrated: vec![("old-a".into(), "new-a".into())],
            merged: Vec::new(),
            skipped: Vec::new(),
            current: 1,
        };
        let check = workspace_keys_check(report, true);
        assert_eq!(check.status, Status::Fixed);
        assert_eq!(check.detail, "1 to re-key");
    }

    #[test]
    fn inspect_mode_keeps_fail_with_fix_hint() {
        let check = workspace_keys_check(partial_report(), false);
        assert_eq!(check.status, Status::Fail);
        assert_eq!(check.hint.as_deref(), Some("run `sivtr doctor --fix`"));
    }

    #[test]
    fn skipped_only_reports_manual_with_reasons() {
        let report = WorkspaceMigration {
            migrated: Vec::new(),
            merged: Vec::new(),
            skipped: vec![(PathBuf::from("old-b"), "rename failed: boom".into())],
            current: 2,
        };
        let check = workspace_keys_check(report, true);
        assert_eq!(check.status, Status::Manual);
        assert!(check.detail.contains("old-b: rename failed: boom"));
    }
}
