use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::{json, Map, Value};
use sivtr_core::agents::AgentProvider;
use sivtr_core::config::SivtrConfig;

use crate::cli::{McpAction, McpCommand, McpInstallArgs, McpLocation};
use crate::commands::interactive;
use crate::mcp;
use crate::output;

const SERVER_NAME: &str = "sivtr";
const SERVER_ARGS: &[&str] = &["mcp", "serve"];
const TOML_MCP_MARKER: &str = "[mcp_servers.sivtr]";

/// Where a host accepts MCP install.
#[derive(Clone, Copy)]
enum McpLocationSupport {
    GlobalOnly,
    GlobalOrLocal,
}

/// Config file shape used by a host.
#[derive(Clone, Copy)]
enum McpConfigKind {
    /// Flat JSON: `{ "<key>": { "sivtr": ... } }`
    Json {
        key: &'static str,
        entry: fn() -> Value,
    },
    /// TOML section: `[mcp_servers.sivtr]`
    Toml,
    /// YAML mapping: `<key>:\n  sivtr: ...`
    Yaml {
        key: &'static str,
        entry: fn() -> serde_yaml::Value,
    },
    /// Nested JSON: `{ "<outer>": { "<inner>": { "sivtr": ... } } }`
    JsonNested {
        outer: &'static str,
        inner: &'static str,
        entry: fn() -> Value,
    },
}

struct McpHostSpec {
    provider: AgentProvider,
    location: McpLocationSupport,
    kind: McpConfigKind,
    config_path: fn(McpLocation) -> PathBuf,
    host_present: fn() -> bool,
}

const MCP_HOSTS: &[McpHostSpec] = &[
    McpHostSpec {
        provider: AgentProvider::Claude,
        location: McpLocationSupport::GlobalOrLocal,
        kind: McpConfigKind::Json {
            key: "mcpServers",
            entry: claude_cursor_entry,
        },
        config_path: claude_config_path,
        host_present: claude_host_present,
    },
    McpHostSpec {
        provider: AgentProvider::Cursor,
        location: McpLocationSupport::GlobalOrLocal,
        kind: McpConfigKind::Json {
            key: "mcpServers",
            entry: claude_cursor_entry,
        },
        config_path: cursor_config_path,
        host_present: cursor_host_present,
    },
    McpHostSpec {
        provider: AgentProvider::Codex,
        location: McpLocationSupport::GlobalOnly,
        kind: McpConfigKind::Toml,
        config_path: |_loc| codex_config_path(),
        host_present: codex_host_present,
    },
    McpHostSpec {
        provider: AgentProvider::OpenCode,
        location: McpLocationSupport::GlobalOrLocal,
        kind: McpConfigKind::Json {
            key: "mcp",
            entry: opencode_entry,
        },
        config_path: opencode_config_path,
        host_present: opencode_host_present,
    },
    McpHostSpec {
        provider: AgentProvider::OpenClaw,
        location: McpLocationSupport::GlobalOnly,
        kind: McpConfigKind::JsonNested {
            outer: "mcp",
            inner: "servers",
            entry: openclaw_entry,
        },
        config_path: |_loc| openclaw_config_path(),
        host_present: openclaw_host_present,
    },
    McpHostSpec {
        provider: AgentProvider::Grok,
        location: McpLocationSupport::GlobalOnly,
        kind: McpConfigKind::Toml,
        config_path: |_loc| grok_config_path(),
        host_present: grok_host_present,
    },
    McpHostSpec {
        provider: AgentProvider::Hermes,
        location: McpLocationSupport::GlobalOnly,
        kind: McpConfigKind::Yaml {
            key: "mcp_servers",
            entry: hermes_entry,
        },
        config_path: |_loc| hermes_config_path(),
        host_present: hermes_host_present,
    },
    McpHostSpec {
        provider: AgentProvider::Pi,
        location: McpLocationSupport::GlobalOnly,
        kind: McpConfigKind::Json {
            key: "mcpServers",
            entry: pi_entry,
        },
        config_path: |_loc| pi_config_path(),
        host_present: pi_host_present,
    },
    McpHostSpec {
        provider: AgentProvider::Qoder,
        location: McpLocationSupport::GlobalOnly,
        kind: McpConfigKind::Json {
            key: "mcpServers",
            entry: claude_cursor_entry,
        },
        config_path: |_loc| qoder_config_path(),
        host_present: qoder_host_present,
    },
    McpHostSpec {
        provider: AgentProvider::QoderCn,
        location: McpLocationSupport::GlobalOnly,
        kind: McpConfigKind::Json {
            key: "mcpServers",
            entry: claude_cursor_entry,
        },
        config_path: |_loc| qoder_cn_config_path(),
        host_present: qoder_cn_host_present,
    },
    McpHostSpec {
        provider: AgentProvider::Gemini,
        location: McpLocationSupport::GlobalOnly,
        kind: McpConfigKind::Json {
            key: "mcpServers",
            entry: pi_entry,
        },
        config_path: |_loc| gemini_config_path(),
        host_present: gemini_host_present,
    },
    McpHostSpec {
        provider: AgentProvider::Qwen,
        location: McpLocationSupport::GlobalOnly,
        kind: McpConfigKind::Json {
            key: "mcpServers",
            entry: pi_entry,
        },
        config_path: |_loc| qwen_config_path(),
        host_present: qwen_host_present,
    },
    McpHostSpec {
        provider: AgentProvider::Goose,
        location: McpLocationSupport::GlobalOnly,
        kind: McpConfigKind::Yaml {
            key: "extensions",
            entry: goose_entry,
        },
        config_path: |_loc| goose_config_path(),
        host_present: goose_host_present,
    },
];

