use clap::{ArgGroup, Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sivtr_core::ai::AgentProvider;
use sivtr_core::record::WorkOutcome;
use sivtr_core::search::{Field, PartKind, Sort};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::LazyLock;

use crate::commands::memory::show::WorkSetOutputFormat;

mod mcp;
mod publish;
mod remote;
pub use mcp::*;
pub use publish::*;
pub use remote::*;

pub(crate) const TIME_FILTER_HELP: &str = "Accepts RFC3339 timestamps, Unix seconds/milliseconds, relative durations like 30m, 2h, 7d, or aliases like today, yesterday, tomorrow, this morning, this afternoon, this evening, tonight, and now.";

/// After-help for `copy`, generated once from the agent registry.
static COPY_AFTER_HELP: LazyLock<String> = LazyLock::new(|| {
    let providers = AgentProvider::command_names_csv();
    format!(
        "\
Grammar:
  sivtr copy [address] [dialogues] [flags]
  sivtr copy (in|out|cmd) [address] [dialogues] [flags]

Slots:
  address     same source/ref language as search/show
              omit → current terminal session
              examples: codex, codex/SESSION, terminal/s/12, desk:codex, → p1
  dialogues   relative to newest: N or A..B (default 1)
              only when address does not already pin a record
  projection  both (default) | in | out | cmd

Sources (same as search):
  terminal, agent, {providers}, plus scope:path remotes

Aliases:
  sivtr c / ci / co / cc

Filters:
  `--regex` then `--lines` after projection.

Examples:
  sivtr copy
  sivtr copy 3 --print
  sivtr copy out 2..4
  sivtr copy codex
  sivtr copy out codex 2..4
  sivtr copy codex/abc/12
  sivtr copy terminal/s/12/p3
  sivtr copy in --prompt ':'
"
    )
});

const COPY_INPUT_AFTER_HELP: &str = "\
`sivtr copy in [address] [dialogues]` → projection=input.

Examples:
  sivtr copy in
  sivtr copy in 3 --print
  sivtr copy in codex
  sivtr copy in --prompt \":\"
";

const COPY_OUTPUT_AFTER_HELP: &str = "\
`sivtr copy out [address] [dialogues]` → projection=output.

Examples:
  sivtr copy out
  sivtr copy out 2..5
  sivtr copy out codex
  sivtr copy out codex/abc 3
";

const COPY_COMMAND_AFTER_HELP: &str = "\
`sivtr copy cmd [address] [dialogues]` → bare command text (terminal).

Examples:
  sivtr copy cmd
  sivtr copy cmd 3
  sivtr copy cmd 2..5
";

const DIFF_AFTER_HELP: &str = "\
Defaults:
  `sivtr diff <left> <right>` compares two command blocks from the current session.
  The default content mode is `--output`.

Selector Semantics:
  Selection is relative to the newest command block.
  `1` means the last block, `2` means the 2nd-last block.
  Each selector must resolve to exactly one block.

Modes:
  --output         Compare command output (default)
  --block          Compare input + output
  --input          Compare input with prompt
  --cmd            Compare bare command text

View:
  Unified diff is the default output.
  --side-by-side   Show a two-column text view

Examples:
  sivtr diff 1 2
  sivtr diff 3 1 --block
  sivtr diff 2 1 --side-by-side
";

const ZOOM_AFTER_HELP: &str = "\
Examples:
  sivtr zoom
  sivtr zoom @last -C 3
  sivtr zoom @panics --before 5 --after 1 --save ctx
  sivtr zoom terminal/session_1/12 -C 2 -f timeline

Behavior:
  Expands each target WorkRecord with neighboring records from the same session.
  Target defaults to @last. WorkSet item selectors are 1-based, e.g. @last[1], @last[1..5], or @last[1,3,8].
  --context sets both sides; --before and --after override the corresponding side.
";

/// After-help for `search`/`filter`, generated once from the agent registry.
static SEARCH_AFTER_HELP: LazyLock<String> = LazyLock::new(|| {
    let providers = AgentProvider::command_names_csv();
    format!(
        r##"
Target selectors:
  terminal[/<session>[/<record>[/p<part>]]]  Search terminal command records
  agent[/<session>[/<turn>[/p<part>]]]       Search all AI/agent records
  <provider>[/<session>[/<turn>[/p<part>]]] Search one provider: {providers}
  <remote>:<selector>                      Run the same selector against a configured remote
  Use * for wildcard path segments, e.g. terminal/*/3 or pi/*/*.

Filters:
  --match <regex>       Case-insensitive regex content filter
  --exclude <regex>     Case-insensitive regex exclusion filter
  --in <field>          content, title, session, input, output, command, or all
  --kind <kind>        Part kind: prompt, command, user, assistant, tool,
                        tool_call, tool_result, skill, thinking, output, or error
  --status <status>     success, failure, or unknown
  --exit-code <code>    Exact terminal process exit code
  --min-duration <dur>  Minimum command duration, e.g. 500ms, 2s, 1m
  --max-duration <dur>  Maximum command duration, e.g. 500ms, 2s, 1m
  --sort <sort>         newest, oldest, duration, duration-asc, exit-code, exit-code-asc
  --last <duration>     Time window, e.g. 30m, 2h, 7d
  --since/--until       Absolute time, relative duration, or aliases: today, yesterday, tomorrow,
                        this morning, this afternoon, this evening, tonight, now
  --latest <n>          Return the latest n matching anchors
  --limit <n>           Cap result anchors after latest/sort
  --exclude-current     Exclude the current agent session from agent searches
  --other               Alias for --exclude-current
  --json                Alias for --format workset
  --refs                Alias for --format refs
  --format <format>    WorkSet output format: full, timeline, compact, md, refs, or workset
  --save <name>        Save the result WorkSet as @name

WorkSets:
  Every search creates a WorkSet, saves it as @last, and prints it in the selected output form.
  Use @name as a search target to refine a saved WorkSet.

Notes:
  When neither --latest nor --limit is set, search defaults to --latest 5.
  Content hits return exact part refs such as `.../p3`.

Examples:
  sivtr search terminal --status failure --latest 1 --json
  sivtr s terminal --status fail -m "panic|failed" -v "example|sample" --latest 20 --refs
  sivtr s terminal -m "panic|failed" --save panics
  sivtr s @panics -v "demo" -i title -f timeline
  sivtr search pi --match "merge|conflict" --latest 20 --format timeline
  sivtr search pi/019e5941 --match "cargo test" --format md
  sivtr search pi/019e5941/7 --format workset
  sivtr search terminal/session_13104/3/p3 --format workset
  sivtr search desk:terminal --status failure --latest 5 --refs
"##
    )
});

const WORK_SESSIONS_AFTER_HELP: &str = "\
Defaults:
  `sivtr work sessions` lists session markers for the current workspace.
  Output stays at marker level and does not print full session content.

Scope:
  Terminal sessions from the current git workspace are always included.
  `--provider` limits which local AI providers are scanned when no source is given.

Examples:
  sivtr work sessions
  sivtr work sessions --provider codex
  sivtr work sessions desk:agent
  sivtr work sessions --json
";

const WORK_RECORDS_AFTER_HELP: &str = "\
Defaults:
  `sivtr work records <source>` projects a source to record anchors.
  Piped stdout emits WorkSet JSON; terminal stdout defaults to full unless `--refs` or `-f` is used.

Sources:
  terminal/<session>
  <provider>/<session>
  <remote>:terminal/<session>
  <remote>:<provider>/<session>
  @last, @name, @name[1,3], @

Examples:
  sivtr work records terminal/session_123 --refs
  sivtr work records desk:terminal/session_123 --refs
  sivtr work records @last[1] -f timeline
  sivtr work records @ --json
";

const WORK_PARTS_AFTER_HELP: &str = r#"
Defaults:
  `sivtr work parts <source>` projects source anchors to part anchors.
  Piped stdout emits WorkSet JSON; terminal stdout defaults to full unless `--refs` or `-f` is used.

Filters:
  `--kind tool` selects tool calls and results.
  `--kind tool_call` selects calls only.
  `--kind skill` selects skill invocations.
  `--match <regex>` filters part content.

Examples:
  sivtr work parts @last[1] --kind assistant --refs
  sivtr work parts desk:pi/019df7fb/3 --kind tool --refs
  sivtr work parts pi/019df7fb/3 --kind tool_result -m "error|failed" --refs
  sivtr s agent -m "ssh.github.com" | sivtr work parts @ --kind output | sivtr s @ -m "main -> main" | sivtr show @ --full
"#;

const HOTKEY_AFTER_HELP: &str = "\
Examples:
  sivtr hotkey start
  sivtr hotkey start --chord alt+y
  sivtr hotkey start --provider claude
  sivtr hotkey status
  sivtr hotkey stop

Behavior:
  The hotkey daemon registers one global shortcut and opens a new
  terminal window for picking from current AI sessions.
";

/// sivtr - Terminal output workspace.
/// Capture, browse, search, select, and export terminal output.
#[derive(Parser, Debug)]
#[command(name = "sivtr", version, about, long_about = None)]
pub struct Cli {
    /// When to use color in status and diagnostic messages
    #[arg(long, value_enum, default_value_t = ColorArg::Auto, global = true)]
    pub color: ColorArg,

    /// Select mounted remotes in the workspace browser
    #[arg(long = "all", default_value_t = false, global = true)]
    pub all: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum ColorArg {
    #[default]
    Auto,
    Always,
    Never,
}

impl From<ColorArg> for crate::output::ColorChoice {
    fn from(value: ColorArg) -> Self {
        match value {
            ColorArg::Auto => Self::Auto,
            ColorArg::Always => Self::Always,
            ColorArg::Never => Self::Never,
        }
    }
}

#[derive(Args, Debug)]
pub struct DoctorArgs {
    /// Fix detected problems automatically
    #[arg(short = 'f', long = "fix")]
    pub fix: bool,

    /// Output structured JSON for agent/skill consumption
    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Wrap a command execution and capture its output
    Run {
        /// The command to run
        command: String,
        /// Arguments to pass to the command
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Read from stdin pipe (e.g., `cmd | sivtr`)
    Pipe,

    /// Open the current session log
    Import,

    /// Manage output history
    History(HistoryCommand),

    /// Search captured terminal and AI workspace sessions
    #[command(visible_alias = "s")]
    Search(SearchArgs),

    /// Benchmark retrieval quality against golden queries
    Eval(EvalArgs),

    /// Filter a WorkSet stream or selector
    Filter(FilterArgs),

    /// Manage named WorkSet vars
    Var(VarCommand),

    /// Navigate WorkSet anchors with a motion expression
    Nav(NavArgs),

    /// Expand each target WorkRecord with neighboring records from the same session
    #[command(after_help = ZOOM_AFTER_HELP)]
    Zoom(ZoomArgs),

    /// Traverse workspace sessions, records, and parts without printing full content
    Work(WorkCommand),

    /// List known local workspaces
    #[command(visible_alias = "ws", alias = "wb")]
    Workspace(WorkspaceCommand),

    /// Show a captured terminal or AI workspace ref
    Show(ShowArgs),

    /// Publish an encrypted, browser-readable conversation snapshot
    Publish(PublishCommand),

    /// Manage the read-only MCP server for agent hosts
    Mcp(McpCommand),

    /// Manage the local sivtr remote-memory daemon
    Serve(ServeCommand),

    /// Manage workspaces explicitly shared by this device
    Share(ShareCommand),

    /// Manage device identities known to this daemon
    Peer(PeerCommand),

    /// Manage remotes for the current workspace (like `git remote`)
    Remote(RemoteCommand),

    /// Manage groups: a named set of devices that share memory with each other
    #[command(visible_alias = "g")]
    Group(GroupCommand),

    /// Manage source origins: one name for every addressable memory source
    /// (local workspace alias or remote mount), renamed through one command
    Origin(OriginCommand),

    /// Manage configuration
    Config(ConfigCommand),

    /// Diagnose installation and environment
    Doctor(DoctorArgs),

    /// Update sivtr to the latest release
    Update,

    /// One-command setup: detect environment, install hooks/config/MCP, smoke test
    Setup,

    /// Generate shell integration or desktop shortcut helpers
    Init {
        /// Integration target: powershell, bash, zsh, nushell, all, tmux, linux-shortcut, macos-shortcut, show, uninstall
        #[arg(value_name = "TARGET", allow_hyphen_values = true)]
        target: String,
    },

    /// Copy recent command blocks to clipboard
    #[command(visible_alias = "c")]
    Copy(Box<CopyCommand>),

    /// Alias for `copy in`
    #[command(name = "ci", hide = true)]
    Ci(CopyInvocation),

    /// Alias for `copy out`
    #[command(name = "co", hide = true)]
    Co(CopyInvocation),

    /// Alias for `copy cmd`
    #[command(name = "cc", hide = true)]
    Cc(CopyInvocation),

    /// Compare two recent command blocks in the current session
    #[command(after_help = DIFF_AFTER_HELP)]
    Diff(DiffArgs),

    /// Manage the global AI session picker hotkey
    #[command(after_help = HOTKEY_AFTER_HELP)]
    Hotkey(HotkeyCommand),

    /// Sync terminal and agent sessions into the unified archive
    Sync(SyncArgs),

    /// Show version and build diagnostics
    Version(VersionArgs),

    /// Clear session logs
    Clear(ClearArgs),

    /// Internal: flush console buffer to session log (called by shell hook)
    #[command(hide = true)]
    Flush,

    /// Internal: run the Windows hotkey daemon loop
    #[command(hide = true)]
    HotkeyServe(HotkeyServeArgs),

    /// Internal: open the AI session picker from the Windows hotkey daemon
    #[command(hide = true)]
    HotkeyPickAgent(HotkeyPickAgentArgs),

    /// Internal: run the remote-memory daemon
    #[command(hide = true)]
    ServeDaemon,
}

#[derive(Args, Debug)]
pub struct CopyCommand {
    #[command(subcommand)]
    pub mode: Option<CopySubcommand>,

    /// Flags for bare `sivtr copy` (free tokens arrive via [`CopySubcommand::External`]).
    #[command(flatten)]
    pub flags: CopyFlagArgs,
}

/// Projection sugar or free `[address] [dialogues]` tokens.
#[derive(Subcommand, Debug)]
pub enum CopySubcommand {
    /// Projection: input only
    #[command(after_help = COPY_INPUT_AFTER_HELP)]
    In(CopyInvocation),

    /// Projection: output only
    #[command(after_help = COPY_OUTPUT_AFTER_HELP)]
    Out(CopyInvocation),

    /// Projection: bare command only
    #[command(after_help = COPY_COMMAND_AFTER_HELP)]
    Cmd(CopyInvocation),

    /// Free tokens: `[address] [dialogues]` plus optional trailing flags.
    #[command(external_subcommand)]
    External(Vec<String>),
}

/// Positionals + flags shared by `copy in|out|cmd` and top-level `ci|co|cc`.
#[derive(Args, Debug, Clone, Default)]
pub struct CopyInvocation {
    /// Optional address then optional dialogues (`N` or `A..B`).
    #[arg(value_name = "ADDRESS|DIALOGUES")]
    pub tokens: Vec<String>,

    #[command(flatten)]
    pub flags: CopyFlagArgs,
}

/// Top-level CLI parse: inject registry-generated after-help, then parse argv once.
pub fn parse() -> Cli {
    let mut cmd = Cli::command();
    if let Some(sub) = cmd.find_subcommand_mut("search") {
        *sub = std::mem::take(sub).after_help(SEARCH_AFTER_HELP.as_str());
    }
    if let Some(sub) = cmd.find_subcommand_mut("filter") {
        *sub = std::mem::take(sub).after_help(SEARCH_AFTER_HELP.as_str());
    }
    if let Some(sub) = cmd.find_subcommand_mut("copy") {
        *sub = std::mem::take(sub).after_help(COPY_AFTER_HELP.as_str());
    }
    let matches = cmd.get_matches();
    Cli::from_arg_matches(&matches).unwrap_or_else(|error| error.exit())
}

/// Flag-only args (no positional selector) used on bare `sivtr copy` and after a selector token.
#[derive(Parser, Debug, Clone, Default)]
#[command(no_binary_name = true, disable_help_subcommand = true)]
pub struct CopyFlagArgs {
    /// Copy the ANSI-decorated version when available
    #[arg(long)]
    pub ansi: bool,

    /// Open the interactive picker
    #[arg(long)]
    pub pick: bool,

    /// Print the copied text after copying
    #[arg(long)]
    pub print: bool,

    /// Keep only lines matching this regex
    #[arg(long, value_name = "PATTERN")]
    pub regex: Option<String>,

    /// Keep only selected 1-based lines, for example `10:20` or `1,3,8:12`
    #[arg(long, value_name = "SPEC")]
    pub lines: Option<String>,

    /// Prompt text used in copied input instead of the original shell prompt
    #[arg(long = "prompt", value_name = "TEXT")]
    pub prompt: Option<String>,
}

#[derive(Args, Debug, Clone)]
#[command(group(
    ArgGroup::new("diff_content_mode")
        .args(["output", "block", "input", "cmd"])
        .multiple(false)
))]
pub struct DiffArgs {
    /// Left selector, for example `1`
    #[arg(value_name = "LEFT")]
    pub left: String,

