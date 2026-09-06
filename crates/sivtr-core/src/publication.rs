//! Provider-neutral, privacy-minimized public conversation snapshots.

use anyhow::{bail, ensure, Context, Result};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::privacy;
use crate::record::{MessageRole, WorkPart, WorkPartBody, WorkRecord, WorkRecordKind, WorkRef};

pub const PUBLICATION_SCHEMA_VERSION: u32 = 1;
pub const GRANULAR_PUBLICATION_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PublicationExpiry {
    TwoHours,
    OneDay,
    ThreeDays,
    #[default]
    SevenDays,
    ThirtyDays,
}

impl PublicationExpiry {
    pub const PICKER_CHOICES: [Self; 5] = [
        Self::TwoHours,
        Self::OneDay,
        Self::ThreeDays,
        Self::SevenDays,
        Self::ThirtyDays,
    ];

    pub fn picker_default_index() -> usize {
        3
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "2h" => Ok(Self::TwoHours),
            "1d" => Ok(Self::OneDay),
            "3d" => Ok(Self::ThreeDays),
            "7d" => Ok(Self::SevenDays),
            "30d" => Ok(Self::ThirtyDays),
            _ => {
                bail!("invalid publication expiry `{value}`; expected 2h, 1d, 3d, 7d, or 30d")
            }
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::TwoHours => "2h",
            Self::OneDay => "1d",
            Self::ThreeDays => "3d",
            Self::SevenDays => "7d",
            Self::ThirtyDays => "30d",
        }
    }

    fn duration(self) -> Duration {
        match self {
            Self::TwoHours => Duration::hours(2),
            Self::OneDay => Duration::days(1),
            Self::ThreeDays => Duration::days(3),
            Self::SevenDays => Duration::days(7),
            Self::ThirtyDays => Duration::days(30),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PublicationPolicy {
    pub title: Option<String>,
    pub expires: PublicationExpiry,
    /// Injectable for deterministic tests; production callers leave this None.
    pub published_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicConversationV1 {
    pub schema_version: u32,
    pub title: String,
    pub provider: String,
    pub published_at: String,
    pub expires_at: String,
    pub items: Vec<PublicConversationItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicConversationItem {
    pub role: PublicRole,
    pub text: String,
    pub occurred_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicConversationV2 {
    pub schema_version: u32,
    pub title: String,
    pub provider: String,
    pub published_at: String,
    pub expires_at: String,
    pub items: Vec<PublicConversationEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicConversationEntry {
    pub kind: PublicEntryKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub parts: Vec<PublicConversationPart>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gap_before: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gap_after: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicConversationPart {
    pub kind: PublicPartKind,
    pub text: String,
    pub occurred_at: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gap_before: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PublicEntryKind {
    User,
    Assistant,
    Tool,
    Skill,
    Thinking,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PublicPartKind {
    User,
    Assistant,
    ToolCall,
    ToolResult,
    Skill,
    Thinking,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum PublicConversationSnapshot {
    V1(PublicConversationV1),
    V2(PublicConversationV2),
}

impl PublicConversationSnapshot {
    pub fn title(&self) -> &str {
        match self {
            Self::V1(snapshot) => &snapshot.title,
            Self::V2(snapshot) => &snapshot.title,
        }
    }

    pub fn provider(&self) -> &str {
        match self {
            Self::V1(snapshot) => &snapshot.provider,
            Self::V2(snapshot) => &snapshot.provider,
        }
    }

    pub fn published_at(&self) -> &str {
        match self {
            Self::V1(snapshot) => &snapshot.published_at,
            Self::V2(snapshot) => &snapshot.published_at,
        }
    }

    pub fn expires_at(&self) -> &str {
        match self {
            Self::V1(snapshot) => &snapshot.expires_at,
            Self::V2(snapshot) => &snapshot.expires_at,
        }
    }

    pub fn item_count(&self) -> usize {
        match self {
            Self::V1(snapshot) => snapshot.items.len(),
            Self::V2(snapshot) => snapshot.items.len(),
        }
    }

    pub fn schema_version(&self) -> u32 {
        match self {
            Self::V1(snapshot) => snapshot.schema_version,
            Self::V2(snapshot) => snapshot.schema_version,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PublicRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationRisk {
    pub kind: String,
    pub count: usize,
    pub item_indices: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct PublicationDraft {
    pub snapshot: PublicConversationSnapshot,
    pub canonical_json: String,
    pub content_sha256: String,
    pub redaction_count: usize,
    pub risks: Vec<PublicationRisk>,
    pub source_refs: Vec<String>,
}

impl PublicationDraft {
    pub fn item_count(&self) -> usize {
        self.snapshot.item_count()
    }

    pub fn warning_count(&self) -> usize {
        self.risks
            .iter()
            .filter(|risk| privacy::is_manual_warning(&risk.kind))
            .map(|risk| risk.count)
            .sum()
    }

    pub fn turn_count(&self) -> usize {
        self.source_refs
            .iter()
            .filter_map(|reference| reference.parse::<WorkRef>().ok())
            .map(|reference| reference.whole().to_string())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    }
}

/// Validate and project a WorkSet's materialized records into a public
/// snapshot. The core never receives a CLI WorkSet type.
pub fn create_publication_draft(
    records: &[WorkRecord],
    anchors: &[WorkRef],
    policy: &PublicationPolicy,
) -> Result<PublicationDraft> {
    ensure!(!records.is_empty(), "cannot publish an empty WorkSet");
    let normalized = if anchors.is_empty() {
        records
            .iter()
            .map(|record| record.work_ref.whole())
            .collect::<Vec<_>>()
    } else {
        anchors.to_vec()
    };
    let has_whole = normalized.iter().any(|anchor| anchor.part().is_none());
    let has_part = normalized.iter().any(|anchor| anchor.part().is_some());
    ensure!(
        !(has_whole && has_part),
        "publication cannot mix whole-record and part anchors"
    );
    if has_part {
        return create_granular_publication_draft(records, &normalized, policy);
    }
    create_record_publication_draft(records, &normalized, policy)
}

/// Expand selected publication anchors to whole parts.
pub fn expand_publication_anchors(
    records: &[WorkRecord],
    picked: &[WorkRef],
) -> Result<Vec<WorkRef>> {
    let mut selected: std::collections::BTreeMap<
        String,
        (usize, std::collections::BTreeSet<usize>),
    > = std::collections::BTreeMap::new();
    for anchor in picked {
        let record_index = records
            .iter()
            .position(|record| record.work_ref.whole() == anchor.whole())
            .ok_or_else(|| anyhow::anyhow!("publication anchor `{anchor}` has no record"))?;
        let record = &records[record_index];
        let entry = selected
            .entry(record.work_ref.whole().to_string())
            .or_insert_with(|| (record_index, std::collections::BTreeSet::new()));
        match anchor.part() {
            Some(seq) => {
                ensure!(
                    record.parts.iter().any(|part| part.seq == seq),
                    "publication anchor `{anchor}` has no part"
                );
                entry.1.insert(seq);
            }
            None => entry.1.extend(record.parts.iter().map(|part| part.seq)),
        }
    }

    let mut groups = selected.into_values().collect::<Vec<_>>();
    groups.sort_by_key(|(record_index, _)| records[*record_index].work_ref.index());
    let mut anchors = Vec::new();
    for (record_index, selected_parts) in groups {
        let record = &records[record_index];
        let mut seqs = selected_parts.into_iter().collect::<Vec<_>>();
        seqs.sort_unstable();
        anchors.extend(seqs.into_iter().map(|seq| record.work_ref.with_part(seq)));
    }
    Ok(anchors)
}

fn add_risks(
    risk_map: &mut std::collections::BTreeMap<String, PublicationRisk>,
    warnings: impl IntoIterator<Item = String>,
    item_index: Option<usize>,
) {
    for kind in warnings {
        let entry = risk_map
            .entry(kind.clone())
            .or_insert_with(|| PublicationRisk {
                kind,
                count: 0,
                item_indices: Vec::new(),
            });
        entry.count += 1;
        if let Some(item_index) = item_index {
            entry.item_indices.push(item_index);
        }
    }
}

fn create_record_publication_draft(
    records: &[WorkRecord],
    anchors: &[WorkRef],
    policy: &PublicationPolicy,
) -> Result<PublicationDraft> {
    ensure!(!records.is_empty(), "cannot publish an empty WorkSet");
    let expected = anchors.iter().map(WorkRef::whole).collect::<Vec<_>>();
    ensure!(
        expected.len() == records.len(),
        "publication anchors and records must have the same length"
    );

    // Search defaults to newest-first; publish snapshots are chronological.
    let mut order: Vec<usize> = (0..records.len()).collect();
    order.sort_by_key(|&i| records[i].work_ref.index());

    let first = &records[order[0]];
    ensure!(
        first.kind == WorkRecordKind::ChatTurn,
        "publish v1 only supports agent conversations, not terminal records"
    );
    ensure!(
        first.work_ref.is_local(),
        "publish v1 only supports local WorkSets"
    );
    ensure!(
        first.work_ref.part().is_none(),
        "publish v1 requires record-level anchors, not part anchors"
    );
    let provider = first
        .work_ref
        .provider()
        .ok_or_else(|| anyhow::anyhow!("publish v1 requires an agent provider"))?;
    let session = first.work_ref.session().to_string();
    let mut source_refs = Vec::with_capacity(records.len());
    let mut items = Vec::new();
    let mut redaction_count = 0;
    let mut risk_map: std::collections::BTreeMap<String, PublicationRisk> =
        std::collections::BTreeMap::new();
    let mut previous_index = None;

    for &idx in &order {
        let record = &records[idx];
        ensure!(
            record.work_ref.whole() == expected[idx],
            "publication anchors must match records in order"
        );
        ensure!(
            record.kind == WorkRecordKind::ChatTurn,
            "publish v1 only supports agent conversations"
        );
        ensure!(
            record.source.channel == crate::record::WorkChannel::Chat,
            "publication contains a non-chat record"
        );
        ensure!(
            record.source.provider.as_deref() == Some(provider.command_name()),
            "publication provider metadata does not match its WorkRef"
        );
        ensure!(
            record.work_ref.is_local(),
            "publication contains a remote or group record"
        );
        ensure!(
            record.work_ref.part().is_none(),
            "publication anchors must target whole records"
        );
        ensure!(
            record.work_ref.provider() == Some(provider),
            "publication cannot mix agent providers"
        );
        ensure!(
            record.work_ref.session() == session,
            "publication cannot mix agent sessions"
        );
        if let Some(previous) = previous_index {
            ensure!(
                record.work_ref.index() == previous + 1,
                "publication record indices must be strictly continuous"
            );
        }
        previous_index = Some(record.work_ref.index());
        source_refs.push(record.work_ref.to_string());

        for part in &record.parts {
            let role = match part.message_role() {
                Some(MessageRole::User) => PublicRole::User,
                Some(MessageRole::Assistant) => PublicRole::Assistant,
                _ => continue,
            };
            let raw = part.text().into_owned();
            let (text, report) = privacy::redact_text_with_report(&raw)?;
            redaction_count += report.redactions;
            let item_index = (!text.trim().is_empty()).then_some(items.len() + 1);
            add_risks(&mut risk_map, report.warnings, item_index);
            if !text.trim().is_empty() {
                items.push(PublicConversationItem {
                    role,
                    text,
                    occurred_at: part
                        .occurred_at
                        .clone()
                        .or_else(|| record.time.primary_at().map(str::to_string)),
                });
            }
        }
    }
    ensure!(
        items.iter().any(|item| item.role == PublicRole::Assistant),
        "publication must contain at least one assistant reply"
    );

    let now = policy.published_at.unwrap_or_else(Utc::now);
    let expires_at = now + policy.expires.duration();
    let title_raw = policy
        .title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| first.title.clone());
    let (title, title_report) = privacy::redact_text_with_report(&title_raw)?;
    redaction_count += title_report.redactions;
    add_risks(&mut risk_map, title_report.warnings, None);
    let snapshot = PublicConversationV1 {
        schema_version: PUBLICATION_SCHEMA_VERSION,
        title: if title.trim().is_empty() {
            "Sivtr conversation".to_string()
        } else {
            title
        },
        provider: provider.command_name().to_string(),
        published_at: now.to_rfc3339_opts(SecondsFormat::Millis, true),
        expires_at: expires_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        items,
    };
    let canonical_json =
        serde_json::to_string(&snapshot).context("failed to serialize publication snapshot")?;
    let content_sha256 = hex_sha256(canonical_json.as_bytes());
    let risks = risk_map
        .into_values()
        .map(|mut risk| {
            risk.item_indices.sort_unstable();
            risk.item_indices.dedup();
            risk
        })
        .collect();
    Ok(PublicationDraft {
        snapshot: PublicConversationSnapshot::V1(snapshot),
        canonical_json,
        content_sha256,
        redaction_count,
        risks,
        source_refs,
    })
}

fn create_granular_publication_draft(
    records: &[WorkRecord],
    anchors: &[WorkRef],
    policy: &PublicationPolicy,
) -> Result<PublicationDraft> {
    ensure!(
        !anchors.is_empty(),
        "cannot publish an empty part selection"
    );
    ensure!(
        anchors.iter().all(|anchor| anchor.part().is_some()),
        "granular publication requires part anchors"
    );

    let first_record = record_for_whole_anchor(records, &anchors[0])?;
    validate_granular_record(first_record, None, None)?;
    let provider = first_record
        .work_ref
        .provider()
        .ok_or_else(|| anyhow::anyhow!("publish requires an agent provider"))?;
    let session = first_record.work_ref.session().to_string();

    // Group and deduplicate part anchors by their owning record. The record
    // index is the stable order used by the publication snapshot.
    let mut groups: std::collections::BTreeMap<String, (usize, std::collections::BTreeSet<usize>)> =
        std::collections::BTreeMap::new();
    for anchor in anchors {
        let record_index = records
            .iter()
            .position(|record| record.work_ref.whole() == anchor.whole())
            .ok_or_else(|| anyhow::anyhow!("publication anchor `{anchor}` has no record"))?;
        let record = &records[record_index];
        validate_granular_record(record, Some(provider), Some(&session))?;
        let seq = anchor.part().expect("part anchor validated");
        ensure!(
            record
                .part_for_at(crate::record::WorkAt::Part(seq))
                .is_some(),
            "publication anchor `{anchor}` points to a missing part"
        );
        groups
            .entry(record.work_ref.whole().to_string())
            .or_insert_with(|| (record_index, std::collections::BTreeSet::new()))
            .1
            .insert(seq);
    }

    let mut ordered_groups = groups.into_values().collect::<Vec<_>>();
    ordered_groups.sort_by_key(|(record_index, _)| records[*record_index].work_ref.index());

    let mut items = Vec::new();
    let mut source_refs = Vec::new();
    let mut redaction_count = 0;
    let mut risk_map: std::collections::BTreeMap<String, PublicationRisk> =
        std::collections::BTreeMap::new();
    let mut previous: Option<(usize, usize, usize, std::collections::BTreeSet<usize>)> = None;

    for (record_index, selected) in ordered_groups {
        let record = &records[record_index];
        for part in &record.parts {
            if !selected.contains(&part.seq) {
                continue;
            }
            let first_seq = part.seq;
            let item_kind = public_item_kind(part)?;
            let gap_before = match previous.as_ref() {
                Some((previous_work_index, _, previous_last, _))
                    if *previous_work_index == record.work_ref.index() =>
                {
                    record.parts.iter().any(|part| {
                        part.seq > *previous_last
                            && part.seq < first_seq
                            && !selected.contains(&part.seq)
                    })
                }
                Some((
                    previous_work_index,
                    previous_position,
                    previous_last,
                    previous_selected,
                )) => {
                    *previous_work_index + 1 != record.work_ref.index()
                        || records[*previous_position].parts.iter().any(|part| {
                            part.seq > *previous_last && !previous_selected.contains(&part.seq)
                        })
                        || record
                            .parts
                            .iter()
                            .any(|part| part.seq < first_seq && !selected.contains(&part.seq))
                }
                None => {
                    record.work_ref.index() > 1
                        || record
                            .parts
                            .iter()
                            .any(|part| part.seq < first_seq && !selected.contains(&part.seq))
                }
            };

            let mut public_parts = Vec::new();
            let mut item_warnings = Vec::new();
            let (text, report) = privacy::redact_text_with_report(&part.text())?;
            redaction_count += report.redactions;
            item_warnings.extend(report.warnings);
            if !text.trim().is_empty() {
                public_parts.push(PublicConversationPart {
                    kind: public_part_kind(part)?,
                    text,
                    occurred_at: part
                        .occurred_at
                        .clone()
                        .or_else(|| record.time.primary_at().map(str::to_string)),
                    gap_before: false,
                });
            }
            if public_parts.is_empty() {
                add_risks(&mut risk_map, item_warnings, None);
                continue;
            }
            source_refs.push(record.work_ref.with_part(part.seq).to_string());
            let item_index = items.len() + 1;
            let label = if let Some(raw_label) = part.label() {
                let (redacted, report) = privacy::redact_text_with_report(raw_label)?;
                redaction_count += report.redactions;
                item_warnings.extend(report.warnings);
                (!redacted.trim().is_empty()).then_some(redacted)
            } else {
                None
            };
            add_risks(&mut risk_map, item_warnings, Some(item_index));
            items.push(PublicConversationEntry {
                kind: item_kind,
                label,
                parts: public_parts,
                gap_before,
                gap_after: false,
            });
            previous = Some((
                record.work_ref.index(),
                record_index,
                part.seq,
                selected.clone(),
            ));
        }
    }
    ensure!(
        !items.is_empty(),
        "publication must contain visible selected content"
    );
    if let Some((_, record_index, last_seq, selected)) = previous.as_ref() {
        let record = &records[*record_index];
        if omitted_after(record, *last_seq, selected) {
            if let Some(item) = items.last_mut() {
                item.gap_after = true;
            }
        }
    }

    let now = policy.published_at.unwrap_or_else(Utc::now);
    let expires_at = now + policy.expires.duration();
    let title_raw = policy
        .title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| title_from_public_items(&items));
    let (title, title_report) = privacy::redact_text_with_report(&title_raw)?;
    redaction_count += title_report.redactions;
    add_risks(&mut risk_map, title_report.warnings, None);

    let snapshot = PublicConversationV2 {
        schema_version: GRANULAR_PUBLICATION_SCHEMA_VERSION,
        title: if title.trim().is_empty() {
            "Sivtr conversation".to_string()
        } else {
            title
        },
        provider: provider.command_name().to_string(),
        published_at: now.to_rfc3339_opts(SecondsFormat::Millis, true),
        expires_at: expires_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        items,
    };
    let canonical_json =
        serde_json::to_string(&snapshot).context("failed to serialize publication snapshot")?;
    let content_sha256 = granular_content_sha256(&snapshot)?;
    let risks = risk_map
        .into_values()
        .map(|mut risk| {
            risk.item_indices.sort_unstable();
            risk.item_indices.dedup();
            risk
        })
        .collect();

    Ok(PublicationDraft {
        snapshot: PublicConversationSnapshot::V2(snapshot),
        canonical_json,
        content_sha256,
        redaction_count,
        risks,
        source_refs,
    })
}

fn omitted_after(
    record: &WorkRecord,
    last_seq: usize,
    selected: &std::collections::BTreeSet<usize>,
) -> bool {
    record
        .parts
        .iter()
        .any(|part| part.seq > last_seq && !selected.contains(&part.seq))
}

fn record_for_whole_anchor<'a>(
    records: &'a [WorkRecord],
    anchor: &WorkRef,
) -> Result<&'a WorkRecord> {
    records
        .iter()
        .find(|record| record.work_ref.whole() == anchor.whole())
        .ok_or_else(|| anyhow::anyhow!("publication anchor `{anchor}` has no record"))
}

fn validate_granular_record(
    record: &WorkRecord,
    provider: Option<crate::agents::AgentProvider>,
    session: Option<&str>,
) -> Result<()> {
    ensure!(
        record.kind == WorkRecordKind::ChatTurn,
        "granular publication only supports agent conversations"
    );
    ensure!(
        record.source.channel == crate::record::WorkChannel::Chat,
        "publication contains a non-chat record"
    );
    ensure!(
        record.work_ref.is_local(),
        "publication contains a remote or group record"
    );
    let record_provider = record
        .work_ref
        .provider()
        .ok_or_else(|| anyhow::anyhow!("publish requires an agent provider"))?;
    ensure!(
        record.source.provider.as_deref() == Some(record_provider.command_name()),
        "publication provider metadata does not match its WorkRef"
    );
    if let Some(provider) = provider {
        ensure!(
            record_provider == provider,
            "publication cannot mix agent providers"
        );
    }
    if let Some(session) = session {
        ensure!(
            record.work_ref.session() == session,
            "publication cannot mix agent sessions"
        );
    }
    Ok(())
}

fn public_item_kind(part: &WorkPart) -> Result<PublicEntryKind> {
    match &part.body {
        WorkPartBody::Message { role, .. } => match role {
            MessageRole::User => Ok(PublicEntryKind::User),
            MessageRole::Assistant => Ok(PublicEntryKind::Assistant),
            MessageRole::System => Ok(PublicEntryKind::Skill),
            MessageRole::Reasoning => Ok(PublicEntryKind::Thinking),
        },
        WorkPartBody::Action { .. } => Ok(PublicEntryKind::Tool),
    }
}

fn public_part_kind(part: &WorkPart) -> Result<PublicPartKind> {
    match &part.body {
        WorkPartBody::Message { role, .. } => match role {
            MessageRole::User => Ok(PublicPartKind::User),
            MessageRole::Assistant => Ok(PublicPartKind::Assistant),
            MessageRole::System => Ok(PublicPartKind::Skill),
            MessageRole::Reasoning => Ok(PublicPartKind::Thinking),
        },
        WorkPartBody::Action { .. } => Ok(PublicPartKind::ToolCall),
    }
}

fn title_from_public_items(items: &[PublicConversationEntry]) -> String {
    let preferred = [
        PublicEntryKind::User,
        PublicEntryKind::Assistant,
        PublicEntryKind::Skill,
        PublicEntryKind::Tool,
        PublicEntryKind::Thinking,
    ];
    for kind in preferred {
        if let Some(text) = items
            .iter()
            .filter(|item| item.kind == kind)
            .flat_map(|item| item.parts.iter())
            .map(|part| part.text.trim())
            .find(|text| !text.is_empty())
        {
            let line = text.lines().next().unwrap_or(text).trim();
            let mut title = line.chars().take(80).collect::<String>();
            if line.chars().count() > 80 {
                title.push('…');
            }
            return title;
        }
    }
    "Sivtr conversation".to_string()
}

/// Hash only the stable public content. Publication timestamps belong to the
/// snapshot envelope, but must not make preview and the subsequent create of
/// the same saved selection look like different content.
fn granular_content_sha256(snapshot: &PublicConversationV2) -> Result<String> {
    let mut value = serde_json::to_value(snapshot)
        .context("failed to serialize granular publication snapshot")?;
    if let serde_json::Value::Object(fields) = &mut value {
        fields.remove("published_at");
        fields.remove("expires_at");
    }
    let canonical = serde_json::to_string(&value)
        .context("failed to serialize granular publication content")?;
    Ok(hex_sha256(canonical.as_bytes()))
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{
        MessageRole, WorkActionStatus, WorkActor, WorkChannel, WorkContent, WorkContentBlock,
        WorkPart, WorkPartBody, WorkRecord, WorkRef, WorkSessionRef, WorkSource, WorkTarget,
        WorkTime, RECORD_SCHEMA_VERSION,
    };
    use crate::test_fixtures::message_part;

    fn user_part(seq: usize, content: &str) -> WorkPart {
        message_part(seq, MessageRole::User, content)
    }

    fn assistant_part(seq: usize, content: &str) -> WorkPart {
        message_part(seq, MessageRole::Assistant, content)
    }

    fn shell_part(seq: usize, command: &str, output: &str) -> WorkPart {
        WorkPart {
            seq,
            occurred_at: None,
            body: WorkPartBody::Action {
                id: format!("action-{seq}"),
                actor: WorkActor::Agent,
                target: WorkTarget::Shell,
                title: None,
                input: Some(WorkContent::Text {
                    content: command.into(),
                    ansi: None,
                }),
                output: (!output.is_empty())
                    .then(|| WorkContentBlock {
                        content: WorkContent::Text {
                            content: output.into(),
                            ansi: None,
                        },
                        start_line: None,
                    })
                    .into_iter()
                    .collect(),
                status: WorkActionStatus::Completed,
                exit_code: None,
            },
        }
    }

    fn thinking_part(seq: usize, content: &str) -> WorkPart {
        message_part(seq, MessageRole::Reasoning, content)
    }

    fn record(index: usize, assistant: &str) -> WorkRecord {
        WorkRecord {
            schema_version: RECORD_SCHEMA_VERSION,
            work_ref: WorkRef::agent(crate::agents::AgentProvider::Codex, "session", index),
            kind: WorkRecordKind::ChatTurn,
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some("codex".into()),
            },
            session: WorkSessionRef {
                id: "session".into(),
                canonical_id: None,
                path: None,
            },
            cwd: Some("C:\\secret".into()),
            time: WorkTime::default(),
            status: None,
            title: "Demo".into(),
            parts: vec![user_part(1, "hello"), assistant_part(2, assistant)],
        }
    }

    fn granular_record(index: usize) -> WorkRecord {
        let mut record = record(index, "reply");
        record.parts = vec![
            user_part(1, "question"),
            shell_part(2, "pwd", "token=sk-abcdefghijklmnop1234"),
            thinking_part(3, "internal reasoning"),
            assistant_part(4, "reply"),
        ];
        record
    }

    #[test]
    fn projects_only_dialogue_and_redacts_secrets() {
        let records = vec![record(3, "token=sk-abcd1234efgh5678ijkl")];
        let draft = create_publication_draft(&records, &[], &PublicationPolicy::default()).unwrap();
        assert_eq!(draft.item_count(), 2);
        let PublicConversationSnapshot::V1(snapshot) = &draft.snapshot else {
            panic!("whole records use the v1 snapshot")
        };
        assert_eq!(snapshot.items[1].text, "token=[REDACTED]");
        assert_eq!(draft.redaction_count, 1);
        let json = serde_json::to_string(&draft.snapshot).unwrap();
        assert!(!json.contains("work_ref"));
        assert!(!json.contains("cwd"));
        assert!(!json.contains("session"));
    }

    #[test]
    fn warning_count_only_includes_manual_privacy_warnings() {
        let draft = PublicationDraft {
            snapshot: PublicConversationSnapshot::V1(PublicConversationV1 {
                schema_version: PUBLICATION_SCHEMA_VERSION,
                title: "title".into(),
                provider: "codex".into(),
                published_at: "2026-01-01T00:00:00Z".into(),
                expires_at: "2026-01-08T00:00:00Z".into(),
                items: Vec::new(),
            }),
            canonical_json: "{}".into(),
            content_sha256: "hash".into(),
            redaction_count: 2,
            risks: vec![
                PublicationRisk {
                    kind: "absolute_path".into(),
                    count: 2,
                    item_indices: vec![1, 2],
                },
                PublicationRisk {
                    kind: "secret".into(),
                    count: 3,
                    item_indices: vec![1],
                },
            ],
            source_refs: Vec::new(),
        };
        assert_eq!(draft.warning_count(), 2);
    }

    #[test]
    fn rejects_gaps_and_mixed_sessions() {
        let records = vec![record(1, "a"), record(3, "b")];
        assert!(create_publication_draft(&records, &[], &PublicationPolicy::default()).is_err());
        let mut mixed = record(2, "b");
        mixed.work_ref = WorkRef::agent(crate::agents::AgentProvider::Codex, "other", 2);
        assert!(create_publication_draft(
            &[record(1, "a"), mixed],
            &[],
            &PublicationPolicy::default()
        )
        .is_err());
        assert!(create_publication_draft(
            &[record(1, "a")],
            &[
                WorkRef::agent(crate::agents::AgentProvider::Codex, "session", 1).with_part(1),
                WorkRef::agent(crate::agents::AgentProvider::Codex, "session", 1).with_part(2),
            ],
            &PublicationPolicy::default()
        )
        .is_ok());
    }

    #[test]
    fn newest_first_records_are_sorted_before_continuity_check() {
        let records = vec![record(3, "c"), record(2, "b"), record(1, "a")];
        let draft = create_publication_draft(&records, &[], &PublicationPolicy::default()).unwrap();
        assert_eq!(draft.turn_count(), 3);
        assert_eq!(
            draft.source_refs,
            vec![
                "codex/session/1".to_string(),
                "codex/session/2".to_string(),
                "codex/session/3".to_string(),
            ]
        );
        let PublicConversationSnapshot::V1(snapshot) = &draft.snapshot else {
            panic!("whole records use the v1 snapshot")
        };
        let assistant: Vec<_> = snapshot
            .items
            .iter()
            .filter(|item| item.role == PublicRole::Assistant)
            .map(|item| item.text.as_str())
            .collect();
        assert_eq!(assistant, ["a", "b", "c"]);
    }

    #[test]
    fn granular_snapshot_keeps_parts_and_marks_omitted_content() {
        let record = granular_record(1);
        let anchors = [1, 2, 4]
            .into_iter()
            .map(|seq| record.work_ref.with_part(seq))
            .collect::<Vec<_>>();
        let draft =
            create_publication_draft(&[record], &anchors, &PublicationPolicy::default()).unwrap();
        let PublicConversationSnapshot::V2(snapshot) = &draft.snapshot else {
            panic!("part anchors use the v2 snapshot")
        };
        assert_eq!(snapshot.schema_version, GRANULAR_PUBLICATION_SCHEMA_VERSION);
        assert_eq!(snapshot.items.len(), 3);
        assert_eq!(snapshot.items[0].kind, PublicEntryKind::User);
        assert_eq!(snapshot.items[1].kind, PublicEntryKind::Tool);
        assert_eq!(snapshot.items[1].parts.len(), 1);
        assert!(snapshot.items[2].gap_before);
        assert_eq!(draft.turn_count(), 1);
        let json = serde_json::to_string(&draft.snapshot).unwrap();
        assert!(!json.contains("work_ref"));
        assert!(!json.contains("session"));
        assert!(!json.contains("sk-abcdefghijklmnop"));
        assert!(json.contains("[REDACTED]"));
        // The record cwd is structural context and never enters the snapshot.
        assert!(!json.contains("C:"));

        let prefix_omitted = [2, 4]
            .into_iter()
            .map(|seq| granular_record(1).work_ref.with_part(seq))
            .collect::<Vec<_>>();
        let record = granular_record(1);
        let prefix_draft = create_publication_draft(
            std::slice::from_ref(&record),
            &prefix_omitted,
            &PublicationPolicy::default(),
        )
        .unwrap();
        let PublicConversationSnapshot::V2(snapshot) = &prefix_draft.snapshot else {
            panic!("part anchors use the v2 snapshot")
        };
        assert!(snapshot.items[0].gap_before);

        let fixed_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let later = create_publication_draft(
            std::slice::from_ref(&record),
            &anchors,
            &PublicationPolicy {
                published_at: Some(fixed_time),
                ..PublicationPolicy::default()
            },
        )
        .unwrap();
        assert_eq!(draft.content_sha256, later.content_sha256);
    }

    #[test]
    fn granular_snapshot_matches_when_unselected_records_are_absent() {
        let first = granular_record(1);
        let selected = granular_record(2);
        let last = granular_record(3);
        let anchors = [1, 4]
            .into_iter()
            .map(|seq| selected.work_ref.with_part(seq))
            .collect::<Vec<_>>();
        let policy = PublicationPolicy {
            published_at: Some(
                DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            ..PublicationPolicy::default()
        };
        let full = create_publication_draft(
            &[first.clone(), selected.clone(), last.clone()],
            &anchors,
            &policy,
        )
        .unwrap();
        let slim =
            create_publication_draft(std::slice::from_ref(&selected), &anchors, &policy).unwrap();
        let PublicConversationSnapshot::V2(full_snapshot) = &full.snapshot else {
            panic!("part anchors use the v2 snapshot")
        };
        let PublicConversationSnapshot::V2(slim_snapshot) = &slim.snapshot else {
            panic!("part anchors use the v2 snapshot")
        };
        assert_eq!(full_snapshot.items, slim_snapshot.items);
        assert!(full_snapshot.items[0].gap_before);
        assert_eq!(full.content_sha256, slim.content_sha256);

        let skip_middle = vec![first.work_ref.with_part(1), last.work_ref.with_part(4)];
        let full_skip =
            create_publication_draft(&[first, selected, last.clone()], &skip_middle, &policy)
                .unwrap();
        let slim_skip =
            create_publication_draft(&[granular_record(1), last], &skip_middle, &policy).unwrap();
        let PublicConversationSnapshot::V2(full_skip_snapshot) = &full_skip.snapshot else {
            panic!("part anchors use the v2 snapshot")
        };
        let PublicConversationSnapshot::V2(slim_skip_snapshot) = &slim_skip.snapshot else {
            panic!("part anchors use the v2 snapshot")
        };
        assert_eq!(full_skip_snapshot.items, slim_skip_snapshot.items);
        assert!(slim_skip_snapshot.items[1].gap_before);
        assert_eq!(full_skip.content_sha256, slim_skip.content_sha256);
    }

    #[test]
    fn granular_snapshot_title_comes_from_selected_public_parts() {
        let mut record = granular_record(1);
        record.title = "secret user prompt".into();
        let anchors = [record.work_ref.with_part(4)];
        let draft = create_publication_draft(
            std::slice::from_ref(&record),
            &anchors,
            &PublicationPolicy::default(),
        )
        .unwrap();
        let PublicConversationSnapshot::V2(snapshot) = &draft.snapshot else {
            panic!("part anchors use the v2 snapshot")
        };
        assert_eq!(snapshot.title, "reply");
        assert!(!snapshot.title.contains("secret"));
    }

    #[test]
    fn granular_snapshot_rejects_mixed_anchors_and_scopes() {
        let record = granular_record(1);
        let mixed = [record.work_ref.whole(), record.work_ref.with_part(4)];
        assert!(
            create_publication_draft(&[record], &mixed, &PublicationPolicy::default()).is_err()
        );

        let local = granular_record(1);
        let mut remote = local.clone();
        remote.work_ref = remote.work_ref.with_named_scope("peer");
        assert!(create_publication_draft(
            std::slice::from_ref(&remote),
            &[remote.work_ref.with_part(1)],
            &PublicationPolicy::default()
        )
        .is_err());

        let mut terminal = local.clone();
        terminal.work_ref = WorkRef::terminal("session", 1);
        terminal.kind = WorkRecordKind::TerminalCommand;
        terminal.source.channel = WorkChannel::Terminal;
        terminal.source.provider = None;
        assert!(create_publication_draft(
            std::slice::from_ref(&terminal),
            &[terminal.work_ref.with_part(1)],
            &PublicationPolicy::default()
        )
        .is_err());

        let mut other_provider = granular_record(2);
        other_provider.work_ref =
            WorkRef::agent(crate::agents::AgentProvider::Claude, "session", 2);
        other_provider.source.provider = Some("claude".into());
        let cross = [
            local.work_ref.with_part(1),
            other_provider.work_ref.with_part(1),
        ];
        assert!(create_publication_draft(
            &[local, other_provider],
            &cross,
            &PublicationPolicy::default()
        )
        .is_err());
    }

    #[test]
    fn publication_expiry_parses_picker_choices() {
        assert_eq!(
            PublicationExpiry::parse("2h").unwrap(),
            PublicationExpiry::TwoHours
        );
        assert_eq!(
            PublicationExpiry::parse("3d").unwrap(),
            PublicationExpiry::ThreeDays
        );
        assert_eq!(PublicationExpiry::TwoHours.as_str(), "2h");
        assert_eq!(PublicationExpiry::ThreeDays.as_str(), "3d");
        assert_eq!(PublicationExpiry::TwoHours.duration(), Duration::hours(2));
        assert_eq!(PublicationExpiry::ThreeDays.duration(), Duration::days(3));
        assert_eq!(
            PublicationExpiry::PICKER_CHOICES[PublicationExpiry::picker_default_index()],
            PublicationExpiry::SevenDays
        );
        assert!(PublicationExpiry::parse("4h").is_err());
        assert!(PublicationExpiry::parse("90d").is_err());
    }
}