/// Look up the managed MCP host entry for a provider, or explain why there is
/// none. Hosts without an entry (e.g. dsh, whose MCP clients are configured as
/// plugin rows in the harness profile's `cordis.patch.yml`) fail loudly
/// instead of panicking on the user-facing paths.
fn mcp_host(provider: AgentProvider) -> Result<&'static McpHostSpec> {
    MCP_HOSTS
        .iter()
        .find(|host| host.provider == provider)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "sivtr does not manage MCP server configuration for `{}`; \
                 configure the host's MCP clients in its own config instead",
                provider.command_name()
            )
        })
}

pub fn execute(command: McpCommand) -> Result<()> {
    match command.action {
        McpAction::Serve(args) => {
            // CLI `--idle-exit` wins over `[mcp] idle_exit_secs`. 0 = never.
            let idle_secs = resolve_idle_exit_secs(
                args.idle_exit,
                Some(SivtrConfig::load()?.mcp.idle_exit_secs),
            );
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(mcp::serve_stdio(idle_secs.map(Duration::from_secs)))
        }
        McpAction::Install(args) => install(&args),
        McpAction::Uninstall(args) => uninstall(&args),
        McpAction::PrintConfig { target } => {
            let provider = parse_target(&target)?;
            print_config(provider)
        }
    }
}

fn resolve_idle_exit_secs(cli: Option<u64>, config: Option<u64>) -> Option<u64> {
    cli.or(config).filter(|&secs| secs > 0)
}

pub fn install(args: &McpInstallArgs) -> Result<()> {
    // No explicit `-p` and no `-y`: let the user pick hosts interactively.
    // Falls back to detected hosts when stdin is not a TTY.
    let targets = if args.providers.is_empty() && !args.yes {
        pick_targets()?
    } else {
        resolve_targets(&args.providers)?
    };
    if targets.is_empty() {
        bail!("no install targets selected; pass -p with host names, or -p all");
    }
    for target in targets {
        install_target(target, args.location)?;
    }
    Ok(())
}

/// Interactive host multi-select, mirroring `npx skills`: all MCP-capable
/// hosts listed, detected ones pre-checked. Returns `Ok(defaults)` when not
/// interactive, so `doctor --fix` / `setup` (which pass `yes: true`) never
/// prompt and CI pipes keep the old auto-detect behavior.
fn pick_targets() -> Result<Vec<AgentProvider>> {
    let detected = detect_targets();
    let labels: Vec<String> = MCP_HOSTS
        .iter()
        .map(|host| {
            format!(
                "{} ({})",
                host.provider.name(),
                host.provider.command_name()
            )
        })
        .collect();
    let defaults: Vec<usize> = detected
        .iter()
        .filter_map(|target| MCP_HOSTS.iter().position(|host| host.provider == *target))
        .collect();
    let picked = interactive::multi_select(
        "Install sivtr MCP into which agent hosts?",
        &labels,
        &defaults,
    )?;
    Ok(picked.into_iter().map(|i| MCP_HOSTS[i].provider).collect())
}

fn uninstall(args: &McpInstallArgs) -> Result<()> {
    let targets = resolve_targets(&args.providers)?;
    if targets.is_empty() {
        bail!("no uninstall targets resolved");
    }
    for target in targets {
        uninstall_target(target, args.location)?;
    }
    Ok(())
}

