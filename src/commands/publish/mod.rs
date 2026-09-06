//! Browser publication: local projection, client-side encryption, and the
//! small local registry needed to revoke bearer links later.

use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;
use sivtr_core::agents::AgentProvider;
use sivtr_core::{
    config::SivtrConfig,
    origin::Reach,
    publication::{
        create_publication_draft, expand_publication_anchors, PublicConversationSnapshot,
        PublicationDraft, PublicationExpiry, PublicationPolicy,
    },
};
use std::io::IsTerminal;

use crate::cli::{
    PublishAction, PublishArgs, PublishCommand, PublishFormat, PublishIdArgs, PublishListArgs,
    PublishPreviewArgs, PublishRevokeArgs,
};
use crate::commands::interactive;
use crate::commands::memory::{filter::Filter, workset};
use crate::output;
use crate::tui::workspace::WorkspaceFocus;

mod registry;
mod transport;

use registry::{is_expired, PublicationDb, PublicationRow, PublicationStatus};
use transport::{
    compress_snapshot, delete_remote, encrypt_snapshot, publication_envelope_size, publication_url,
    random_token, resolve_endpoint, upload,
};
#[cfg(test)]
use transport::{
    encrypt_snapshot_with_nonce, ensure_snapshot_plaintext_limit, ENVELOPE_MAGIC,
    SNAPSHOT_PLAINTEXT_LIMIT,
};

#[derive(Debug, Serialize)]
struct PublicationListItem {
    publication_id: String,
    title: String,
    provider: String,
    status: String,
    created_at: String,
    expires_at: String,
    redaction_count: i64,
    warning_count: i64,
    content_sha256: String,
    last_error: Option<String>,
}

pub fn execute(command: PublishCommand) -> Result<()> {
    let PublishCommand {
        action,
        source,
        title,
        expires,
        yes,
        allow_warnings,
    } = command;
    match action {
        Some(PublishAction::Preview(args)) => preview(args),
        Some(PublishAction::List(args)) => list(args),
        Some(PublishAction::Link(args)) => link(args),
        Some(PublishAction::Revoke(args)) => revoke(args),
        None => publish(PublishArgs {
            source: source.ok_or_else(|| anyhow::anyhow!("publish requires a source"))?,
            title,
            expires,
            yes,
            allow_warnings,
        }),
    }
}

fn load_publication_set(source: &str) -> Result<workset::WorkSet> {
    ensure_local_publication_source(source)?;
    let set = if source.starts_with('@') || is_selector_source(source) {
        workset::query(source, Filter::none(), None)
    } else {
        workset::load_saved(source)
            .with_context(|| format!("failed to load saved WorkSet `{source}`"))
    }
    .with_context(|| format!("failed to resolve publication source `{source}`"))?;
    Ok(set)
}

fn is_selector_source(source: &str) -> bool {
    source.contains(['/', ':', '*'])
        || matches!(
            source.to_ascii_lowercase().as_str(),
            "agent" | "all" | "terminal"
        )
        || AgentProvider::from_command_name(source).is_some()
}

fn ensure_local_publication_source(source: &str) -> Result<()> {
    // Resolve named scopes through the origin registry before querying so a
    // remote alias or group cannot start a daemon or dial a peer.
    let Some((scope, _)) = source.split_once(':') else {
        return Ok(());
    };
    if scope.eq_ignore_ascii_case("local") || is_windows_drive_path(source) {
        return Ok(());
    }
    let cwd = std::env::current_dir().context("failed to resolve current directory")?;
    let registry = crate::origins::collect(&cwd).context("failed to resolve publication scope")?;
    let entry = registry.resolve(scope)?.ok_or_else(|| {
        anyhow::anyhow!("publication scope `{scope}` is not a registered local workspace")
    })?;
    ensure!(
        matches!(&entry.reach, Reach::Local { .. }),
        "publication scope `{scope}` is remote or grouped; only local WorkSets are publishable"
    );
    Ok(())
}

