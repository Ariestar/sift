use anyhow::Result;
use sivtr_core::config::SivtrConfig;
use sivtr_core::workspace;

use crate::cli::{McpInstallArgs, McpLocation};
use crate::commands::interactive;
use crate::commands::system::doctor;
use crate::commands::system::skill;
use crate::output;

pub fn execute() -> Result<()> {
    output::info("sivtr setup — get from zero to working in one command");
    output::blank();

    let shells = pick_shells()?;
    let mcp_targets = pick_mcp_targets()?;
    let install_skill = want_skill_install()?;

    run_step("initializing config", || {
        let path = SivtrConfig::init_default()?;
        Ok(format!("created {}", path.display()))
    })?;

    run_step(
        &format!("installing hooks for {}", shells.join(", ")),
        || {
            for shell in &shells {
                crate::commands::terminal::init::execute(shell)?;
            }
            Ok(format!("installed for {}", shells.join(", ")))
        },
    )?;

    if !mcp_targets.is_empty() {
        run_step("installing MCP for selected agent hosts", || {
            for target in &mcp_targets {
                let mcp_args = McpInstallArgs {
                    providers: vec![target.clone()],
                    location: McpLocation::Global,
                    yes: true,
                };
                crate::commands::system::mcp::install(&mcp_args)?;
            }
            Ok(format!("installed for {} host(s)", mcp_targets.len()))
        })?;
    }

    if install_skill {
        // Soft-fail: skill is recommended but should not abort hooks/MCP setup.
        match skill::ensure_installed() {
            Ok(detail) => {
                output::info("installing sivtr-memory skill");
                output::success(detail);
            }
            Err(e) => {
                output::info("installing sivtr-memory skill");
                output::warning(format!("skipped: {e}"));
                output::hint(format!("install later with `{}`", skill::install_hint()));
            }
        }
    } else {
        output::info("sivtr-memory skill");
        output::success("already installed");
    }

    run_step("running smoke test", smoke_test)?;

    output::blank();
    output::success("setup complete");
    output::blank();
    output::plain("next steps:");
    output::detail("1", "restart your shell for hooks to take effect");
    output::detail(
        "2",
        "ask your agent: fix the latest terminal error (MCP/skill path)",
    );
    output::detail("3", "run `sivtr doctor` to verify everything is healthy");
    Ok(())
}

fn pick_shells() -> Result<Vec<String>> {
    let detected = doctor::detect_current_shell();
    let installed = doctor::detect_installed_shells();
    let shells = vec![
        "powershell".to_string(),
        "bash".to_string(),
        "zsh".to_string(),
        "nushell".to_string(),
    ];
    let mut defaults = Vec::new();
    if let Some(idx) = shells.iter().position(|s| s == &detected) {
        defaults.push(idx);
    }
    for shell in &installed {
        if let Some(idx) = shells.iter().position(|s| s == shell) {
            if !defaults.contains(&idx) {
                defaults.push(idx);
            }
        }
    }
    if defaults.is_empty() {
        defaults.push(0);
    }

    if !installed.is_empty() {
        output::detail("existing hooks", installed.join(", "));
    }

    let selected = interactive::multi_select("Which shells do you use?", &shells, &defaults)?;
    if selected.is_empty() {
        return Ok(vec![detected]);
    }
    Ok(selected.into_iter().map(|i| shells[i].clone()).collect())
}

fn pick_mcp_targets() -> Result<Vec<String>> {
    let detected = crate::commands::system::mcp::detect_targets();
    let managed = crate::commands::system::mcp::managed_targets().collect::<Vec<_>>();
    let all: Vec<String> = managed
        .iter()
        .map(|provider| provider.name().to_string())
        .collect();
    let defaults: Vec<usize> = detected
        .iter()
        .filter_map(|provider| managed.iter().position(|candidate| candidate == provider))
        .collect();

    let selected =
        interactive::multi_select("Install sivtr MCP into which agent hosts?", &all, &defaults)?;

    if selected.is_empty() {
        return Ok(Vec::new());
    }

    let targets: Vec<String> = selected
        .iter()
        .map(|&i| managed[i].command_name().to_string())
        .collect();
    Ok(targets)
}

fn want_skill_install() -> Result<bool> {
    if skill::is_installed() {
        return Ok(false);
    }
    // Default yes: first-time setup should teach agents how to use local memory.
    interactive::confirm(
        &format!("Install the `{}` skill for agents?", skill::SKILL_NAME),
        true,
    )
}

fn run_step(msg: &str, action: impl FnOnce() -> Result<String>) -> Result<()> {
    output::info(msg);
    match action() {
        Ok(detail) => {
            output::success(detail);
            Ok(())
        }
        Err(e) => {
            output::warning(format!("failed: {e}"));
            Err(e)
        }
    }
}

fn smoke_test() -> Result<String> {
    let has_terminal = workspace::resolve_current_workspace()?.is_some();
    let providers: Vec<&str> = sivtr_core::agents::AgentProvider::all()
        .iter()
        .map(|spec| spec.provider.name())
        .collect();
    let skill_status = if skill::is_installed() {
        "installed"
    } else {
        "missing"
    };
    Ok(format!(
        "workspace: {}; providers: {}; skill: {skill_status}",
        if has_terminal {
            "detected"
        } else {
            "none (not in a git repo)"
        },
        providers.join(", ")
    ))
}