fn install_target(target: AgentProvider, location: McpLocation) -> Result<()> {
    let host = mcp_host(target)?;
    ensure_location_allowed(host, location, "install")?;
    let path = (host.config_path)(location);
    match host.kind {
        McpConfigKind::Json { key, entry } => install_json(path, key, entry(), target),
        McpConfigKind::Toml => install_toml(path, target),
        McpConfigKind::Yaml { key, entry } => install_yaml(path, key, entry(), target),
        McpConfigKind::JsonNested {
            outer,
            inner,
            entry,
        } => install_json_nested(path, outer, inner, entry(), target),
    }
}

fn uninstall_target(target: AgentProvider, location: McpLocation) -> Result<()> {
    let host = mcp_host(target)?;
    ensure_location_allowed(host, location, "uninstall")?;
    let path = (host.config_path)(location);
    match host.kind {
        McpConfigKind::Json { key, .. } => uninstall_json(path, key, target),
        McpConfigKind::Toml => uninstall_toml(path, target),
        McpConfigKind::Yaml { key, .. } => uninstall_yaml(path, key, target),
        McpConfigKind::JsonNested { outer, inner, .. } => {
            uninstall_json_nested(path, outer, inner, target)
        }
    }
}

fn ensure_location_allowed(host: &McpHostSpec, location: McpLocation, action: &str) -> Result<()> {
    if matches!(host.location, McpLocationSupport::GlobalOnly)
        && matches!(location, McpLocation::Local)
    {
        bail!(
            "{} only supports global {action} currently; use --location global",
            host.provider.command_name()
        );
    }
    Ok(())
}

fn resolve_targets(providers: &[String]) -> Result<Vec<AgentProvider>> {
    // Default (no -p): detect installed hosts.
    if providers.is_empty() {
        return Ok(detect_targets());
    }

    let mut out = Vec::new();
    for raw in providers {
        for part in raw.split(',') {
            let part = part.trim().to_ascii_lowercase();
            if part.is_empty() {
                continue;
            }
            if part == "auto" {
                bail!(
                    "`auto` was removed; omit `-p` to detect installed hosts, or pass `-p all` / `-p claude,cursor,...`"
                );
            }
            if part == "all" {
                return Ok(managed_targets().collect());
            }
            out.push(parse_target(&part)?);
        }
    }
    // Dedup while preserving order.
    let mut seen = std::collections::HashSet::new();
    out.retain(|provider| seen.insert(*provider));
    Ok(out)
}

fn parse_target(value: &str) -> Result<AgentProvider> {
    AgentProvider::from_command_name(value).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown MCP target `{value}`; expected {}",
            valid_target_list()
        )
    })
}

fn valid_target_list() -> String {
    managed_targets()
        .map(|provider| provider.command_name())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Agent hosts that appear installed (config dir / config file present).
/// Does not imply sivtr MCP is registered. Used for install defaults and doctor.
pub fn detected_hosts() -> Vec<AgentProvider> {
    MCP_HOSTS
        .iter()
        .filter(|host| (host.host_present)())
        .map(|host| host.provider)
        .collect()
}

/// Install targets when `-p` is omitted: detected hosts, or Claude if none found.
pub fn detect_targets() -> Vec<AgentProvider> {
    let mut targets = detected_hosts();
    if targets.is_empty() {
        targets.push(AgentProvider::Claude);
    }
    targets
}

/// Providers with a host configuration that sivtr can manage.
pub fn managed_targets() -> impl Iterator<Item = AgentProvider> {
    MCP_HOSTS.iter().map(|host| host.provider)
}

/// Hosts where sivtr MCP is actually present in config (not merely "host installed").
pub fn registered_targets() -> Vec<AgentProvider> {
    MCP_HOSTS
        .iter()
        .filter(|host| is_mcp_registered(host))
        .map(|host| host.provider)
        .collect()
}

fn is_mcp_registered(host: &McpHostSpec) -> bool {
    match host.location {
        McpLocationSupport::GlobalOnly => config_has_server(host, McpLocation::Global),
        McpLocationSupport::GlobalOrLocal => {
            config_has_server(host, McpLocation::Global)
                || config_has_server(host, McpLocation::Local)
        }
    }
}

fn config_has_server(host: &McpHostSpec, location: McpLocation) -> bool {
    let path = (host.config_path)(location);
    match host.kind {
        McpConfigKind::Json { key, .. } => json_has_server(&path, key),
        McpConfigKind::Toml => toml_has_server(&path),
        McpConfigKind::Yaml { key, .. } => yaml_has_server(&path, key),
        McpConfigKind::JsonNested { outer, inner, .. } => {
            json_nested_has_server(&path, outer, inner)
        }
    }
}

fn print_config(target: AgentProvider) -> Result<()> {
    let host = mcp_host(target)?;
    let path = (host.config_path)(McpLocation::Global);
    output::info(format!("Add to {}", path.display()));
    println!();
    match host.kind {
        McpConfigKind::Json { key, entry } => {
            let mut root = Map::new();
            root.insert(key.to_string(), Value::Object(Map::new()));
            if let Some(Value::Object(servers)) = root.get_mut(key) {
                servers.insert(SERVER_NAME.to_string(), entry());
            }
            println!("{}", serde_json::to_string_pretty(&Value::Object(root))?);
        }
        McpConfigKind::Toml => {
            println!("{}", toml_mcp_snippet());
        }
        McpConfigKind::Yaml { key, entry } => {
            println!("{}", yaml_config_snippet(key, entry())?);
        }
        McpConfigKind::JsonNested {
            outer,
            inner,
            entry,
        } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    outer: {
                        inner: {
                            SERVER_NAME: entry(),
                        }
                    }
                }))?
            );
        }
    }
    Ok(())
}