fn is_windows_drive_path(source: &str) -> bool {
    source.len() >= 3
        && source.as_bytes()[0].is_ascii_alphabetic()
        && source.as_bytes()[1] == b':'
        && matches!(source.as_bytes()[2], b'/' | b'\\')
}

pub(crate) fn load_draft_from_set(
    set: &mut workset::WorkSet,
    title: Option<String>,
    expires: PublicationExpiry,
) -> Result<PublicationDraft> {
    set.materialize_parts()?;
    create_publication_draft(
        set.records(),
        set.anchors(),
        &PublicationPolicy {
            title,
            expires,
            published_at: None,
        },
    )
}

pub(crate) fn prepare_picker(
    selection: &workset::WorkSet,
    title: Option<&str>,
    expires: PublicationExpiry,
) -> Result<(workset::WorkSet, PublicationDraft)> {
    ensure!(!selection.anchors().is_empty(), "发布选择为空");
    let anchors = expand_publication_anchors(selection.records(), selection.anchors())?;
    ensure!(!anchors.is_empty(), "发布选择为空");

    let mut set = selection.clone();
    set.select_anchors(anchors);
    let draft = load_draft_from_set(&mut set, title.map(str::to_owned), expires)?;
    Ok((set, draft))
}

fn preview(args: PublishPreviewArgs) -> Result<()> {
    let expires = PublicationExpiry::parse(&args.expires)?;
    let title = args.title.clone();
    let draft = if let Some(source) = args.source {
        let mut set = load_publication_set(&source)?;
        load_draft_from_set(&mut set, title.clone(), expires)?
    } else {
        if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
            bail!("publish preview without a source requires an interactive terminal");
        }
        let providers = AgentProvider::all()
            .iter()
            .map(|spec| spec.provider)
            .collect::<Vec<_>>();
        match crate::commands::browse::run_preview(
            &providers,
            false,
            WorkspaceFocus::Sessions,
            title.clone(),
            expires,
        )? {
            crate::commands::browse::PickerResult::Picked(
                crate::commands::browse::PickedContent::WorkSet { mut set, .. },
            ) => load_draft_from_set(&mut set, title.clone(), expires)?,
            crate::commands::browse::PickerResult::Picked(
                crate::commands::browse::PickedContent::Text { .. },
            ) => bail!("publication preview requires a WorkSet selection"),
            crate::commands::browse::PickerResult::Publish {
                mut set,
                draft,
                expires: _,
                save_name,
            } => {
                save_picker_set(&mut set, save_name.as_deref())?;
                *draft
            }
        }
    };
    print_preview(draft, args.format)
}

fn print_preview(draft: PublicationDraft, format: PublishFormat) -> Result<()> {
    match format {
        PublishFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&draft.snapshot)?);
            print_risks(&draft);
        }
        PublishFormat::Human => {
            print!("{}", format_human_preview_meta(&draft));
            println!();
            print_snapshot_items(&draft.snapshot);
        }
    }
    Ok(())
}

fn print_snapshot_items(snapshot: &PublicConversationSnapshot) {
    match snapshot {
        PublicConversationSnapshot::V1(snapshot) => {
            for item in &snapshot.items {
                println!(
                    "[{}]",
                    match item.role {
                        sivtr_core::publication::PublicRole::User => "用户",
                        sivtr_core::publication::PublicRole::Assistant => "助手",
                    }
                );
                println!("{}", item.text);
                println!();
            }
        }
        PublicConversationSnapshot::V2(snapshot) => {
            for item in &snapshot.items {
                if item.gap_before {
                    println!("[部分内容未分享]");
                    println!();
                }
                let label = item
                    .label
                    .as_deref()
                    .map(|label| format!(" ({label})"))
                    .unwrap_or_default();
                let kind = match item.kind {
                    sivtr_core::publication::PublicEntryKind::User => "用户",
                    sivtr_core::publication::PublicEntryKind::Assistant => "助手",
                    sivtr_core::publication::PublicEntryKind::Tool => "工具",
                    sivtr_core::publication::PublicEntryKind::Skill => "技能",
                    sivtr_core::publication::PublicEntryKind::Thinking => "思考",
                };
                println!("[{kind}{label}]");
                for part in &item.parts {
                    if part.gap_before {
                        println!("[部分内容未分享]");
                        println!();
                    }
                    println!("{}", part.text);
                }
                println!();
                if item.gap_after {
                    println!("[部分内容未分享]");
                    println!();
                }
            }
        }
    }
}