    /// Right selector, for example `2`
    #[arg(value_name = "RIGHT")]
    pub right: String,

    /// Compare output text (default)
    #[arg(long)]
    pub output: bool,

    /// Compare input + output
    #[arg(long)]
    pub block: bool,

    /// Compare input with prompt
    #[arg(long)]
    pub input: bool,

    /// Compare bare command text
    #[arg(long)]
    pub cmd: bool,

    /// Show side-by-side text output instead of unified diff
    #[arg(long = "side-by-side")]
    pub side_by_side: bool,
}

#[derive(Args, Debug, Clone)]
pub struct SearchArgs {
    /// Source selector, e.g. terminal, agent, pi, pi/<session>/<turn>, terminal/<session>/<record>, @last, or @ctx[1,3].
    /// Prefix with `all:` (e.g. all:agent) to search every origin — all local workspaces plus the current workspace's remote mounts — as one corpus
    #[arg(value_name = "SOURCE")]
    pub source: String,

    /// Plain-text search query: BM25 ranks the source by these terms (no
    /// regex). With --match present, the regex still bounds the set and this
    /// query ranks it.
    #[arg(value_name = "QUERY")]
    pub query: Option<String>,

    /// Case-insensitive regex content filter (optional refinement; bounds
    /// the set before relevance ranking)
    #[arg(short = 'm', long = "match", value_name = "REGEX")]
    pub match_: Option<String>,