fn yaml_config_snippet(key: &str, entry: serde_yaml::Value) -> Result<String> {
    let mut root = serde_yaml::Mapping::new();
    let mut servers = serde_yaml::Mapping::new();
    servers.insert(serde_yaml::Value::String(SERVER_NAME.to_string()), entry);
    root.insert(
        serde_yaml::Value::String(key.to_string()),
        serde_yaml::Value::Mapping(servers),
    );
    serde_yaml::to_string(&root).context("failed to serialize MCP YAML snippet")
}

fn install_json(path: PathBuf, key: &str, entry: Value, provider: AgentProvider) -> Result<()> {
    let mut root = read_json_object(&path)?;
    let servers = ensure_object(&mut root, key)?;
    servers.insert(SERVER_NAME.to_string(), entry);
    write_json(&path, &Value::Object(root))?;
    report_installed(provider, &path);
    Ok(())
}

fn uninstall_json(path: PathBuf, key: &str, provider: AgentProvider) -> Result<()> {
    if !path.exists() {
        output::info(format!("no config at {}", path.display()));
        return Ok(());
    }
    let mut root = read_json_object(&path)?;
    if let Some(Value::Object(servers)) = root.get_mut(key) {
        if servers.remove(SERVER_NAME).is_some() {
            write_json(&path, &Value::Object(root))?;
            report_removed(provider, &path);
            return Ok(());
        }
    }
    output::info(format!("sivtr MCP was not installed in {}", path.display()));
    Ok(())
}

fn install_toml(path: PathBuf, provider: AgentProvider) -> Result<()> {
    let mut text = if path.exists() {
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?
    } else {
        String::new()
    };
    remove_toml_mcp_block(&mut text);
    if !text.ends_with('\n') && !text.is_empty() {
        text.push('\n');
    }
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(&toml_mcp_snippet());
    text.push('\n');
    write_text(&path, &text)?;
    report_installed(provider, &path);
    Ok(())
}

fn uninstall_toml(path: PathBuf, provider: AgentProvider) -> Result<()> {
    if !path.exists() {
        output::info(format!(
            "no {} config at {}",
            provider.name(),
            path.display()
        ));
        return Ok(());
    }
    let mut text =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    if !remove_toml_mcp_block(&mut text) {
        output::info(format!("sivtr MCP was not installed in {}", path.display()));
        return Ok(());
    }
    write_text(&path, &text)?;
    report_removed(provider, &path);
    Ok(())
}

fn remove_toml_mcp_block(text: &mut String) -> bool {
    let marker = TOML_MCP_MARKER;
    let Some(start) = text.find(marker) else {
        return false;
    };
    let after = &text[start + marker.len()..];
    let rel_end = after.find("\n[").map(|idx| idx + 1).unwrap_or(after.len());
    let end = start + marker.len() + rel_end;
    let mut begin = start;
    while begin > 0 && text.as_bytes()[begin - 1] == b'\n' {
        begin -= 1;
        if begin > 0 && text.as_bytes()[begin - 1] == b'\n' {
            break;
        }
    }
    text.replace_range(begin..end, "\n");
    true
}