/// Create a link from a picker-confirmed WorkSet.
pub(crate) fn create_from_picker(
    mut set: workset::WorkSet,
    preview_draft: PublicationDraft,
    expires: PublicationExpiry,
    save_name: Option<String>,
) -> Result<()> {
    save_picker_set(&mut set, save_name.as_deref())?;
    let title = preview_draft.snapshot.title().to_string();
    let allow_warnings = preview_draft.warning_count() > 0;
    if allow_warnings {
        interactive::require_interactive("publish with privacy warnings")?;
        if !dialoguer::Confirm::new()
            .with_prompt("公开内容包含隐私风险，仍要创建链接？")
            .default(false)
            .interact()?
        {
            bail!("publication cancelled");
        }
    }
    let url = mint_publication(&mut set, Some(title), expires, allow_warnings)?;
    if let Err(error) = sivtr_core::export::clipboard::copy_to_clipboard(&url) {
        println!("{url}");
        bail!("publication created, but clipboard copy failed: {error:#}");
    }
    output::success("copied link to clipboard");
    println!("{url}");
    Ok(())
}

pub(crate) fn save_picker_set(set: &mut workset::WorkSet, name: Option<&str>) -> Result<()> {
    if let Some(name) = name {
        workset::save_as(set, name)?;
        output::detail("saved", format!("@{name}"));
    }
    Ok(())
}

fn mint_draft(
    draft: PublicationDraft,
    expires: PublicationExpiry,
    compressed: Vec<u8>,
) -> Result<String> {
    let config = SivtrConfig::load()?;
    let endpoint = resolve_endpoint(&config)?;
    let id = format!("{}_{}", expires.as_str(), random_token(16)?);
    let viewer_key = random_token(32)?;
    let management_token = random_token(32)?;
    let created_at = draft.snapshot.published_at().to_string();
    let expires_at = draft.snapshot.expires_at().to_string();
    let mut db = PublicationDb::open()?;
    db.insert_pending(&PublicationRow {
        id: id.clone(),
        endpoint: endpoint.clone(),
        viewer_key: viewer_key.clone(),
        management_token: management_token.clone(),
        title: draft.snapshot.title().to_string(),
        provider: draft.snapshot.provider().to_string(),
        source_refs: serde_json::to_string(&draft.source_refs)
            .context("failed to serialize publication source references")?,
        content_sha256: draft.content_sha256.clone(),
        redaction_count: draft.redaction_count as i64,
        warning_count: draft.warning_count() as i64,
        created_at: created_at.clone(),
        expires_at,
        status: PublicationStatus::Pending,
        last_error: None,
    })?;
    let envelope = match encrypt_snapshot(compressed, &id, &viewer_key) {
        Ok(value) => value,
        Err(error) => {
            let _ = db.mark_failed(&id, &error.to_string());
            return Err(error);
        }
    };
    if let Err(error) = upload(&endpoint, &id, &management_token, &created_at, &envelope) {
        let _ = db.mark_failed(&id, &error.to_string());
        return Err(error);
    }
    if let Err(error) = db.mark_active(&id) {
        bail!("remote publication may have been created, but local state update failed: {error:#}; keep the local database backup for revoke");
    }
    output::detail("publication", &id);
    output::detail("expires", draft.snapshot.expires_at());
    Ok(publication_url(&endpoint, &id, &viewer_key))
}

fn mint_publication(
    set: &mut workset::WorkSet,
    title: Option<String>,
    expires: PublicationExpiry,
    allow_warnings: bool,
) -> Result<String> {
    let (draft, compressed, _) = prepare_publication(set, title, expires)?;
    require_allow_warnings(draft.warning_count() > 0, allow_warnings)?;
    mint_draft(draft, expires, compressed)
}