    /// Case-insensitive regex exclusion filter
    #[arg(short = 'v', long, value_name = "REGEX")]
    pub exclude: Option<String>,

    /// Field to match: content, title, session, input, output, command, or all
    #[arg(short = 'i', long = "in", default_value_t = Field::default(), value_name = "FIELD")]
    pub in_field: Field,

    /// Part kind filter: prompt, command, user, assistant, tool, tool_call, tool_result, skill, thinking, output, or error
    #[arg(long, value_name = "KIND")]
    pub kind: Option<PartKind>,

    /// Record status filter: success, failure, or unknown
    #[arg(long, value_name = "STATUS")]
    pub status: Option<WorkOutcome>,

    /// Exact terminal process exit code filter
    #[arg(long, value_name = "CODE")]
    pub exit_code: Option<i32>,

    /// Minimum command duration filter, e.g. 500ms, 2s, 1m
    #[arg(long, value_name = "DURATION")]
    pub min_duration: Option<String>,

    /// Maximum command duration filter, e.g. 500ms, 2s, 1m
    #[arg(long, value_name = "DURATION")]
    pub max_duration: Option<String>,

    /// Result sort: newest (default), relevance (default with a QUERY or --match), oldest, duration, duration-asc, exit-code, or exit-code-asc
    #[arg(long, value_name = "SORT")]
    pub sort: Option<Sort>,

    /// Workspace directory used to resolve current AI sessions
    #[arg(long, value_name = "PATH")]
    pub cwd: Option<PathBuf>,

    /// Only search content at or after this time.
    #[arg(long, value_name = "TIME", help = TIME_FILTER_HELP)]
    pub since: Option<String>,

    /// Only search content at or before this time.
    #[arg(long, value_name = "TIME", help = TIME_FILTER_HELP)]
    pub until: Option<String>,

    /// Only search content within this recent duration, e.g. 30m, 2h, 7d.
    #[arg(long, value_name = "DURATION")]
    pub last: Option<String>,

    /// Return the latest N matching anchors. Defaults to 5 when neither --latest nor --limit is set.
    #[arg(long, value_name = "N")]
    pub latest: Option<usize>,

    /// Maximum number of result anchors to print (hard ceiling after latest/sort)
    #[arg(short = 'l', long, value_name = "N")]
    pub limit: Option<usize>,

    /// Exclude the current agent session from agent searches
    #[arg(long = "exclude-current", alias = "other")]
    pub exclude_current: bool,

    /// WorkSet output format: full, timeline, compact, md, refs, or workset.
    /// Defaults to full when stdout is a terminal and workset when piped.
    #[arg(short = 'f', long, value_name = "FORMAT")]
    pub format: Option<WorkSetOutputFormat>,

    /// Alias for --format workset
    #[arg(long, conflicts_with = "format")]
    pub json: bool,

    /// Alias for --format refs
    #[arg(long, conflicts_with_all = ["format", "json"])]
    pub refs: bool,