fn install_yaml(
    path: PathBuf,
    key: &str,
    entry: serde_yaml::Value,
    provider: AgentProvider,
) -> Result<()> {
    let mut root: serde_yaml::Value = if path.exists() {
        let text = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        if text.trim().is_empty() {
            serde_yaml::Value::Mapping(Default::default())
        } else {
            serde_yaml::from_str(&text)
                .with_context(|| format!("Failed to parse YAML at {}", path.display()))?
        }
    } else {
        serde_yaml::Value::Mapping(Default::default())
    };

    let servers = ensure_yaml_mapping(&mut root, key)?;
    servers.insert(serde_yaml::Value::String(SERVER_NAME.to_string()), entry);

    let text = serde_yaml::to_string(&root)?;
    write_text(&path, &text)?;
    report_installed(provider, &path);
    Ok(())
}

fn uninstall_yaml(path: PathBuf, key: &str, provider: AgentProvider) -> Result<()> {
    if !path.exists() {
        output::info(format!(
            "no {} config at {}",
            provider.name(),
            path.display()
        ));
        return Ok(());
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    let mut root: serde_yaml::Value = serde_yaml::from_str(&text)
        .with_context(|| format!("Failed to parse YAML at {}", path.display()))?;
    if !remove_yaml_server(&mut root, key, SERVER_NAME) {
        output::info(format!("sivtr MCP was not installed in {}", path.display()));
        return Ok(());
    }
    let text = serde_yaml::to_string(&root)?;
    write_text(&path, &text)?;
    report_removed(provider, &path);
    Ok(())
}

fn install_json_nested(
    path: PathBuf,
    outer: &str,
    inner: &str,
    entry: Value,
    provider: AgentProvider,
) -> Result<()> {
    let mut root = read_json_object(&path)?;
    let outer_obj = ensure_object(&mut root, outer)?;
    let servers = ensure_object(outer_obj, inner)?;
    servers.insert(SERVER_NAME.to_string(), entry);
    write_json(&path, &Value::Object(root))?;
    report_installed(provider, &path);
    Ok(())
}

fn uninstall_json_nested(
    path: PathBuf,
    outer: &str,
    inner: &str,
    provider: AgentProvider,
) -> Result<()> {
    if !path.exists() {
        output::info(format!(
            "no {} config at {}",
            provider.name(),
            path.display()
        ));
        return Ok(());
    }
    let mut root = read_json_object(&path)?;
    let removed = root
        .get_mut(outer)
        .and_then(|value| value.as_object_mut())
        .and_then(|value| value.get_mut(inner))
        .and_then(|servers| servers.as_object_mut())
        .and_then(|servers| servers.remove(SERVER_NAME))
        .is_some();
    if !removed {
        output::info(format!("sivtr MCP was not installed in {}", path.display()));
        return Ok(());
    }
    write_json(&path, &Value::Object(root))?;
    report_removed(provider, &path);
    Ok(())
}

fn report_installed(provider: AgentProvider, path: &Path) {
    output::success(format!(
        "installed MCP server for {} into {}",
        provider.name(),
        path.display()
    ));
}

fn report_removed(provider: AgentProvider, path: &Path) {
    output::success(format!(
        "removed MCP server for {} from {}",
        provider.name(),
        path.display()
    ));
}

fn claude_cursor_entry() -> Value {
    json!({
        "type": "stdio",
        "command": "sivtr",
        "args": SERVER_ARGS,
    })
}

fn opencode_entry() -> Value {
    let mut command = vec!["sivtr"];
    command.extend_from_slice(SERVER_ARGS);
    json!({
        "type": "local",
        "command": command,
        "enabled": true,
    })
}

fn pi_entry() -> Value {
    json!({
        "command": "sivtr",
        "args": SERVER_ARGS,
    })
}

fn openclaw_entry() -> Value {
    json!({
        "command": "sivtr",
        "args": SERVER_ARGS,
    })
}

fn toml_mcp_snippet() -> String {
    format!("{TOML_MCP_MARKER}\ncommand = \"sivtr\"\nargs = [\"mcp\", \"serve\"]\n")
}

fn hermes_entry() -> serde_yaml::Value {
    let mut entry = serde_yaml::Mapping::new();
    entry.insert(
        serde_yaml::Value::String("command".to_string()),
        serde_yaml::Value::String("sivtr".to_string()),
    );
    let args = serde_yaml::Sequence::from([
        serde_yaml::Value::String("mcp".to_string()),
        serde_yaml::Value::String("serve".to_string()),
    ]);
    entry.insert(
        serde_yaml::Value::String("args".to_string()),
        serde_yaml::Value::Sequence(args),
    );
    serde_yaml::Value::Mapping(entry)
}

fn json_has_server(path: &Path, key: &str) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(Value::Object(root)) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    root.get(key)
        .and_then(Value::as_object)
        .is_some_and(|servers| servers.contains_key(SERVER_NAME))
}

