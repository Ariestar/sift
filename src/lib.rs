//! Library + CLI entry used by the `sivtr` binary, tests, and benches.

#![allow(private_interfaces)]
#![allow(private_bounds)]

pub mod cli;
pub mod commands;
pub mod mcp;
pub mod origins;
pub mod output;
pub mod pane;
pub mod remote;
pub mod tui;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{
    Commands, CopyCommand, CopyFlagArgs, CopyInvocation, CopySubcommand, DiffArgs,
    HotkeyPickAgentArgs, HotkeyServeArgs,
};
use commands::memory::copy::{plan_from_cli, CopyFilters, Projection};
use commands::memory::diff::{DiffRequest, DiffTextMode};
use std::io::IsTerminal;
use tui::workspace::WorkspaceFocus;

use sivtr_core::ai::AgentProvider;
use std::process::ExitCode;

/// Binary entry — keeps `main.rs` a one-liner so benches can depend on the lib.
pub fn cli_main() -> ExitCode {
    tui::panic::install();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if commands::browse::is_pick_cancelled(&error) => ExitCode::SUCCESS,
        Err(error) => {
            output::error(format!("{error:#}"));
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = cli::parse();
    output::set_color_choice(cli.color.into());
    let select_remotes = cli.all;

    match cli.command {
        Some(Commands::Run { command, args }) => {
            commands::terminal::run::execute(&command, &args)?;
        }
        Some(Commands::Pipe) => {
            commands::terminal::pipe::execute()?;
        }
        Some(Commands::Import) => {
            commands::terminal::import::execute()?;
        }
        Some(Commands::History(hist_cmd)) => {
            commands::system::history::execute(hist_cmd)?;
        }
        Some(Commands::Search(args)) => {
            commands::memory::search::execute(&args)?;
        }
        Some(Commands::Eval(args)) => {
            commands::memory::eval::execute(&args)?;
        }
        Some(Commands::Filter(args)) => {
            commands::memory::filter::execute(&args)?;
        }
        Some(Commands::Var(command)) => {
            commands::memory::var::execute(&command)?;
        }
        Some(Commands::Nav(args)) => {
            commands::memory::nav::execute(&args)?;
        }
        Some(Commands::Zoom(args)) => {
            commands::memory::zoom::execute(&args)?;
        }
        Some(Commands::Work(cmd)) => {
            commands::memory::work::execute(&cmd)?;
        }
        Some(Commands::Workspace(cmd)) => {
            commands::remote::workspace::execute(cmd)?;
        }
        Some(Commands::Show(args)) => {
            commands::memory::show::execute(&args)?;
        }
        Some(Commands::Publish(command)) => {
            commands::publish::execute(command)?;
        }
        Some(Commands::Mcp(cmd)) => {
            commands::system::mcp::execute(cmd)?;
        }
        Some(Commands::Serve(args)) => {
            commands::remote::serve::execute(&args)?;
        }
        Some(Commands::Share(cmd)) => {
            commands::remote::share::execute(cmd)?;
        }
        Some(Commands::Peer(cmd)) => {
            commands::remote::peer::execute(cmd)?;
        }
        Some(Commands::Remote(cmd)) => {
            commands::remote::execute(cmd)?;
        }
        Some(Commands::Group(cmd)) => {
            commands::remote::group::execute(cmd)?;
        }
        Some(Commands::Origin(cmd)) => {
            commands::remote::origin::execute(cmd)?;
        }
        Some(Commands::Hotkey(cmd)) => {
            commands::system::hotkey::execute(cmd)?;
        }
        Some(Commands::Sync(args)) => {
            commands::system::sync::execute(&args)?;
        }
        Some(Commands::Config(cfg_cmd)) => {
            commands::system::config::execute(cfg_cmd)?;
        }
        Some(Commands::Doctor(args)) => {
            commands::system::doctor::execute(args)?;
        }
        Some(Commands::Update) => commands::system::update::execute()?,
        Some(Commands::Setup) => {
            commands::system::setup::execute()?;
        }
        Some(Commands::Init { target }) => {
            commands::terminal::init::execute(&target)?;
        }
        Some(Commands::Copy(cmd)) => run_copy_command(*cmd)?,
        Some(Commands::Ci(inv)) => run_copy_invocation(Projection::Input, &inv)?,
        Some(Commands::Co(inv)) => run_copy_invocation(Projection::Output, &inv)?,
        Some(Commands::Cc(inv)) => run_copy_invocation(Projection::Command, &inv)?,
        Some(Commands::Diff(args)) => {
            run_diff(&args)?;
        }
        Some(Commands::Version(args)) => {
            commands::system::version::execute(args.verbose)?;
        }
        Some(Commands::Clear(args)) => {
            commands::terminal::clear::execute(args.all)?;
        }
        Some(Commands::Flush) => {
            commands::terminal::flush::execute()?;
        }
        Some(Commands::HotkeyServe(args)) => {
            run_hotkey_serve(&args)?;
        }
        Some(Commands::HotkeyPickAgent(args)) => {
            run_hotkey_pick_agent(&args)?;
        }
        Some(Commands::ServeDaemon) => {
            remote::daemon::run()?;
        }
        None => {
            if !std::io::stdin().is_terminal() {
                commands::terminal::pipe::execute()?;
            } else {
                run_workspace(select_remotes)?;
            }
        }
    }

    Ok(())
}

fn run_workspace(select_remotes: bool) -> Result<()> {
    let providers = AgentProvider::all()
        .iter()
        .map(|spec| spec.provider)
        .collect::<Vec<_>>();
    commands::finish_picker(
        commands::browse::run(&providers, select_remotes, WorkspaceFocus::Sessions)
            .context("工作区选择器失败")?,
        false,
        None,
        None,
        false,
    )
}

fn run_copy_command(cmd: CopyCommand) -> Result<()> {
    match cmd.mode {
        Some(CopySubcommand::In(inv)) => run_copy_invocation(Projection::Input, &inv),
        Some(CopySubcommand::Out(inv)) => run_copy_invocation(Projection::Output, &inv),
        Some(CopySubcommand::Cmd(inv)) => run_copy_invocation(Projection::Command, &inv),
        Some(CopySubcommand::External(tokens)) => {
            let (free, trailing) = split_external_tokens(&tokens)?;
            let flags = merge_copy_flags(&cmd.flags, &trailing);
            run_copy_tokens(Projection::Both, &free, &flags)
        }
        None => run_copy_tokens(Projection::Both, &[], &cmd.flags),
    }
}

fn run_copy_invocation(projection: Projection, inv: &CopyInvocation) -> Result<()> {
    run_copy_tokens(projection, &inv.tokens, &inv.flags)
}

fn run_copy_tokens(projection: Projection, tokens: &[String], flags: &CopyFlagArgs) -> Result<()> {
    let plan = plan_from_cli(projection, tokens, flags.pick, filters_from_flags(flags))
        .map_err(|message| anyhow::anyhow!(message))?;
    commands::memory::copy::execute(plan)
}

fn filters_from_flags(flags: &CopyFlagArgs) -> CopyFilters {
    CopyFilters {
        print: flags.print,
        ansi: flags.ansi,
        regex: flags.regex.clone(),
        lines: flags.lines.clone(),
        prompt: flags.prompt.clone(),
        cwd: None,
    }
}

fn merge_copy_flags(parent: &CopyFlagArgs, trailing: &CopyFlagArgs) -> CopyFlagArgs {
    CopyFlagArgs {
        ansi: parent.ansi || trailing.ansi,
        pick: parent.pick || trailing.pick,
        print: parent.print || trailing.print,
        regex: trailing.regex.clone().or_else(|| parent.regex.clone()),
        lines: trailing.lines.clone().or_else(|| parent.lines.clone()),
        prompt: trailing.prompt.clone().or_else(|| parent.prompt.clone()),
    }
}

fn split_external_tokens(tokens: &[String]) -> Result<(Vec<String>, CopyFlagArgs)> {
    let flag_start = tokens
        .iter()
        .position(|t| t.starts_with('-'))
        .unwrap_or(tokens.len());
    let free = tokens[..flag_start].to_vec();
    let flag_tokens = &tokens[flag_start..];
    let flags = if flag_tokens.is_empty() {
        CopyFlagArgs::default()
    } else {
        CopyFlagArgs::try_parse_from(flag_tokens).unwrap_or_else(|error| error.exit())
    };
    if free.len() > 2 {
        anyhow::bail!("too many arguments; expected `copy [address] [dialogues]`");
    }
    Ok((free, flags))
}

fn run_diff(args: &DiffArgs) -> Result<()> {
    let mode = if args.block {
        DiffTextMode::Block
    } else if args.input {
        DiffTextMode::Input
    } else if args.cmd {
        DiffTextMode::Command
    } else {
        DiffTextMode::Output
    };

    commands::memory::diff::execute(DiffRequest {
        left_selector: &args.left,
        right_selector: &args.right,
        mode,
        side_by_side: args.side_by_side,
    })
}

fn run_hotkey_serve(args: &HotkeyServeArgs) -> Result<()> {
    commands::system::hotkey::serve(args)
}

fn run_hotkey_pick_agent(args: &HotkeyPickAgentArgs) -> Result<()> {
    commands::system::hotkey::pick_agent(args)
}