fn prepare_publication(
    set: &mut workset::WorkSet,
    title: Option<String>,
    expires: PublicationExpiry,
) -> Result<(PublicationDraft, Vec<u8>, usize)> {
    let draft = load_draft_from_set(set, title, expires)?;
    let compressed = compress_snapshot(&draft)?;
    let envelope_size = publication_envelope_size(&compressed)?;
    Ok((draft, compressed, envelope_size))
}

fn publish(args: PublishArgs) -> Result<()> {
    let expires = PublicationExpiry::parse(&args.expires)?;
    let mut set = load_publication_set(&args.source)?;
    let (draft, _, envelope_size) = prepare_publication(&mut set, args.title.clone(), expires)?;
    let has_warnings = draft.warning_count() > 0;
    for (label, value) in format_create_summary(&draft, expires.as_str(), envelope_size) {
        output::detail(label, value);
    }
    if has_warnings {
        output::warning("存在未自动处理的路径、邮箱或内网地址风险；请确认公开内容");
    }
    let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    if !args.yes {
        if !interactive {
            bail!("non-interactive publish requires --yes");
        }
        let confirmed = dialoguer::Confirm::new()
            .with_prompt("创建只读公开链接？")
            .default(false)
            .interact()?;
        if !confirmed {
            bail!("publication cancelled");
        }
    }
    require_allow_warnings(has_warnings, args.allow_warnings)?;
    // Rebuild after confirmation so the snapshot expiry starts at the actual
    // create operation, not before an interactive prompt or warning review.
    let url = mint_publication(&mut set, args.title, expires, args.allow_warnings)?;
    println!("{url}");
    Ok(())
}

fn list(args: PublishListArgs) -> Result<()> {
    let mut db = PublicationDb::open()?;
    db.refresh_expired()?;
    let rows = db.rows()?;
    let items = rows.iter().map(list_item).collect::<Vec<_>>();
    if args.json {
        println!("{}", serde_json::to_string_pretty(&items)?);
    } else if items.is_empty() {
        println!("暂无公开链接");
    } else {
        let show_links = std::io::stdout().is_terminal();
        for (row, item) in rows.iter().zip(items) {
            let link = if row.status == PublicationStatus::Active && !is_expired(&row.expires_at)? {
                let url = publication_url(&row.endpoint, &row.id, &row.viewer_key);
                if show_links {
                    format!("\x1b]8;;{url}\x1b\\{url}\x1b]8;;\x1b\\")
                } else {
                    format!("sivtr publish link {}", row.id)
                }
            } else {
                "-".to_string()
            };
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}",
                item.publication_id, item.status, item.title, item.provider, item.expires_at, link
            );
        }
    }
    Ok(())
}

fn link(args: PublishIdArgs) -> Result<()> {
    let mut db = PublicationDb::open()?;
    db.refresh_expired()?;
    let row = resolve_publication(
        &db,
        args.publication_id,
        |row| row.status == PublicationStatus::Active,
        "Open which publication?",
    )?;
    if row.status != PublicationStatus::Active {
        bail!(
            "publication `{}` is {} and has no usable link",
            row.id,
            row.status.as_str()
        );
    }
    if is_expired(&row.expires_at)? {
        bail!("publication `{}` has expired", row.id);
    }
    println!(
        "{}",
        publication_url(&row.endpoint, &row.id, &row.viewer_key)
    );
    Ok(())
}

fn revoke(args: PublishRevokeArgs) -> Result<()> {
    let mut db = PublicationDb::open()?;
    db.refresh_expired()?;
    let row = resolve_publication(
        &db,
        args.publication_id,
        |row| row.status != PublicationStatus::Revoked,
        "Revoke which publication?",
    )?;
    if row.status == PublicationStatus::Revoked {
        return Ok(());
    }
    if !args.yes {
        interactive::require_interactive("revoke")?;
        if !dialoguer::Confirm::new()
            .with_prompt(format!("撤销 {}？", row.id))
            .default(false)
            .interact()?
        {
            bail!("revoke cancelled");
        }
    }
    match delete_remote(&row.endpoint, &row.id, &row.management_token) {
        Ok(()) => {
            db.mark_revoked(&row.id)?;
            output::success(format!("revoked {}", row.id));
            Ok(())
        }
        Err(error) => {
            let _ = db.record_error(&row.id, &error.to_string());
            Err(error)
        }
    }
}