fn toml_has_server(path: &Path) -> bool {
    path.exists()
        && fs::read_to_string(path)
            .map(|text| text.contains(TOML_MCP_MARKER))
            .unwrap_or(false)
}

fn yaml_has_server(path: &Path, key: &str) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(root) = serde_yaml::from_str::<serde_yaml::Value>(&text) else {
        return false;
    };
    root.get(key)
        .and_then(serde_yaml::Value::as_mapping)
        .is_some_and(|servers| {
            servers.contains_key(serde_yaml::Value::String(SERVER_NAME.to_string()))
        })
}

fn json_nested_has_server(path: &Path, outer: &str, inner: &str) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(Value::Object(root)) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    root.get(outer)
        .and_then(Value::as_object)
        .and_then(|value| value.get(inner))
        .and_then(Value::as_object)
        .is_some_and(|servers| servers.contains_key(SERVER_NAME))
}

fn ensure_yaml_mapping<'a>(
    root: &'a mut serde_yaml::Value,
    key: &str,
) -> Result<&'a mut serde_yaml::Mapping> {
    if let serde_yaml::Value::Mapping(map) = root {
        if !map.contains_key(serde_yaml::Value::String(key.to_string())) {
            map.insert(
                serde_yaml::Value::String(key.to_string()),
                serde_yaml::Value::Mapping(Default::default()),
            );
        }
    } else {
        bail!("config root must be a YAML mapping");
    }
    match root.get_mut(serde_yaml::Value::String(key.to_string())) {
        Some(serde_yaml::Value::Mapping(map)) => Ok(map),
        Some(_) => bail!("`{key}` must be a YAML mapping"),
        None => unreachable!(),
    }
}

fn remove_yaml_server(root: &mut serde_yaml::Value, key: &str, name: &str) -> bool {
    let serde_yaml::Value::Mapping(map) = root else {
        return false;
    };
    let Some(serde_yaml::Value::Mapping(servers)) =
        map.get_mut(serde_yaml::Value::String(key.to_string()))
    else {
        return false;
    };
    servers
        .remove(serde_yaml::Value::String(name.to_string()))
        .is_some()
}

fn claude_config_path(location: McpLocation) -> PathBuf {
    match location {
        McpLocation::Global => dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".claude.json"),
        McpLocation::Local => env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".mcp.json"),
    }
}

fn cursor_config_path(location: McpLocation) -> PathBuf {
    match location {
        McpLocation::Global => dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".cursor")
            .join("mcp.json"),
        McpLocation::Local => env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".cursor")
            .join("mcp.json"),
    }
}

fn codex_config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
        .join("config.toml")
}

fn opencode_config_path(location: McpLocation) -> PathBuf {
    match location {
        McpLocation::Global => node_config_dir().join("opencode").join("opencode.json"),
        McpLocation::Local => env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("opencode.json"),
    }
}