    /// Save the result WorkSet as @name
    #[arg(long, value_name = "NAME")]
    pub save: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct EvalArgs {
    /// Evaluation depth for recall, precision, and NDCG
    #[arg(long, default_value_t = 5)]
    pub k: usize,

    /// Sort strategy to benchmark against golden queries
    #[arg(long, default_value_t = Sort::default(), value_name = "SORT")]
    pub sort: Sort,

    /// Frozen eval snapshot file (queries + corpus JSON)
    #[arg(long, value_name = "PATH")]
    pub snapshot: Option<PathBuf>,

    /// Dump current workspace records into a new snapshot (queries start empty)
    #[arg(long, value_name = "PATH", conflicts_with = "snapshot")]
    pub create_snapshot: Option<PathBuf>,

    /// Write qrels.txt and results.txt (trec_eval format) into this directory
    #[arg(long, value_name = "DIR")]
    pub export: Option<PathBuf>,

    /// Emit the report as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct FilterArgs {
    /// Source selector, WorkSet reference, or `@` for WorkSet JSON from stdin.
    #[arg(default_value = "@")]
    pub source: String,

    /// Select matching parts instead of preserving matching anchors
    #[arg(long)]
    pub parts: bool,

    /// Case-insensitive regex content filter
    #[arg(short = 'm', long = "match", value_name = "REGEX")]
    pub match_: Option<String>,

    /// Case-insensitive regex exclusion filter
    #[arg(short = 'v', long, value_name = "REGEX")]
    pub exclude: Option<String>,

    /// Field to match: content, title, session, input, output, command, or all
    #[arg(short = 'i', long = "in", default_value_t = Field::default(), value_name = "FIELD")]
    pub in_field: Field,

    /// Part kind filter: prompt, command, user, assistant, tool, tool_call, tool_result, skill, thinking, output, or error
    #[arg(long, value_name = "KIND")]
    pub kind: Option<PartKind>,

    /// Record status filter: success, failure, or unknown
    #[arg(long, value_name = "STATUS")]
    pub status: Option<WorkOutcome>,

    /// Exact terminal process exit code filter
    #[arg(long, value_name = "CODE")]
    pub exit_code: Option<i32>,

    /// Minimum command duration filter, e.g. 500ms, 2s, 1m
    #[arg(long, value_name = "DURATION")]
    pub min_duration: Option<String>,

    /// Maximum command duration filter, e.g. 500ms, 2s, 1m
    #[arg(long, value_name = "DURATION")]
    pub max_duration: Option<String>,

    /// Result sort: newest, oldest, duration, duration-asc, exit-code, or exit-code-asc
    #[arg(long, value_name = "SORT")]
    pub sort: Option<Sort>,

    /// Workspace directory used to resolve current AI sessions
    #[arg(long, value_name = "PATH")]
    pub cwd: Option<PathBuf>,

    /// Only search content at or after this time.
    #[arg(long, value_name = "TIME", help = TIME_FILTER_HELP)]
    pub since: Option<String>,

    /// Only search content at or before this time.
    #[arg(long, value_name = "TIME", help = TIME_FILTER_HELP)]
    pub until: Option<String>,

    /// Only search content within this recent duration, e.g. 30m, 2h, 7d.
    #[arg(long, value_name = "DURATION")]
    pub last: Option<String>,

    /// Return the latest N matching anchors before final sort
    #[arg(long, value_name = "N")]
    pub latest: Option<usize>,

    /// Maximum number of result anchors to print
    #[arg(short = 'l', long, value_name = "N")]
    pub limit: Option<usize>,

    /// Exclude the current agent session from agent searches
    #[arg(long = "exclude-current", alias = "other")]
    pub exclude_current: bool,

    /// WorkSet output format: full, timeline, compact, md, refs, or workset.
    /// Defaults to full when stdout is a terminal and workset when piped.
    #[arg(short = 'f', long, value_name = "FORMAT")]
    pub format: Option<WorkSetOutputFormat>,

    /// Alias for --format workset
    #[arg(long, conflicts_with = "format")]
    pub json: bool,

    /// Alias for --format refs
    #[arg(long, conflicts_with_all = ["format", "json"])]
    pub refs: bool,

    /// Save the result WorkSet as @name
    #[arg(long, value_name = "NAME")]
    pub save: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct VarCommand {
    #[command(subcommand)]
    pub action: VarSubcommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum VarSubcommand {
    /// Save a source WorkSet as @name
    Set(VarSetArgs),

    /// List saved vars
    List,

    /// Remove a saved var
    Rm(VarNameArgs),

    /// Remove all saved vars
    Cleanup,

    /// Merge sources into a saved var, deduplicating by anchor
    Merge(VarSourcesArgs),

    /// Drop source anchors from a saved var
    Drop(VarSourcesArgs),
}

#[derive(Args, Debug, Clone)]
pub struct VarSetArgs {
    /// Var name, referenced later as @name
    pub name: String,

    /// Source selector or `@` for WorkSet JSON from stdin
    pub source: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct VarNameArgs {
    /// Var name without @
    pub name: String,
}

#[derive(Args, Debug, Clone)]
pub struct VarSourcesArgs {
    /// Var name without @
    pub name: String,

    /// Source selectors to merge/drop
    #[arg(required = true)]
    pub sources: Vec<String>,
}

#[derive(Args, Debug, Clone)]
pub struct NavArgs {
    /// Source WorkSet reference or WorkRef.
    pub source: String,

    /// Motion expression: <, >N, +N, -N, [A..B], ~
    pub motion: String,

    /// Workspace directory used to resolve current AI sessions
    #[arg(long, value_name = "PATH")]
    pub cwd: Option<PathBuf>,

    /// WorkSet output format: full, timeline, compact, md, refs, or workset.
    /// Defaults to full when stdout is a terminal and workset when piped.
    #[arg(short = 'f', long, value_name = "FORMAT")]
    pub format: Option<WorkSetOutputFormat>,

    /// Alias for --format workset
    #[arg(long, conflicts_with = "format")]
    pub json: bool,

    /// Alias for --format refs
    #[arg(long, conflicts_with_all = ["format", "json"])]
    pub refs: bool,
}

#[derive(Args, Debug, Clone)]
pub struct ZoomArgs {
    /// Source WorkSet reference or WorkRef. Defaults to @last.
    #[arg(default_value = "@last")]
    pub source: String,

    /// Set both --before and --after.
    #[arg(short = 'C', long, value_name = "N")]
    pub context: Option<usize>,

    /// Records before each target record.
    #[arg(long, value_name = "N")]
    pub before: Option<usize>,

    /// Records after each target record.
    #[arg(long, value_name = "N")]
    pub after: Option<usize>,

    /// Workspace directory used to resolve current AI sessions
    #[arg(long, value_name = "PATH")]
    pub cwd: Option<PathBuf>,

    /// WorkSet output format: full, timeline, compact, md, refs, or workset.
    /// Defaults to full when stdout is a terminal and workset when piped.
    #[arg(short = 'f', long, value_name = "FORMAT")]
    pub format: Option<WorkSetOutputFormat>,

    /// Alias for --format workset
    #[arg(long, conflicts_with = "format")]
    pub json: bool,

    /// Alias for --format refs
    #[arg(long, conflicts_with_all = ["format", "json"])]
    pub refs: bool,

    /// Save the result WorkSet as @name
    #[arg(long, value_name = "NAME")]
    pub save: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct ShowArgs {
    /// Source ref or WorkSet reference to show, for example `pi/019e4f40/3`, `terminal/current/12/p2`, `@last`, or `@ctx[1,3]`.
    #[arg(value_name = "SOURCE")]
    pub source: String,

    /// Workspace directory used to resolve current AI sessions
    #[arg(long, value_name = "PATH")]
    pub cwd: Option<PathBuf>,

    /// WorkSet output format: full, timeline, compact, md, refs, or workset
    #[arg(short = 'f', long, value_name = "FORMAT")]
    pub format: Option<WorkSetOutputFormat>,

    /// Alias for --format full
    #[arg(long, conflicts_with_all = ["format", "refs", "json"])]
    pub full: bool,

    /// Alias for --format refs
    #[arg(long, conflicts_with_all = ["format", "full", "json"])]
    pub refs: bool,

    /// Alias for --format workset
    #[arg(long, conflicts_with = "format")]
    pub json: bool,
}

#[derive(Parser, Debug)]
pub struct WorkCommand {
    #[command(subcommand)]
    pub action: WorkSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum WorkSubcommand {
    /// List session markers for the current workspace
    #[command(after_help = WORK_SESSIONS_AFTER_HELP)]
    Sessions(WorkSessionsArgs),

    /// List record markers for one session marker
    #[command(after_help = WORK_RECORDS_AFTER_HELP)]
    Records(WorkRecordsArgs),

    /// List part markers for one record ref
    #[command(after_help = WORK_PARTS_AFTER_HELP)]
    Parts(WorkPartsArgs),
}

#[derive(Args, Debug, Clone)]
pub struct WorkSessionsArgs {
    /// Optional local or remote source selector, for example `desk:agent`.
    pub source: Option<String>,

    /// AI provider sessions to include; terminal workspace records are always included
    #[arg(long, default_value_t = HotkeyProviderSelection::default(), value_name = "PROVIDER")]
    pub provider: HotkeyProviderSelection,

    /// Workspace directory used to resolve current AI sessions
    #[arg(long, value_name = "PATH")]
    pub cwd: Option<PathBuf>,

    /// Print machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct WorkRecordsArgs {
    /// Source selector, WorkSet reference, or session marker.
    pub source: String,

    /// Workspace directory used to resolve current AI sessions
    #[arg(long, value_name = "PATH")]
    pub cwd: Option<PathBuf>,

    /// WorkSet output format: full, timeline, compact, md, refs, or workset.
    #[arg(short = 'f', long, value_name = "FORMAT")]
    pub format: Option<WorkSetOutputFormat>,

    /// Alias for --format workset
    #[arg(long, conflicts_with = "format")]
    pub json: bool,

    /// Alias for --format refs
    #[arg(long, conflicts_with_all = ["format", "json"])]
    pub refs: bool,

    /// Save the result WorkSet as @name
    #[arg(long, value_name = "NAME")]
    pub save: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct WorkPartsArgs {
    /// Source selector, WorkSet reference, or record ref.
    pub source: String,

    /// Part kind filter: prompt, command, user, assistant, tool, tool_call, tool_result, skill, thinking, output, or error
    #[arg(long, value_name = "KIND")]
    pub kind: Option<PartKind>,

    /// Case-insensitive regex part text filter
    #[arg(short = 'm', long = "match", value_name = "REGEX")]
    pub match_: Option<String>,

    /// Workspace directory used to resolve current AI sessions
    #[arg(long, value_name = "PATH")]
    pub cwd: Option<PathBuf>,

    /// WorkSet output format: full, timeline, compact, md, refs, or workset.
    #[arg(short = 'f', long, value_name = "FORMAT")]
    pub format: Option<WorkSetOutputFormat>,

    /// Alias for --format workset
    #[arg(long, conflicts_with = "format")]
    pub json: bool,

    /// Alias for --format refs
    #[arg(long, conflicts_with_all = ["format", "json"])]
    pub refs: bool,

    /// Save the result WorkSet as @name
    #[arg(long, value_name = "NAME")]
    pub save: Option<String>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct VersionArgs {
    /// Print binary path, build metadata, and local repo diagnostics
    #[arg(long)]
    pub verbose: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct ClearArgs {
    /// Clear all recorded session logs and state files
    #[arg(short = 'a', long = "all")]
    pub all: bool,
}

#[derive(Parser, Debug)]
pub struct HotkeyCommand {
    #[command(subcommand)]
    pub action: Option<HotkeyAction>,
}

#[derive(Subcommand, Debug)]
pub enum HotkeyAction {
    /// Start the global hotkey daemon
    Start(HotkeyStartArgs),

    /// Stop the global hotkey daemon
    Stop,

    /// Show daemon status
    Status,
}

#[derive(Args, Debug, Clone)]
pub struct HotkeyStartArgs {
    /// Override the configured hotkey chord, for example `alt+y`
    #[arg(long, value_name = "CHORD")]
    pub chord: Option<String>,

    /// AI provider opened by the hotkey
    #[arg(long, default_value_t = HotkeyProviderSelection::default(), value_name = "PROVIDER")]
    pub provider: HotkeyProviderSelection,
}

#[derive(Args, Debug, Clone)]
pub struct HotkeyServeArgs {
    /// Absolute working directory used when the picker terminal opens
    #[arg(long, value_name = "PATH")]
    pub cwd: String,

    /// Registered global hotkey chord, for example `alt+y`
    #[arg(long, value_name = "CHORD")]
    pub chord: String,

    /// AI provider opened by the hotkey
    #[arg(long, default_value_t = HotkeyProviderSelection::default(), value_name = "PROVIDER")]
    pub provider: HotkeyProviderSelection,
}

#[derive(Args, Debug, Clone)]
pub struct HotkeyPickAgentArgs {
    /// Working directory used for launching the picker process
    #[arg(long, value_name = "PATH")]
    pub cwd: PathBuf,

    /// AI provider sessions to show
    #[arg(long, default_value_t = HotkeyProviderSelection::default(), value_name = "PROVIDER")]
    pub provider: HotkeyProviderSelection,

    /// Restrict the picker to sessions whose cwd matches `--cwd`
    #[arg(long, default_value_t = false)]
    pub current_session: bool,

    /// Select mounted remotes in the picker
    #[arg(long = "all", default_value_t = false)]
    pub all: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HotkeyProviderSelection(Option<AgentProvider>);

impl HotkeyProviderSelection {
    pub fn provider(provider: AgentProvider) -> Self {
        Self(Some(provider))
    }

    pub fn providers(self) -> Vec<AgentProvider> {
        match self.0 {
            Some(provider) => vec![provider],
            None => AgentProvider::all()
                .iter()
                .map(|spec| spec.provider)
                .collect(),
        }
    }

    pub fn as_str(self) -> &'static str {
        self.0.map(AgentProvider::command_name).unwrap_or("all")
    }

    pub fn label(self) -> &'static str {
        self.0
            .map(AgentProvider::name)
            .unwrap_or("all AI providers")
    }
}

impl FromStr for HotkeyProviderSelection {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.eq_ignore_ascii_case("all") {
            return Ok(Self::default());
        }

        AgentProvider::from_command_name(value)
            .map(Self::provider)
            .ok_or_else(|| format!("unknown AI provider `{value}`"))
    }
}

impl std::fmt::Display for HotkeyProviderSelection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for HotkeyProviderSelection {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for HotkeyProviderSelection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Args, Debug)]
pub struct SyncArgs {
    /// Re-parse every session, ignoring cached fingerprints
    #[arg(long, default_value_t = false)]
    pub full: bool,

    /// Output structured JSON for agent/skill consumption
    #[arg(long)]
    pub json: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn mcp_serve_parses() {
        let cli = Cli::try_parse_from(["sivtr", "mcp", "serve"]).unwrap();
        match cli.command {
            Some(Commands::Mcp(cmd)) => assert!(matches!(cmd.action, McpAction::Serve(_))),
            _ => panic!("expected mcp serve"),
        }
    }

    #[test]
    fn mcp_serve_parses_idle_exit() {
        let cli = Cli::try_parse_from(["sivtr", "mcp", "serve", "--idle-exit", "60"]).unwrap();
        match cli.command {
            Some(Commands::Mcp(cmd)) => match cmd.action {
                McpAction::Serve(args) => assert_eq!(args.idle_exit, Some(60)),
                _ => panic!("expected mcp serve"),
            },
            _ => panic!("expected mcp serve"),
        }
    }

    #[test]
    fn mcp_install_parses_targets() {
        let cli = Cli::try_parse_from([
            "sivtr",
            "mcp",
            "install",
            "-p",
            "claude,cursor",
            "-l",
            "global",
            "-y",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Mcp(cmd)) => match cmd.action {
                McpAction::Install(args) => {
                    assert_eq!(
                        args.providers,
                        vec!["claude".to_string(), "cursor".to_string()]
                    );
                    assert_eq!(args.location, McpLocation::Global);
                    assert!(args.yes);
                }
                _ => panic!("expected mcp install"),
            },
            _ => panic!("expected mcp command"),
        }
    }

    #[test]
    fn mcp_print_config_parses() {
        let cli = Cli::try_parse_from(["sivtr", "mcp", "print-config", "claude"]).unwrap();
        match cli.command {
            Some(Commands::Mcp(cmd)) => match cmd.action {
                McpAction::PrintConfig { target } => assert_eq!(target, "claude"),
                _ => panic!("expected print-config"),
            },
            _ => panic!("expected mcp command"),
        }
    }

    #[test]
    fn global_color_flag_accepts_never() {
        let cli = Cli::try_parse_from(["sivtr", "--color", "never", "version"]).unwrap();

        assert_eq!(cli.color, ColorArg::Never);
    }

    #[test]
    fn copy_input_accepts_prompt_override() {
        let cli = Cli::try_parse_from(["sivtr", "ci", "--prompt", ":"]).unwrap();

        match cli.command {
            Some(Commands::Ci(inv)) => assert_eq!(inv.flags.prompt.as_deref(), Some(":")),
            _ => panic!("expected ci command"),
        }
    }

    #[test]
    fn copy_out_accepts_shared_flags() {
        // Projection sugar shares CopyFlagArgs; --prompt is ignored for output projection.
        let cli = Cli::try_parse_from(["sivtr", "co", "--print", "--ansi"]).unwrap();
        match cli.command {
            Some(Commands::Co(inv)) => {
                assert!(inv.flags.print);
                assert!(inv.flags.ansi);
            }
            _ => panic!("expected co command"),
        }
    }

    #[test]
    fn copy_aliases_accept_ansi_flag() {
        let cli = Cli::try_parse_from(["sivtr", "co", "--ansi"]).unwrap();

        match cli.command {
            Some(Commands::Co(inv)) => assert!(inv.flags.ansi),
            _ => panic!("expected co command"),
        }
    }

    #[test]
    fn version_accepts_verbose() {
        let cli = Cli::try_parse_from(["sivtr", "version", "--verbose"]).unwrap();

        match cli.command {
            Some(Commands::Version(args)) => assert!(args.verbose),
            _ => panic!("expected version command"),
        }
    }

    #[test]
    fn clear_accepts_all_flag() {
        let cli = Cli::try_parse_from(["sivtr", "clear", "--all"]).unwrap();

        match cli.command {
            Some(Commands::Clear(args)) => assert!(args.all),
            _ => panic!("expected clear command"),
        }
    }

    #[test]
    fn serve_parses_lifecycle_action() {
        let cli = Cli::try_parse_from(["sivtr", "serve", "start"]).unwrap();

        match cli.command {
            Some(Commands::Serve(command)) => {
                assert!(matches!(command.action, ServeAction::Start));
            }
            _ => panic!("expected serve command"),
        }
    }

    #[test]
    fn diff_parses_two_selectors() {
        let cli = Cli::try_parse_from(["sivtr", "diff", "1", "2"]).unwrap();

        match cli.command {
            Some(Commands::Diff(args)) => {
                assert_eq!(args.left, "1");
                assert_eq!(args.right, "2");
                assert!(!args.side_by_side);
                assert!(!args.output);
                assert!(!args.block);
                assert!(!args.input);
                assert!(!args.cmd);
            }
            _ => panic!("expected diff command"),
        }
    }

    #[test]
    fn diff_parses_block_mode_and_side_by_side() {
        let cli =
            Cli::try_parse_from(["sivtr", "diff", "3", "1", "--block", "--side-by-side"]).unwrap();

        match cli.command {
            Some(Commands::Diff(args)) => {
                assert_eq!(args.left, "3");
                assert_eq!(args.right, "1");
                assert!(args.block);
                assert!(args.side_by_side);
            }
            _ => panic!("expected diff command"),
        }
    }

    #[test]
    fn diff_rejects_multiple_content_modes() {
        let result = Cli::try_parse_from(["sivtr", "diff", "1", "2", "--output", "--cmd"]);
        assert!(result.is_err());
    }

    #[test]
    fn copy_work_ref_is_free_address_token() {
        let cli = Cli::try_parse_from([
            "sivtr",
            "copy",
            "codex/session/3/p1",
            "--print",
            "--regex",
            "error",
        ])
        .unwrap();
        let tokens = copy_external_tokens(cli);
        assert_eq!(
            tokens,
            vec![
                "codex/session/3/p1".to_string(),
                "--print".to_string(),
                "--regex".to_string(),
                "error".to_string(),
            ]
        );
    }

    fn copy_external_tokens(cli: Cli) -> Vec<String> {
        match cli.command {
            Some(Commands::Copy(cmd)) => match cmd.mode {
                Some(CopySubcommand::External(tokens)) => tokens,
                other => panic!("expected copy external subcommand, got {other:?}"),
            },
            other => panic!("expected copy command, got {other:?}"),
        }
    }

    #[test]
    fn copy_provider_name_is_free_address_token() {
        for name in AgentProvider::command_names() {
            let cli = Cli::try_parse_from(["sivtr", "copy", name]).unwrap();
            let tokens = copy_external_tokens(cli);
            assert_eq!(tokens, vec![name.to_string()]);
        }
    }

    #[test]
    fn copy_address_and_dialogues_as_external_tokens() {
        let cli = Cli::try_parse_from(["sivtr", "copy", "codex", "2..4", "--print"]).unwrap();
        let tokens = copy_external_tokens(cli);
        assert_eq!(
            tokens,
            vec![
                "codex".to_string(),
                "2..4".to_string(),
                "--print".to_string()
            ]
        );
    }

    #[test]
    fn copy_dialogues_only_is_external_token() {
        let cli = Cli::try_parse_from(["sivtr", "copy", "3", "--print"]).unwrap();
        let tokens = copy_external_tokens(cli);
        assert_eq!(tokens, vec!["3".to_string(), "--print".to_string()]);
    }

    #[test]
    fn copy_out_accepts_address_and_dialogues() {
        let cli = Cli::try_parse_from(["sivtr", "copy", "out", "codex", "2"]).unwrap();
        match cli.command {
            Some(Commands::Copy(cmd)) => match cmd.mode {
                Some(CopySubcommand::Out(inv)) => {
                    assert_eq!(inv.tokens, vec!["codex".to_string(), "2".to_string()]);
                }
                other => panic!("expected Out, got {other:?}"),
            },
            other => panic!("expected copy, got {other:?}"),
        }
    }

    #[test]
    fn registry_provider_names_parse_for_hotkey_selection() {
        for name in AgentProvider::command_names() {
            assert_eq!(
                name.parse::<HotkeyProviderSelection>().unwrap(),
                HotkeyProviderSelection::provider(
                    AgentProvider::from_command_name(name).expect("registry name")
                )
            );
        }
    }

    #[test]
    fn search_accepts_target_filters_and_format() {
        let cli = Cli::try_parse_from([
            "sivtr",
            "search",
            "pi/019e5941",
            "--match",
            "workspace picker",
            "--in",
            "title",
            "--kind",
            "assistant",
            "--status",
            "unknown",
            "--format",
            "timeline",
            "--limit",
            "5",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::Search(args)) => {
                assert_eq!(args.source, "pi/019e5941");
                assert_eq!(args.match_.as_deref(), Some("workspace picker"));
                assert_eq!(args.exclude.as_deref(), None);
                assert_eq!(args.in_field, Field::Title);
                assert_eq!(args.kind, Some(PartKind::Assistant));
                assert_eq!(args.status, Some(WorkOutcome::Unknown));
                assert_eq!(args.exit_code, None);
                assert_eq!(args.min_duration, None);
                assert_eq!(args.max_duration, None);
                assert_eq!(args.sort, None);
                assert_eq!(args.format, Some(WorkSetOutputFormat::Timeline));
                assert_eq!(args.limit, Some(5));
                assert_eq!(args.since, None);
                assert_eq!(args.until, None);
                assert_eq!(args.last, None);
                assert_eq!(args.latest, None);
                assert!(!args.exclude_current);
                assert!(!args.json);
                assert!(!args.refs);
            }
            _ => panic!("expected search command"),
        }
    }

    #[test]
    fn zoom_accepts_context_and_output_options() {
        let cli = Cli::try_parse_from([
            "sivtr",
            "zoom",
            "@panics[1..3,5]",
            "-C",
            "5",
            "--after",
            "1",
            "--save",
            "ctx",
            "-f",
            "timeline",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::Zoom(args)) => {
                assert_eq!(args.source, "@panics[1..3,5]");
                assert_eq!(args.context, Some(5));
                assert_eq!(args.before, None);
                assert_eq!(args.after, Some(1));
                assert_eq!(args.save.as_deref(), Some("ctx"));
                assert_eq!(args.format, Some(WorkSetOutputFormat::Timeline));
            }
            _ => panic!("expected zoom command"),
        }
    }

    #[test]
    fn zoom_defaults_to_last_workset() {
        let cli = Cli::try_parse_from(["sivtr", "zoom"]).unwrap();

        match cli.command {
            Some(Commands::Zoom(args)) => assert_eq!(args.source, "@last"),
            _ => panic!("expected zoom command"),
        }
    }

    #[test]
    fn zoom_rejects_refs_alias_with_format_or_json() {
        assert!(
            Cli::try_parse_from(["sivtr", "zoom", "@last", "--refs", "--format", "timeline",])
                .is_err()
        );
        assert!(Cli::try_parse_from(["sivtr", "zoom", "@last", "--refs", "--json"]).is_err());
    }

    #[test]
    fn show_accepts_workset_output_options() {
        let cli = Cli::try_parse_from(["sivtr", "show", "@ctx[1,3]", "-f", "md"]).unwrap();

        match cli.command {
            Some(Commands::Show(args)) => {
                assert_eq!(args.source, "@ctx[1,3]");
                assert_eq!(args.format, Some(WorkSetOutputFormat::Md));
                assert!(!args.json);
                assert!(!args.refs);
                assert!(!args.full);
            }
            _ => panic!("expected show command"),
        }
    }

    #[test]
    fn show_accepts_refs_alias_for_worksets() {
        let cli = Cli::try_parse_from(["sivtr", "show", "@last", "--refs"]).unwrap();

        match cli.command {
            Some(Commands::Show(args)) => {
                assert_eq!(args.source, "@last");
                assert!(args.refs);
            }
            _ => panic!("expected show command"),
        }
    }

    #[test]
    fn show_accepts_full_for_any_target() {
        let cli = Cli::try_parse_from(["sivtr", "show", "pi/019e6c57/17", "--full"]).unwrap();

        match cli.command {
            Some(Commands::Show(args)) => {
                assert_eq!(args.source, "pi/019e6c57/17");
                assert!(args.full);
            }
            _ => panic!("expected show command"),
        }
    }

    #[test]
    fn show_accepts_full_format_for_any_target() {
        let cli = Cli::try_parse_from(["sivtr", "show", "pi/019e6c57/17", "-f", "full"]).unwrap();

        match cli.command {
            Some(Commands::Show(args)) => {
                assert_eq!(args.source, "pi/019e6c57/17");
                assert_eq!(args.format, Some(WorkSetOutputFormat::Full));
            }
            _ => panic!("expected show command"),
        }
    }

    #[test]
    fn show_rejects_alias_conflicts() {
        assert!(Cli::try_parse_from(["sivtr", "show", "@last", "--full", "--refs"]).is_err());
        assert!(Cli::try_parse_from(["sivtr", "show", "@last", "--full", "--json"]).is_err());
        assert!(
            Cli::try_parse_from(["sivtr", "show", "@last", "--full", "--format", "timeline",])
                .is_err()
        );
    }

    #[test]
    fn show_rejects_refs_alias_with_format_or_json() {
        assert!(
            Cli::try_parse_from(["sivtr", "show", "@last", "--refs", "--format", "timeline",])
                .is_err()
        );
        assert!(Cli::try_parse_from(["sivtr", "show", "@last", "--refs", "--json"]).is_err());
    }

    #[test]
    fn publish_parses_preview_and_default_publish_flags() {
        let cli = Cli::try_parse_from([
            "sivtr",
            "publish",
            "preview",
            "@share_ready",
            "--expires",
            "30d",
            "--format",
            "json",
        ])
        .expect("publish preview with a saved WorkSet should parse");
        match cli.command {
            Some(Commands::Publish(command)) => match command.action {
                Some(PublishAction::Preview(args)) => {
                    assert_eq!(args.source.as_deref(), Some("@share_ready"));
                    assert_eq!(args.expires, "30d");
                    assert_eq!(args.format, PublishFormat::Json);
                }
                _ => panic!("expected publish preview"),
            },
            _ => panic!("expected publish command"),
        }

        let cli = Cli::try_parse_from(["sivtr", "publish", "preview"])
            .expect("publish preview without a source should parse");
        match cli.command {
            Some(Commands::Publish(command)) => match command.action {
                Some(PublishAction::Preview(args)) => {
                    assert!(args.source.is_none());
                    assert_eq!(args.expires, "7d");
                }
                _ => panic!("expected publish preview"),
            },
            _ => panic!("expected publish command"),
        }

        let cli = Cli::try_parse_from(["sivtr", "publish", "@", "--yes", "--allow-warnings"])
            .expect("default publish flags should parse");
        match cli.command {
            Some(Commands::Publish(command)) => {
                assert!(command.action.is_none());
                assert_eq!(command.source.as_deref(), Some("@"));
                assert!(command.yes);
                assert!(command.allow_warnings);
            }
            _ => panic!("expected publish command"),
        }

        let cli = Cli::try_parse_from(["sivtr", "publish", "review"])
            .expect("a saved WorkSet name should parse as a publish source");
        match cli.command {
            Some(Commands::Publish(command)) => {
                assert!(command.action.is_none());
                assert_eq!(command.source.as_deref(), Some("review"));
            }
            _ => panic!("expected publish command"),
        }

        let cli = Cli::try_parse_from(["sivtr", "publish", "link"])
            .expect("publish link without an id should parse");
        match cli.command {
            Some(Commands::Publish(command)) => match command.action {
                Some(PublishAction::Link(args)) => assert!(args.publication_id.is_none()),
                _ => panic!("expected publish link"),
            },
            _ => panic!("expected publish command"),
        }
    }

    #[test]
    fn work_sessions_accepts_provider_and_json() {
        let cli =
            Cli::try_parse_from(["sivtr", "work", "sessions", "--provider", "codex", "--json"])
                .unwrap();

        match cli.command {
            Some(Commands::Work(cmd)) => match cmd.action {
                WorkSubcommand::Sessions(args) => {
                    assert_eq!(args.source, None);
                    assert_eq!(
                        args.provider,
                        HotkeyProviderSelection::provider(AgentProvider::Codex)
                    );
                    assert!(args.json);
                }
                _ => panic!("expected work sessions command"),
            },
            _ => panic!("expected work command"),
        }
    }

    #[test]
    fn work_sessions_accepts_remote_source() {
        let cli = Cli::try_parse_from(["sivtr", "work", "sessions", "desk:agent"]).unwrap();

        match cli.command {
            Some(Commands::Work(cmd)) => match cmd.action {
                WorkSubcommand::Sessions(args) => {
                    assert_eq!(args.source.as_deref(), Some("desk:agent"));
                }
                _ => panic!("expected work sessions command"),
            },
            _ => panic!("expected work command"),
        }
    }

    #[test]
    fn work_records_accepts_session_marker() {
        let cli = Cli::try_parse_from(["sivtr", "work", "records", "codex/019df7fb"]).unwrap();

        match cli.command {
            Some(Commands::Work(cmd)) => match cmd.action {
                WorkSubcommand::Records(args) => {
                    assert_eq!(args.source, "codex/019df7fb");
                    assert!(!args.json);
                    assert!(!args.refs);
                }
                _ => panic!("expected work records command"),
            },
            _ => panic!("expected work command"),
        }
    }

    #[test]
    fn work_parts_accepts_kind_filter() {
        let cli = Cli::try_parse_from([
            "sivtr",
            "work",
            "parts",
            "codex/019df7fb/3",
            "--kind",
            "assistant",
            "--json",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::Work(cmd)) => match cmd.action {
                WorkSubcommand::Parts(args) => {
                    assert_eq!(args.source, "codex/019df7fb/3");
                    assert_eq!(args.kind, Some(PartKind::Assistant));
                    assert!(args.json);
                }
                _ => panic!("expected work parts command"),
            },
            _ => panic!("expected work command"),
        }
    }

    #[test]
    fn search_accepts_time_and_latest_filters() {
        let cli = Cli::try_parse_from([
            "sivtr",
            "search",
            "terminal",
            "--last",
            "2h",
            "--since",
            "2026-05-23T00:00:00Z",
            "--until",
            "2026-05-24",
            "--latest",
            "1",
            "--exit-code",
            "101",
            "--min-duration",
            "500ms",
            "--max-duration",
            "2s",
            "--sort",
            "duration",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::Search(args)) => {
                assert_eq!(args.last.as_deref(), Some("2h"));
                assert_eq!(args.since.as_deref(), Some("2026-05-23T00:00:00Z"));
                assert_eq!(args.until.as_deref(), Some("2026-05-24"));
                assert_eq!(args.latest, Some(1));
                assert_eq!(args.exit_code, Some(101));
                assert_eq!(args.min_duration.as_deref(), Some("500ms"));
                assert_eq!(args.max_duration.as_deref(), Some("2s"));
                assert_eq!(args.sort, Some(Sort::Duration));
            }
            _ => panic!("expected search command"),
        }
    }

    #[test]
    fn search_accepts_positional_query() {
        let cli =
            Cli::try_parse_from(["sivtr", "search", "terminal", "docker pull failed"]).unwrap();
        match cli.command {
            Some(Commands::Search(args)) => {
                assert_eq!(args.source, "terminal");
                assert_eq!(args.query.as_deref(), Some("docker pull failed"));
                assert_eq!(args.match_, None);
            }
            _ => panic!("expected search command"),
        }
    }

    #[test]
    fn search_accepts_query_with_match_refinement() {
        let cli = Cli::try_parse_from(["sivtr", "search", "terminal", "docker", "-m", "error.*E0"])
            .unwrap();
        match cli.command {
            Some(Commands::Search(args)) => {
                assert_eq!(args.source, "terminal");
                assert_eq!(args.query.as_deref(), Some("docker"));
                assert_eq!(args.match_.as_deref(), Some("error.*E0"));
            }
            _ => panic!("expected search command"),
        }
    }

    #[test]
    fn search_rejects_old_filter_flags() {
        assert!(Cli::try_parse_from(["sivtr", "search", "needle", "--shell"]).is_err());
        assert!(Cli::try_parse_from(["sivtr", "search", "needle", "--agent"]).is_err());
        assert!(Cli::try_parse_from(["sivtr", "search", "needle", "--provider", "pi"]).is_err());
        assert!(Cli::try_parse_from(["sivtr", "search", "needle", "--scope", "content"]).is_err());
        assert!(Cli::try_parse_from(["sivtr", "search", "needle", "--recent", "2h"]).is_err());
    }

    #[test]
    fn search_rejects_unknown_field() {
        let result = Cli::try_parse_from(["sivtr", "search", "needle", "--in", "unknown"]);

        assert!(result.is_err());
    }

    #[test]
    fn eval_parses_defaults() {
        let cli = Cli::try_parse_from(["sivtr", "eval"]).unwrap();
        match cli.command {
            Some(Commands::Eval(args)) => {
                assert_eq!(args.k, 5);
                assert_eq!(args.sort, Sort::Newest);
                assert!(args.snapshot.is_none());
                assert!(args.create_snapshot.is_none());
                assert!(args.export.is_none());
                assert!(!args.json);
            }
            _ => panic!("expected eval command"),
        }
    }

    #[test]
    fn eval_parses_flags() {
        let cli = Cli::try_parse_from([
            "sivtr",
            "eval",
            "--k",
            "10",
            "--sort",
            "duration",
            "--snapshot",
            "snap.json",
            "--export",
            "out",
            "--json",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Eval(args)) => {
                assert_eq!(args.k, 10);
                assert_eq!(args.sort, Sort::Duration);
                assert_eq!(
                    args.snapshot.as_deref(),
                    Some(std::path::Path::new("snap.json"))
                );
                assert!(args.create_snapshot.is_none());
                assert_eq!(args.export.as_deref(), Some(std::path::Path::new("out")));
                assert!(args.json);
            }
            _ => panic!("expected eval command"),
        }
    }

    #[test]
    fn eval_parses_create_snapshot() {
        let cli = Cli::try_parse_from(["sivtr", "eval", "--create-snapshot", "snap.json"]).unwrap();
        match cli.command {
            Some(Commands::Eval(args)) => {
                assert_eq!(
                    args.create_snapshot.as_deref(),
                    Some(std::path::Path::new("snap.json"))
                );
                assert!(args.snapshot.is_none());
            }
            _ => panic!("expected eval command"),
        }
    }

    #[test]
    fn eval_rejects_snapshot_and_create_together() {
        let result = Cli::try_parse_from([
            "sivtr",
            "eval",
            "--snapshot",
            "a.json",
            "--create-snapshot",
            "b.json",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn eval_rejects_unknown_sort() {
        let result = Cli::try_parse_from(["sivtr", "eval", "--sort", "unknown"]);
        assert!(result.is_err());
    }

    #[test]
    fn search_rejects_unknown_sort() {
        let result = Cli::try_parse_from(["sivtr", "search", "terminal", "--sort", "unknown"]);

        assert!(result.is_err());
    }

    #[test]
    fn search_target_accepts_line_segment() {
        let cli = Cli::try_parse_from([
            "sivtr",
            "search",
            "terminal/session_1/3/2",
            "--format",
            "json",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::Search(args)) => assert_eq!(args.source, "terminal/session_1/3/2"),
            _ => panic!("expected search command"),
        }
    }

    #[test]
    fn search_accepts_json_alias() {
        let cli = Cli::try_parse_from(["sivtr", "search", "agent", "--json"]).unwrap();

        match cli.command {
            Some(Commands::Search(args)) => {
                assert!(args.json);
                assert_eq!(args.format, None);
            }
            _ => panic!("expected search command"),
        }
    }

    #[test]
    fn search_rejects_json_alias_with_format() {
        assert!(Cli::try_parse_from([
            "sivtr", "search", "agent", "--json", "--format", "timeline",
        ])
        .is_err());
    }

    #[test]
    fn search_accepts_refs_alias() {
        let cli = Cli::try_parse_from(["sivtr", "search", "agent", "--refs"]).unwrap();

        match cli.command {
            Some(Commands::Search(args)) => {
                assert!(args.refs);
                assert_eq!(args.format, None);
            }
            _ => panic!("expected search command"),
        }
    }

    #[test]
    fn search_rejects_refs_alias_with_format_or_json() {
        assert!(Cli::try_parse_from([
            "sivtr", "search", "agent", "--refs", "--format", "timeline",
        ])
        .is_err());
        assert!(Cli::try_parse_from(["sivtr", "search", "agent", "--refs", "--json"]).is_err());
    }

    #[test]
    fn search_accepts_exclude_current_aliases() {
        let cli = Cli::try_parse_from(["sivtr", "search", "agent", "--exclude-current"]).unwrap();

        match cli.command {
            Some(Commands::Search(args)) => assert!(args.exclude_current),
            _ => panic!("expected search command"),
        }

        let cli = Cli::try_parse_from(["sivtr", "search", "agent", "--other"]).unwrap();

        match cli.command {
            Some(Commands::Search(args)) => assert!(args.exclude_current),
            _ => panic!("expected search command"),
        }
    }

    #[test]
    fn search_accepts_exclude_filter() {
        let cli = Cli::try_parse_from([
            "sivtr",
            "search",
            "agent",
            "--match",
            "TODO|pending",
            "--exclude",
            "example|示例",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::Search(args)) => {
                assert_eq!(args.match_.as_deref(), Some("TODO|pending"));
                assert_eq!(args.exclude.as_deref(), Some("example|示例"));
            }
            _ => panic!("expected search command"),
        }
    }

    #[test]
    fn search_accepts_short_aliases() {
        let cli = Cli::try_parse_from([
            "sivtr",
            "s",
            "agent",
            "-m",
            "TODO|pending",
            "-v",
            "example|示例",
            "-i",
            "title",
            "-f",
            "timeline",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::Search(args)) => {
                assert_eq!(args.match_.as_deref(), Some("TODO|pending"));
                assert_eq!(args.exclude.as_deref(), Some("example|示例"));
                assert_eq!(args.in_field, Field::Title);
                assert_eq!(args.format, Some(WorkSetOutputFormat::Timeline));
            }
            _ => panic!("expected search command"),
        }
    }

    #[test]
    fn nav_accepts_motion_and_output_flags() {
        let cli = Cli::try_parse_from(["sivtr", "nav", "@hit", "<+1>2", "--refs"]).unwrap();
        match cli.command {
            Some(Commands::Nav(args)) => {
                assert_eq!(args.source, "@hit");
                assert_eq!(args.motion, "<+1>2");
                assert!(args.refs);
            }
            _ => panic!("expected nav command"),
        }
    }

    #[test]
    fn var_accepts_set_rm_cleanup_merge_drop() {
        let cli = Cli::try_parse_from(["sivtr", "var", "set", "ctx", "@last"]).unwrap();
        match cli.command {
            Some(Commands::Var(cmd)) => match cmd.action {
                VarSubcommand::Set(args) => {
                    assert_eq!(args.name, "ctx");
                    assert_eq!(args.source.as_deref(), Some("@last"));
                }
                _ => panic!("expected var set"),
            },
            _ => panic!("expected var command"),
        }

        let cli = Cli::try_parse_from(["sivtr", "var", "list"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Var(VarCommand {
                action: VarSubcommand::List
            }))
        ));

        let cli = Cli::try_parse_from(["sivtr", "var", "rm", "ctx"]).unwrap();
        match cli.command {
            Some(Commands::Var(cmd)) => match cmd.action {
                VarSubcommand::Rm(args) => assert_eq!(args.name, "ctx"),
                _ => panic!("expected var rm"),
            },
            _ => panic!("expected var command"),
        }

        assert!(matches!(
            Cli::try_parse_from(["sivtr", "var", "cleanup"])
                .unwrap()
                .command,
            Some(Commands::Var(VarCommand {
                action: VarSubcommand::Cleanup
            }))
        ));

        let cli = Cli::try_parse_from(["sivtr", "var", "merge", "ctx", "@a", "@b"]).unwrap();
        match cli.command {
            Some(Commands::Var(cmd)) => match cmd.action {
                VarSubcommand::Merge(args) => {
                    assert_eq!(args.name, "ctx");
                    assert_eq!(args.sources, vec!["@a", "@b"]);
                }
                _ => panic!("expected var merge"),
            },
            _ => panic!("expected var command"),
        }

        let cli = Cli::try_parse_from(["sivtr", "var", "drop", "ctx", "@noise"]).unwrap();
        match cli.command {
            Some(Commands::Var(cmd)) => match cmd.action {
                VarSubcommand::Drop(args) => {
                    assert_eq!(args.name, "ctx");
                    assert_eq!(args.sources, vec!["@noise"]);
                }
                _ => panic!("expected var drop"),
            },
            _ => panic!("expected var command"),
        }
    }

    #[test]
    fn var_merge_and_drop_require_sources() {
        assert!(Cli::try_parse_from(["sivtr", "var", "merge", "ctx"]).is_err());
        assert!(Cli::try_parse_from(["sivtr", "var", "drop", "ctx"]).is_err());
    }

    #[test]
    fn hotkey_start_accepts_chord_override() {
        let cli = Cli::try_parse_from(["sivtr", "hotkey", "start", "--chord", "alt+y"]).unwrap();

        match cli.command {
            Some(Commands::Hotkey(cmd)) => match cmd.action {
                Some(HotkeyAction::Start(args)) => {
                    assert_eq!(args.chord.as_deref(), Some("alt+y"));
                    assert_eq!(args.provider, HotkeyProviderSelection::default());
                }
                _ => panic!("expected hotkey start"),
            },
            _ => panic!("expected hotkey command"),
        }
    }

    #[test]
    fn hotkey_start_accepts_provider_override() {
        let cli =
            Cli::try_parse_from(["sivtr", "hotkey", "start", "--provider", "claude"]).unwrap();

        match cli.command {
            Some(Commands::Hotkey(cmd)) => match cmd.action {
                Some(HotkeyAction::Start(args)) => {
                    assert_eq!(
                        args.provider,
                        HotkeyProviderSelection::provider(AgentProvider::Claude)
                    );
                }
                _ => panic!("expected hotkey start"),
            },
            _ => panic!("expected hotkey command"),
        }
    }

    #[test]
    fn hotkey_pick_agent_defaults_to_all() {
        let cli = Cli::try_parse_from(["sivtr", "hotkey-pick-agent", "--cwd", "."]).unwrap();

        match cli.command {
            Some(Commands::HotkeyPickAgent(args)) => {
                assert_eq!(args.cwd, PathBuf::from("."));
                assert_eq!(args.provider, HotkeyProviderSelection::default());
                assert!(!args.current_session);
                assert!(!args.all);
            }
            _ => panic!("expected hotkey-pick-agent command"),
        }
    }

    #[test]
    fn global_all_flag_defaults_off_and_accepts_all() {
        let default = Cli::try_parse_from(["sivtr"]).unwrap();
        assert!(!default.all);

        let with_all = Cli::try_parse_from(["sivtr", "--all"]).unwrap();
        assert!(with_all.all);
        assert!(with_all.command.is_none());
    }

    #[test]
    fn hotkey_pick_agent_accepts_all_flag() {
        let cli =
            Cli::try_parse_from(["sivtr", "hotkey-pick-agent", "--cwd", ".", "--all"]).unwrap();

        match cli.command {
            Some(Commands::HotkeyPickAgent(args)) => {
                assert!(args.all);
                assert!(!args.current_session);
            }
            _ => panic!("expected hotkey-pick-agent command"),
        }
    }

    #[test]
    fn hotkey_pick_agent_accepts_current_session_flag() {
        let cli = Cli::try_parse_from([
            "sivtr",
            "hotkey-pick-agent",
            "--cwd",
            ".",
            "--current-session",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::HotkeyPickAgent(args)) => {
                assert_eq!(args.cwd, PathBuf::from("."));
                assert!(args.current_session);
            }
            _ => panic!("expected hotkey-pick-agent command"),
        }
    }

    #[test]
    fn sync_accepts_full_and_json_flags() {
        let cli = Cli::try_parse_from(["sivtr", "sync", "--full", "--json"]).unwrap();

        match cli.command {
            Some(Commands::Sync(args)) => {
                assert!(args.full);
                assert!(args.json);
            }
            _ => panic!("expected sync command"),
        }
    }

    #[test]
    fn hotkey_provider_rejects_unknown_provider() {
        let result = Cli::try_parse_from(["sivtr", "hotkey", "start", "--provider", "unknown"]);

        assert!(result.is_err());
    }

    #[test]
    fn run_accepts_hyphen_prefixed_child_args_without_separator() {
        let cli = Cli::try_parse_from(["sivtr", "run", "bash", "-lc", "printf ok"]).unwrap();

        match cli.command {
            Some(Commands::Run { command, args }) => {
                assert_eq!(command, "bash");
                assert_eq!(args, vec!["-lc".to_string(), "printf ok".to_string()]);
            }
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn init_accepts_tmux_target() {
        let cli = Cli::try_parse_from(["sivtr", "init", "tmux"]).unwrap();

        match cli.command {
            Some(Commands::Init { target }) => assert_eq!(target, "tmux"),
            _ => panic!("expected init command"),
        }
    }

    #[test]
    fn init_accepts_linux_shortcut_target() {
        let cli = Cli::try_parse_from(["sivtr", "init", "linux-shortcut"]).unwrap();

        match cli.command {
            Some(Commands::Init { target }) => assert_eq!(target, "linux-shortcut"),
            _ => panic!("expected init command"),
        }
    }

    #[test]
    fn init_accepts_macos_shortcut_target() {
        let cli = Cli::try_parse_from(["sivtr", "init", "macos-shortcut"]).unwrap();

        match cli.command {
            Some(Commands::Init { target }) => assert_eq!(target, "macos-shortcut"),
            _ => panic!("expected init command"),
        }
    }

    #[test]
    fn init_help_mentions_macos_shortcut_target() {
        let mut cmd = Cli::command();
        let init = cmd
            .find_subcommand_mut("init")
            .expect("init subcommand should exist");
        let mut help = Vec::new();
        init.write_long_help(&mut help).unwrap();
        let help = String::from_utf8(help).unwrap();

        assert!(help.contains("macos-shortcut"));
    }

    #[test]
    fn init_accepts_all_target() {
        let cli = Cli::try_parse_from(["sivtr", "init", "all"]).unwrap();

        match cli.command {
            Some(Commands::Init { target }) => assert_eq!(target, "all"),
            _ => panic!("expected init command"),
        }
    }

    #[test]
    fn init_accepts_dash_all_target() {
        let cli = Cli::try_parse_from(["sivtr", "init", "-all"]).unwrap();

        match cli.command {
            Some(Commands::Init { target }) => assert_eq!(target, "-all"),
            _ => panic!("expected init command"),
        }
    }

    #[test]
    fn init_accepts_show_target() {
        let cli = Cli::try_parse_from(["sivtr", "init", "show"]).unwrap();

        match cli.command {
            Some(Commands::Init { target }) => assert_eq!(target, "show"),
            _ => panic!("expected init command"),
        }
    }

    #[test]
    fn init_accepts_uninstall_target() {
        let cli = Cli::try_parse_from(["sivtr", "init", "uninstall"]).unwrap();

        match cli.command {
            Some(Commands::Init { target }) => assert_eq!(target, "uninstall"),
            _ => panic!("expected init command"),
        }
    }
}

#[derive(Parser, Debug)]
pub struct ConfigCommand {
    #[command(subcommand)]
    pub action: Option<ConfigAction>,
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Show current config file path and contents
    Show,
    /// Create default config file if it doesn't exist
    Init,
    /// Open config file in editor
    Edit,
}

#[derive(Parser, Debug)]
pub struct HistoryCommand {
    #[command(subcommand)]
    pub action: Option<HistoryAction>,
}

#[derive(Subcommand, Debug)]
pub enum HistoryAction {
    /// Search history by keyword
    Search {
        /// Search keyword
        keyword: String,
        /// Maximum number of results
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },
    /// Show a specific history entry
    Show {
        /// History entry ID
        id: i64,
    },
    /// List recent history entries
    List {
        /// Maximum number of entries to show
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },
}