fn resolve_publication(
    db: &PublicationDb,
    id: Option<String>,
    eligible: impl Fn(&PublicationRow) -> bool,
    prompt: &str,
) -> Result<PublicationRow> {
    match id {
        Some(id) => db
            .find(&id)?
            .ok_or_else(|| anyhow::anyhow!("unknown publication id `{id}`")),
        None => {
            interactive::require_interactive("choose a publication")?;
            let candidates: Vec<_> = db.rows()?.into_iter().filter(eligible).collect();
            if candidates.is_empty() {
                bail!("no matching publications");
            }
            let labels = candidates
                .iter()
                .map(|row| {
                    format!(
                        "{}  {}  {}  {}",
                        row.id,
                        row.status.as_str(),
                        row.title,
                        row.expires_at
                    )
                })
                .collect::<Vec<_>>();
            let selected = interactive::select(prompt, &labels, 0)?;
            candidates
                .into_iter()
                .nth(selected)
                .ok_or_else(|| anyhow::anyhow!("publication selection is empty"))
        }
    }
}

fn format_human_preview_meta(draft: &PublicationDraft) -> String {
    let mut out = format!(
        "标题: {}\n提供方: {}\n快照版本: v{}\n轮次数: {}\n消息数: {}\n预计过期: {}\n内容 SHA-256: {}\n自动脱敏: {} 项\n",
        draft.snapshot.title(),
        draft.snapshot.provider(),
        draft.snapshot.schema_version(),
        draft.turn_count(),
        draft.item_count(),
        draft.snapshot.expires_at(),
        draft.content_sha256,
        draft.redaction_count,
    );
    if draft.risks.is_empty() {
        out.push_str("风险提示: 无\n");
    } else {
        out.push_str("风险提示:\n");
        for risk in &draft.risks {
            out.push_str(&format!(
                "  - {}: {} 项{}\n",
                risk.kind,
                risk.count,
                format_item_indices(&risk.item_indices)
            ));
        }
    }
    out
}

fn format_create_summary(
    draft: &PublicationDraft,
    expiry: &str,
    envelope_size: usize,
) -> Vec<(&'static str, String)> {
    vec![
        ("title", draft.snapshot.title().to_string()),
        ("turns", draft.turn_count().to_string()),
        ("messages", draft.item_count().to_string()),
        ("schema", format!("v{}", draft.snapshot.schema_version())),
        ("envelope", format!("{envelope_size} bytes")),
        ("redactions", draft.redaction_count.to_string()),
        ("expiry", expiry.to_string()),
        (
            "source",
            "local WorkSet; original refs and paths stay local".to_string(),
        ),
    ]
}

fn print_risks(draft: &PublicationDraft) {
    if draft.risks.is_empty() {
        eprintln!("risk warnings: none");
    } else {
        for risk in &draft.risks {
            eprintln!(
                "risk {}: {} item(s){}",
                risk.kind,
                risk.count,
                format_item_indices(&risk.item_indices)
            );
        }
    }
}