fn node_config_dir() -> PathBuf {
    if let Ok(dir) = env::var("XDG_CONFIG_HOME") {
        let path = PathBuf::from(dir);
        if path.is_absolute() {
            return path;
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
}

fn pi_home() -> PathBuf {
    if let Ok(path) = env::var("PI_HOME") {
        return PathBuf::from(path);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pi")
}

fn pi_config_path() -> PathBuf {
    node_config_dir().join("mcp").join("mcp.json")
}

fn hermes_home() -> PathBuf {
    if let Ok(path) = env::var("HERMES_HOME") {
        return PathBuf::from(path);
    }
    if cfg!(windows) {
        if let Some(local_data) = dirs::data_local_dir() {
            return local_data.join("hermes");
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".hermes")
}

fn hermes_config_path() -> PathBuf {
    hermes_home().join("config.yaml")
}

fn grok_config_path() -> PathBuf {
    sivtr_core::agents::grok::grok_config_path()
}

fn openclaw_config_path() -> PathBuf {
    sivtr_core::agents::openclaw::openclaw_config_path()
}

fn claude_host_present() -> bool {
    claude_config_path(McpLocation::Global).exists()
        || dirs::home_dir().is_some_and(|home| home.join(".claude").exists())
}

fn cursor_host_present() -> bool {
    cursor_config_path(McpLocation::Global).exists()
        || dirs::home_dir().is_some_and(|home| home.join(".cursor").exists())
}

fn codex_host_present() -> bool {
    codex_config_path().exists()
}

fn opencode_host_present() -> bool {
    opencode_config_path(McpLocation::Global).exists()
        || node_config_dir().join("opencode").exists()
}

fn pi_host_present() -> bool {
    pi_config_path().exists() || pi_home().exists()
}

fn qoder_config_path() -> PathBuf {
    sivtr_core::agents::qoder::qoder_home().join("settings.json")
}

fn qoder_host_present() -> bool {
    qoder_config_path().exists() || sivtr_core::agents::qoder::qoder_home().exists()
}

fn qoder_cn_config_path() -> PathBuf {
    sivtr_core::agents::qoder::qoder_cn_home().join("settings.json")
}

fn qoder_cn_host_present() -> bool {
    qoder_cn_config_path().exists() || sivtr_core::agents::qoder::qoder_cn_home().exists()
}

/// Gemini CLI reads MCP servers from `settings.json` (`mcpServers` key).
fn gemini_config_path() -> PathBuf {
    sivtr_core::agents::gemini::gemini_home().join("settings.json")
}

fn gemini_host_present() -> bool {
    gemini_config_path().exists() || sivtr_core::agents::gemini::gemini_home().exists()
}

/// Qwen Code reads MCP servers from `settings.json` (`mcpServers` key).
fn qwen_config_path() -> PathBuf {
    sivtr_core::agents::qwen::qwen_home().join("settings.json")
}

fn qwen_host_present() -> bool {
    qwen_config_path().exists() || sivtr_core::agents::qwen::qwen_home().exists()
}

/// Goose loads MCP servers from `config.yaml` under `extensions:` (the
/// ExtensionConfig shape written by `goose configure`). Paths follow
/// etcetera's app layout for `Block/goose` on Windows or plain `goose`
/// under XDG config on macOS and Linux.
fn goose_config_path() -> PathBuf {
    if cfg!(windows) {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Block")
            .join("goose")
            .join("config")
            .join("config.yaml")
    } else {
        node_config_dir().join("goose").join("config.yaml")
    }
}

/// Goose extension entry (`extensions.sivtr`) in the ExtensionConfig schema:
/// `{enabled, type: stdio, name, cmd, args, envs}`.
fn goose_entry() -> serde_yaml::Value {
    let mut entry = serde_yaml::Mapping::new();
    entry.insert(
        serde_yaml::Value::String("enabled".to_string()),
        serde_yaml::Value::Bool(true),
    );
    entry.insert(
        serde_yaml::Value::String("type".to_string()),
        serde_yaml::Value::String("stdio".to_string()),
    );
    entry.insert(
        serde_yaml::Value::String("name".to_string()),
        serde_yaml::Value::String(SERVER_NAME.to_string()),
    );
    entry.insert(
        serde_yaml::Value::String("cmd".to_string()),
        serde_yaml::Value::String("sivtr".to_string()),
    );
    let args = serde_yaml::Sequence::from([
        serde_yaml::Value::String("mcp".to_string()),
        serde_yaml::Value::String("serve".to_string()),
    ]);
    entry.insert(
        serde_yaml::Value::String("args".to_string()),
        serde_yaml::Value::Sequence(args),
    );
    entry.insert(
        serde_yaml::Value::String("envs".to_string()),
        serde_yaml::Value::Mapping(Default::default()),
    );
    serde_yaml::Value::Mapping(entry)
}

fn goose_host_present() -> bool {
    goose_config_path().exists() || sivtr_core::agents::goose::goose_db_path().exists()
}

fn hermes_host_present() -> bool {
    hermes_config_path().exists() || hermes_home().exists()
}

fn openclaw_host_present() -> bool {
    openclaw_config_path().exists() || sivtr_core::agents::openclaw::openclaw_home().exists()
}

fn grok_host_present() -> bool {
    grok_config_path().exists() || sivtr_core::agents::grok::grok_home().exists()
}

fn read_json_object(path: &Path) -> Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let text =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    let value: Value = serde_json::from_str(&text)
        .with_context(|| format!("Failed to parse JSON at {}", path.display()))?;
    match value {
        Value::Object(map) => Ok(map),
        _ => bail!("{} must contain a JSON object", path.display()),
    }
}

fn ensure_object<'a>(
    root: &'a mut Map<String, Value>,
    key: &str,
) -> Result<&'a mut Map<String, Value>> {
    if !root.contains_key(key) {
        root.insert(key.to_string(), Value::Object(Map::new()));
    }
    match root.get_mut(key) {
        Some(Value::Object(map)) => Ok(map),
        Some(_) => bail!("`{key}` must be a JSON object"),
        None => unreachable!(),
    }
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    write_text(path, &format!("{}\n", serde_json::to_string_pretty(value)?))
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    fs::write(path, text).with_context(|| format!("Failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn goose_config_uses_xdg_config_home() {
        let dir = tempfile::tempdir().unwrap();
        let config_home = dir.path().join("config");
        let previous = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", &config_home);

        assert_eq!(
            goose_config_path(),
            config_home.join("goose").join("config.yaml")
        );

        match previous {
            Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }

    #[test]
    fn resolves_default_named_and_all_targets() {
        let named = resolve_targets(&["claude".into(), "cursor".into()]).expect("parse");
        assert_eq!(named, vec![AgentProvider::Claude, AgentProvider::Cursor]);

        let csv = resolve_targets(&["claude,cursor".into()]).expect("parse csv");
        assert_eq!(csv, vec![AgentProvider::Claude, AgentProvider::Cursor]);

        assert!(resolve_targets(&["nope".into()]).is_err());
        assert!(resolve_targets(&["auto".into()])
            .unwrap_err()
            .to_string()
            .contains("removed"));

        let all = resolve_targets(&["all".into()]).expect("parse all");
        assert_eq!(all.len(), MCP_HOSTS.len());
    }

    #[test]
    fn resolves_all_targets() {
        let all = resolve_targets(&["all".into()]).expect("parse");
        assert_eq!(all.len(), MCP_HOSTS.len());
        assert!(!all.contains(&AgentProvider::Dsh));
    }

    #[test]
    fn mcp_host_registry_covers_every_managed_provider() {
        let registered: std::collections::HashSet<_> = AgentProvider::all()
            .iter()
            .map(|spec| spec.provider)
            .collect();
        let mut managed = std::collections::HashSet::new();
        for host in MCP_HOSTS {
            assert!(registered.contains(&host.provider));
            assert!(managed.insert(host.provider), "duplicate MCP host");
        }
        for provider in managed {
            let host = mcp_host(provider).expect("managed host resolves");
            assert_eq!(host.provider, provider);
        }
    }

    #[test]
    fn removes_toml_mcp_block() {
        let mut text = String::from(
            "[mcp_servers.context7]\ncommand = \"x\"\n\n[mcp_servers.sivtr]\ncommand = \"sivtr\"\nargs = [\"mcp\", \"serve\"]\n\n[other]\na = 1\n",
        );
        assert!(remove_toml_mcp_block(&mut text));
        assert!(!text.contains("mcp_servers.sivtr"));
        assert!(text.contains("mcp_servers.context7"));
        assert!(text.contains("[other]"));
    }

    #[test]
    fn removes_hermes_server_via_yaml() {
        let text = "auth_key: xyz\nmcp_servers:\n  sivtr:\n    command: sivtr\n    args:\n      - mcp\n      - serve\n\nother: 1\n";
        let mut root: serde_yaml::Value = serde_yaml::from_str(text).expect("parse yaml");
        assert!(remove_yaml_server(&mut root, "mcp_servers", "sivtr"));
        let out = serde_yaml::to_string(&root).unwrap();
        assert!(!out.contains("sivtr"));
        assert!(out.contains("other: 1"));
        assert!(out.contains("mcp_servers:"));
    }

    #[test]
    fn removes_hermes_server_leaves_empty_mapping() {
        let text = "mcp_servers:\n  sivtr:\n    command: sivtr\n";
        let mut root: serde_yaml::Value = serde_yaml::from_str(text).expect("parse yaml");
        assert!(remove_yaml_server(&mut root, "mcp_servers", "sivtr"));
        let out = serde_yaml::to_string(&root).unwrap();
        assert!(out.contains("mcp_servers:"));
        assert!(!out.contains("sivtr"));
    }

    #[test]
    fn prints_provider_specific_yaml_config() {
        let goose = yaml_config_snippet("extensions", goose_entry()).expect("serialize YAML");
        assert!(goose.contains("enabled: true"));
        assert!(goose.contains("type: stdio"));
        assert!(goose.contains("cmd: sivtr"));
        assert!(!goose.contains("command: sivtr"));

        let hermes = yaml_config_snippet("mcp_servers", hermes_entry()).expect("serialize YAML");
        assert!(hermes.contains("command: sivtr"));
    }

    #[test]
    fn resolve_idle_exit_secs_cli_zero_wins_over_config() {
        assert_eq!(resolve_idle_exit_secs(Some(0), Some(60)), None);
        assert_eq!(resolve_idle_exit_secs(Some(30), Some(60)), Some(30));
        assert_eq!(resolve_idle_exit_secs(None, Some(60)), Some(60));
        assert_eq!(resolve_idle_exit_secs(None, Some(0)), None);
    }
}