fn format_item_indices(indices: &[usize]) -> String {
    if indices.is_empty() {
        String::new()
    } else {
        format!(
            " (message {})",
            indices
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn require_allow_warnings(has_warnings: bool, allow_warnings: bool) -> Result<()> {
    if has_warnings && !allow_warnings {
        bail!("publish with privacy warnings requires --allow-warnings");
    }
    Ok(())
}

fn list_item(row: &PublicationRow) -> PublicationListItem {
    PublicationListItem {
        publication_id: row.id.clone(),
        title: row.title.clone(),
        provider: row.provider.clone(),
        status: row.status.as_str().to_string(),
        created_at: row.created_at.clone(),
        expires_at: row.expires_at.clone(),
        redaction_count: row.redaction_count,
        warning_count: row.warning_count,
        content_sha256: row.content_sha256.clone(),
        last_error: row.last_error.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    use chrono::{DateTime, Utc};
    use rusqlite::Connection;

    use crate::test_fixtures::message_part;
    use sivtr_core::record::{WorkRecord, WorkRef, RECORD_SCHEMA_VERSION};

    #[test]
    fn envelope_has_publication_header_and_aad_is_id_bound() {
        let snapshot = sivtr_core::publication::PublicConversationV1 {
            schema_version: 1,
            title: "t".into(),
            provider: "codex".into(),
            published_at: "2026-01-01T00:00:00Z".into(),
            expires_at: "2026-01-08T00:00:00Z".into(),
            items: vec![],
        };
        let draft = PublicationDraft {
            canonical_json: serde_json::to_string(&snapshot)
                .expect("publication snapshot should serialize"),
            snapshot: PublicConversationSnapshot::V1(snapshot),
            content_sha256: "x".into(),
            redaction_count: 0,
            risks: vec![],
            source_refs: vec![],
        };
        let key = URL_SAFE_NO_PAD.encode([7_u8; 32]);
        let compressed = compress_snapshot(&draft).expect("publication snapshot should compress");
        let envelope = encrypt_snapshot(compressed.clone(), "7d_abc", &key)
            .expect("publication snapshot should encrypt");
        assert_eq!(&envelope[..8], ENVELOPE_MAGIC);
        assert_eq!(envelope[8], 1);
        assert_eq!(envelope[9], 1);
        assert_ne!(
            encrypt_snapshot(
                compress_snapshot(&draft).expect("publication snapshot should compress"),
                "7d_abc",
                &key,
            )
            .expect("publication snapshot should encrypt"),
            envelope
        );
        let fixture = encrypt_snapshot_with_nonce(
            compressed,
            "7d_0123456789abcdefghijkl",
            &key,
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        )
        .expect("publication snapshot should encrypt with a fixed nonce");
        assert_eq!(fixture.len(), 160);
    }

    #[test]
    fn url_keeps_key_in_fragment() {
        let url = publication_url(
            "https://share.hnnulwh.cn",
            "7d_0123456789abcdefghijkl",
            "key",
        );
        assert_eq!(
            url,
            "https://share.hnnulwh.cn/s/7d_0123456789abcdefghijkl#k=key"
        );
    }

    #[test]
    fn local_registry_tracks_pending_active_failed_and_revoked() {
        let connection = Connection::open_in_memory().unwrap();
        let mut db = PublicationDb::from_connection(connection).unwrap();
        let row = PublicationRow {
            id: "7d_0123456789abcdefghijkl".into(),
            endpoint: "https://share.hnnulwh.cn".into(),
            viewer_key: "k".into(),
            management_token: "m".into(),
            title: "title".into(),
            provider: "codex".into(),
            source_refs: "[]".into(),
            content_sha256: "hash".into(),
            redaction_count: 0,
            warning_count: 0,
            created_at: "2026-01-01T00:00:00Z".into(),
            expires_at: "2099-01-01T00:00:00Z".into(),
            status: PublicationStatus::Pending,
            last_error: None,
        };
        db.insert_pending(&row).unwrap();
        assert_eq!(
            db.find(&row.id).unwrap().unwrap().status,
            PublicationStatus::Pending
        );
        db.mark_active(&row.id).unwrap();
        assert_eq!(
            db.find(&row.id).unwrap().unwrap().status,
            PublicationStatus::Active
        );
        db.mark_failed(&row.id, "network").unwrap();
        assert_eq!(
            db.find(&row.id).unwrap().unwrap().status,
            PublicationStatus::Failed
        );
        db.mark_revoked(&row.id).unwrap();
        assert_eq!(
            db.find(&row.id).unwrap().unwrap().status,
            PublicationStatus::Revoked
        );
    }

    #[test]
    fn uncompressed_snapshot_over_16mib_is_rejected() {
        assert!(ensure_snapshot_plaintext_limit(SNAPSHOT_PLAINTEXT_LIMIT).is_ok());
        assert!(ensure_snapshot_plaintext_limit(SNAPSHOT_PLAINTEXT_LIMIT + 1).is_err());
    }

    #[test]
    fn warnings_always_require_explicit_allow() {
        assert!(require_allow_warnings(true, false).is_err());
        assert!(require_allow_warnings(true, true).is_ok());
        assert!(require_allow_warnings(false, false).is_ok());
    }

    #[test]
    fn empty_endpoint_is_rejected() {
        let mut config = SivtrConfig::default();
        config.publish.endpoint.clear();
        assert!(resolve_endpoint(&config).is_err());
        config.publish.endpoint = "https://share.hnnulwh.cn/".into();
        assert_eq!(
            resolve_endpoint(&config).unwrap(),
            "https://share.hnnulwh.cn"
        );
        config.publish.endpoint = "http://example.com".into();
        assert!(resolve_endpoint(&config).is_err());
        config.publish.endpoint = "http://127.0.0.1:8791".into();
        assert_eq!(resolve_endpoint(&config).unwrap(), "http://127.0.0.1:8791");
        config.publish.endpoint = "https://user:secret@example.com".into();
        assert!(resolve_endpoint(&config).is_err());
    }

    fn chat_turn(session: &str, index: usize, thinking: &str, tool_out: &str) -> WorkRecord {
        WorkRecord {
            schema_version: RECORD_SCHEMA_VERSION,
            work_ref: WorkRef::agent(sivtr_core::agents::AgentProvider::Codex, session, index),
            session: sivtr_core::record::WorkSessionRef {
                id: session.into(),
                canonical_id: None,
                path: None,
            },
            cwd: None,
            time: sivtr_core::record::WorkTime::default(),
            status: None,
            title: "Demo".into(),
            parts: vec![
                message_part(1, sivtr_core::record::MessageRole::User, "question"),
                sivtr_core::record::WorkPart {
                    seq: 2,
                    occurred_at: None,
                    body: sivtr_core::record::WorkPartBody::Action {
                        id: "shell-2".into(),
                        actor: sivtr_core::record::WorkActor::Agent,
                        target: sivtr_core::record::WorkTarget::Shell,
                        title: None,
                        input: Some(sivtr_core::record::WorkContent::Text {
                            content: "ls".into(),
                            ansi: None,
                        }),
                        output: vec![sivtr_core::record::WorkContentBlock {
                            content: sivtr_core::record::WorkContent::Text {
                                content: tool_out.into(),
                                ansi: None,
                            },
                            start_line: None,
                        }],
                        status: sivtr_core::record::WorkActionStatus::Completed,
                        exit_code: None,
                    },
                },
                message_part(3, sivtr_core::record::MessageRole::Reasoning, thinking),
                message_part(4, sivtr_core::record::MessageRole::Assistant, "reply"),
            ],
        }
    }

    fn seqs(anchors: &[WorkRef]) -> Vec<usize> {
        anchors
            .iter()
            .map(|anchor| anchor.part().expect("part anchor"))
            .collect()
    }

    #[test]
    fn expand_publication_anchors_scopes_whole_part_and_half_refs() {
        let record = chat_turn("session", 1, "thinking", "ok");
        let whole = sivtr_core::publication::expand_publication_anchors(
            std::slice::from_ref(&record),
            &[record.work_ref.whole()],
        )
        .unwrap();
        assert_eq!(seqs(&whole), vec![1, 2, 3, 4]);

        let tool_part = sivtr_core::publication::expand_publication_anchors(
            std::slice::from_ref(&record),
            &[record.work_ref.with_part(2)],
        )
        .unwrap();
        assert_eq!(seqs(&tool_part), vec![2]);

        let input_half = sivtr_core::publication::expand_publication_anchors(
            std::slice::from_ref(&record),
            &[record.work_ref.with_part(1)],
        )
        .unwrap();
        assert_eq!(seqs(&input_half), vec![1]);

        let other = chat_turn("other", 1, "x", "y");
        assert!(sivtr_core::publication::expand_publication_anchors(
            std::slice::from_ref(&record),
            &[other.work_ref.whole()],
        )
        .is_err());
    }

    #[test]
    fn pick_saved_workset_drops_unselected_turn_bodies() {
        let first = chat_turn("session", 1, "SECRET_TURN1", "tool-1");
        let selected = chat_turn("session", 2, "keep-thinking", "keep-tool");
        let last = chat_turn("session", 3, "secret-think", "SECRET_TURN3");
        let anchors = vec![
            selected.work_ref.with_part(1),
            selected.work_ref.with_part(4),
        ];
        let set = workset::WorkSet::from_parts(
            ".",
            vec![first, selected.clone(), last],
            vec![selected.work_ref.whole()],
        );
        let mut slim = set;
        slim.select_anchors(anchors.clone());
        assert_eq!(slim.records().len(), 1);
        assert_eq!(slim.records()[0].work_ref.index(), 2);
        let persisted = serde_json::to_string(slim.records()).unwrap();
        assert!(!persisted.contains("SECRET_TURN1"));
        assert!(!persisted.contains("SECRET_TURN3"));
        assert!(persisted.contains("keep-thinking"));
        assert!(persisted.contains("keep-tool"));

        let policy = PublicationPolicy {
            published_at: Some(
                DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            ..PublicationPolicy::default()
        };
        let full = create_publication_draft(
            &[
                chat_turn("session", 1, "SECRET_TURN1", "tool-1"),
                selected.clone(),
                chat_turn("session", 3, "secret-think", "SECRET_TURN3"),
            ],
            &anchors,
            &policy,
        )
        .unwrap();
        let from_saved = create_publication_draft(slim.records(), slim.anchors(), &policy).unwrap();
        assert_eq!(full.content_sha256, from_saved.content_sha256);
        let PublicConversationSnapshot::V2(snapshot) = &from_saved.snapshot else {
            panic!("pick-saved WorkSet is schema v2");
        };
        assert!(snapshot.items[0].gap_before);
    }

    #[test]
    fn human_preview_and_create_summary_name_schema_version() {
        let v1_record = chat_turn("session", 1, "thinking", "ok");
        let v1 = create_publication_draft(
            std::slice::from_ref(&v1_record),
            &[],
            &PublicationPolicy::default(),
        )
        .unwrap();
        let v2 = create_publication_draft(
            std::slice::from_ref(&v1_record),
            &[
                v1_record.work_ref.with_part(1),
                v1_record.work_ref.with_part(4),
            ],
            &PublicationPolicy::default(),
        )
        .unwrap();

        let v1_preview = format_human_preview_meta(&v1);
        let v2_preview = format_human_preview_meta(&v2);
        assert!(
            v1_preview.contains("快照版本: v1"),
            "v1 preview missing schema: {v1_preview}"
        );
        assert!(
            v2_preview.contains("快照版本: v2"),
            "v2 preview missing schema: {v2_preview}"
        );

        let v1_summary = format_create_summary(&v1, "7d", 12);
        let v2_summary = format_create_summary(&v2, "7d", 12);
        assert!(v1_summary
            .iter()
            .any(|(label, value)| *label == "schema" && value == "v1"));
        assert!(v2_summary
            .iter()
            .any(|(label, value)| *label == "schema" && value == "v2"));
    }

    #[test]
    fn publish_source_kind_does_not_fallback_between_name_and_selector() {
        assert!(is_selector_source("codex"));
        assert!(is_selector_source("codex/session/1"));
        assert!(is_selector_source("desk:agent"));
        assert!(is_selector_source("terminal"));
        assert!(!is_selector_source("review"));
    }

    #[test]
    fn only_drive_paths_bypass_origin_scope_validation() {
        assert!(is_windows_drive_path("C:\\logs"));
        assert!(!is_windows_drive_path("r:codex/session/1"));
    }
}
