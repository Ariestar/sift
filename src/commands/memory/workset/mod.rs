mod source;
pub mod store;

pub(crate) use source::{
    load_context_records, query, query_sources, run_on_share, QuerySource, QuerySourceResult,
};
pub(crate) use store::{cleanup_saved, delete_saved, list_saved, load_saved, save_named};

use anyhow::{bail, Context, Result};
use sivtr_core::record::WorkRef;
use std::collections::HashSet;

pub use sivtr_core::workset::{
    find_record, records_for_anchors, require_record, WorkSelectionAction, WorkSelectionKind,
    WorkSelectionTarget, WorkSet, WORKSET_SCHEMA_VERSION,
};

fn apply_selection(mut set: WorkSet, selection: WorkSetSelection) -> WorkSet {
    let WorkSetSelection::Indices(indices) = selection else {
        return set;
    };

    let anchors = indices
        .into_iter()
        .map(|index| set.anchors()[index - 1].clone())
        .collect::<Vec<_>>();
    set.select_anchors(anchors);
    set
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkSetSelection {
    All,
    Indices(Vec<usize>),
}

/// Persist a named WorkSet (`@name`). `materialize_parts` runs first so a
/// light-loaded set saves complete records; [`WorkSet::validate`] rejects
/// malformed sets before they hit disk.
pub(crate) fn save_as(set: &mut WorkSet, name: &str) -> Result<()> {
    store::validate_name(name)?;
    set.materialize_parts()?;
    set.validate()?;
    set.name = Some(name.to_string());
    save_named(name, set).with_context(|| format!("save WorkSet @{name}"))
}

/// Persist the `@last` WorkSet.
pub(crate) fn save_last(set: &WorkSet) -> Result<()> {
    let mut set = set.clone();
    set.materialize_parts()?;
    set.validate()?;
    save_named("last", &set).context("save @last WorkSet")
}

pub(crate) fn persist(set: &mut WorkSet, name: Option<&str>) -> Result<()> {
    save_last(set)?;
    if let Some(name) = name {
        save_as(set, name)?;
    }
    Ok(())
}

pub(crate) fn unique_anchors(anchors: Vec<WorkRef>) -> Vec<WorkRef> {
    let mut seen = HashSet::new();
    anchors
        .into_iter()
        .filter(|anchor| seen.insert(anchor.to_string()))
        .collect()
}

pub fn load_reference(reference: &str) -> Result<WorkSet> {
    let parsed = parse_reference(reference)?;
    let set = load_saved(parsed.name)?;
    validate_selection(reference, &set, &parsed.selection)?;
    Ok(apply_selection(set, parsed.selection))
}

struct ParsedWorkSetReference<'a> {
    name: &'a str,
    selection: WorkSetSelection,
}

fn parse_reference(reference: &str) -> Result<ParsedWorkSetReference<'_>> {
    let body = reference
        .strip_prefix('@')
        .ok_or_else(|| anyhow::anyhow!("WorkSet reference must start with @"))?;
    if let Some(open) = body.find('[') {
        if !body.ends_with(']') {
            bail!("Invalid WorkSet reference `{reference}`; missing closing ]");
        }
        let name = &body[..open];
        store::validate_name(name)?;
        let selector = &body[open + 1..body.len() - 1];
        if selector.is_empty() {
            bail!("Invalid WorkSet reference `{reference}`");
        }
        let selection = parse_selector(selector, reference)?;
        Ok(ParsedWorkSetReference { name, selection })
    } else {
        store::validate_name(body)?;
        Ok(ParsedWorkSetReference {
            name: body,
            selection: WorkSetSelection::All,
        })
    }
}

fn parse_selector(selector: &str, reference: &str) -> Result<WorkSetSelection> {
    let mut indices = Vec::new();
    for segment in selector.split(',') {
        if segment.is_empty() {
            bail!("Invalid WorkSet reference `{reference}`; empty selector segment");
        }
        if let Some((start, end)) = segment.split_once("..") {
            let start = parse_index(start, reference)?;
            let end = parse_index(end, reference)?;
            if start > end {
                bail!("Invalid WorkSet reference `{reference}`; range start must be <= end");
            }
            indices.extend(start..=end);
        } else {
            indices.push(parse_index(segment, reference)?);
        }
    }
    Ok(WorkSetSelection::Indices(indices))
}

fn parse_index(value: &str, reference: &str) -> Result<usize> {
    let index = value.parse::<usize>().with_context(|| {
        format!("Invalid WorkSet reference `{reference}`; index must be a positive integer")
    })?;
    if index == 0 {
        bail!("Invalid WorkSet reference `{reference}`; index must be 1-based");
    }
    Ok(index)
}

fn validate_selection(reference: &str, set: &WorkSet, selection: &WorkSetSelection) -> Result<()> {
    match selection {
        WorkSetSelection::All => Ok(()),
        WorkSetSelection::Indices(indices) => {
            for index in indices {
                if *index > set.anchors().len() {
                    bail!(
                        "Invalid WorkSet reference `{reference}`; index {index} exceeds WorkSet length {}",
                        set.anchors().len()
                    );
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::record::{MessageRole, WorkRecord, WorkSessionRef, WorkTime};

    fn record(index: usize) -> WorkRecord {
        WorkRecord {
            schema_version: sivtr_core::record::RECORD_SCHEMA_VERSION,
            work_ref: format!("terminal/session_1/{index}")
                .parse()
                .expect("valid work ref"),
            session: WorkSessionRef {
                id: "session_1".to_string(),
                canonical_id: Some("session_1".to_string()),
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: None,
            title: format!("record {index}"),
            parts: (1..=2)
                .map(|seq| {
                    crate::test_fixtures::message_part(
                        seq,
                        MessageRole::Assistant,
                        &format!("record {index} part {seq}"),
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn parses_discrete_and_range_selectors_in_order() {
        let selection = parse_selector("1,3..5,2", "@hits[1,3..5,2]").expect("selector parses");
        assert_eq!(selection, WorkSetSelection::Indices(vec![1, 3, 4, 5, 2]));
    }

    #[test]
    fn selected_keeps_discrete_selector_order() {
        let set = WorkSet::new(".", (1..=5).map(record).collect());
        let selected = apply_selection(set, WorkSetSelection::Indices(vec![3, 1, 5]));

        let refs = selected
            .anchors()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert_eq!(
            refs,
            vec![
                "terminal/session_1/3",
                "terminal/session_1/1",
                "terminal/session_1/5"
            ]
        );
    }

    #[test]
    fn selected_keeps_part_anchor_order() {
        let records = vec![record(1), record(2)];
        let anchors = vec![
            records[1].work_ref.with_part(1),
            records[0].work_ref.with_part(1),
        ];
        let set = WorkSet::from_parts(".", records, anchors);
        let selected = apply_selection(set, WorkSetSelection::Indices(vec![2, 1]));

        let refs = selected
            .anchors()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert_eq!(
            refs,
            vec!["terminal/session_1/1/p1", "terminal/session_1/2/p1"]
        );
    }

    #[test]
    fn from_parts_canonicalizes_duplicate_and_shadowed_anchors() {
        let first = record(1);
        let part = first.work_ref.with_part(1);
        let set = WorkSet::from_parts(
            ".",
            vec![first.clone()],
            vec![
                part.clone(),
                part,
                first.work_ref.whole(),
                first.work_ref.with_part(2),
            ],
        );
        assert_eq!(set.anchors(), &[first.work_ref.whole()]);
    }

    #[test]
    fn validation_rejects_shadowed_anchors_loaded_from_json() {
        let first = record(1);
        let mut value = serde_json::to_value(WorkSet::from_parts(
            ".",
            vec![first.clone()],
            vec![first.work_ref.whole()],
        ))
        .expect("serialize WorkSet");
        value["anchors"] = serde_json::json!([first.work_ref.whole(), first.work_ref.with_part(1)]);
        let set: WorkSet = serde_json::from_value(value).expect("deserialize WorkSet");
        assert!(set.validate().is_err());
    }

    #[test]
    fn validation_rejects_part_before_whole_loaded_from_json() {
        let first = record(1);
        let mut value = serde_json::to_value(WorkSet::from_parts(
            ".",
            vec![first.clone()],
            vec![first.work_ref.whole()],
        ))
        .expect("serialize WorkSet");
        value["anchors"] = serde_json::json!([first.work_ref.with_part(1), first.work_ref.whole()]);
        let set: WorkSet = serde_json::from_value(value).expect("deserialize WorkSet");
        assert!(set.validate().is_err());
    }

    #[test]
    fn validation_rejects_anchor_without_backing_record() {
        let first = record(1);
        let mut value = serde_json::to_value(WorkSet::from_parts(".", vec![first.clone()], vec![]))
            .expect("serialize WorkSet");
        value["anchors"] = serde_json::json!([record(2).work_ref]);
        let set: WorkSet = serde_json::from_value(value).expect("deserialize WorkSet");
        assert!(set.validate().is_err());
    }

    #[test]
    fn validation_rejects_part_anchor_without_matching_part() {
        let first = record(1);
        let mut value = serde_json::to_value(WorkSet::from_parts(".", vec![first.clone()], vec![]))
            .expect("serialize WorkSet");
        value["anchors"] = serde_json::json!([first.work_ref.with_part(99)]);
        let set: WorkSet = serde_json::from_value(value).expect("deserialize WorkSet");
        assert!(set.validate().is_err());
    }

    #[test]
    fn whole_anchor_covers_and_replaces_parts() {
        let first = record(1);
        let mut set = WorkSet::from_parts(".", vec![first.clone()], vec![]);
        set.apply_target(
            WorkSelectionAction::Include,
            WorkSelectionTarget::Parts {
                record: first.clone(),
                parts: vec![1],
            },
            [],
        );
        assert!(set.contains(&first.work_ref.with_part(1)));
        set.apply_target(
            WorkSelectionAction::Include,
            WorkSelectionTarget::Whole(first.clone()),
            [],
        );
        assert_eq!(set.anchors(), vec![first.work_ref.whole()]);
        assert!(set.contains(&first.work_ref.with_part(1)));
    }

    #[test]
    fn part_anchors_deduplicate_without_collapsing() {
        let first = record(1);
        let mut set = WorkSet::new(".", Vec::new());
        set.apply_target(
            WorkSelectionAction::Include,
            WorkSelectionTarget::Parts {
                record: first.clone(),
                parts: vec![1, 1],
            },
            [],
        );
        assert_eq!(set.anchors(), vec![first.work_ref.with_part(1)]);
    }

    #[test]
    fn complete_part_selection_is_canonical_whole() {
        let first = record(1);
        let mut set = WorkSet::new(".", Vec::new());
        set.apply_target(
            WorkSelectionAction::Include,
            WorkSelectionTarget::Parts {
                record: first.clone(),
                parts: vec![1, 2, 1],
            },
            [],
        );
        assert_eq!(set.anchors(), vec![first.work_ref.whole()]);
        assert!(set.contains(&first.work_ref.with_part(2)));
    }

    #[test]
    fn incremental_part_selection_is_canonical_whole() {
        let first = record(1);
        let mut set = WorkSet::new(".", Vec::new());
        for parts in [vec![1], vec![2]] {
            set.apply_target(
                WorkSelectionAction::Include,
                WorkSelectionTarget::Parts {
                    record: first.clone(),
                    parts,
                },
                [],
            );
        }
        assert_eq!(set.anchors(), vec![first.work_ref.whole()]);
    }

    #[test]
    fn from_parts_collapses_complete_part_anchors() {
        let first = record(1);
        let set = WorkSet::from_parts(
            ".",
            vec![first.clone()],
            vec![first.work_ref.with_part(1), first.work_ref.with_part(2)],
        );
        assert_eq!(set.anchors(), vec![first.work_ref.whole()]);
    }

    #[test]
    fn toggling_whole_replaces_narrow_selection() {
        let first = record(1);
        let mut set = WorkSet::new(".", Vec::new());
        set.apply_target(
            WorkSelectionAction::Include,
            WorkSelectionTarget::Parts {
                record: first.clone(),
                parts: vec![1],
            },
            [],
        );
        set.apply_target(
            WorkSelectionAction::Toggle,
            WorkSelectionTarget::Whole(first.clone()),
            [],
        );
        assert_eq!(set.anchors(), vec![first.work_ref.whole()]);
        set.apply_target(
            WorkSelectionAction::Toggle,
            WorkSelectionTarget::Whole(first.clone()),
            [],
        );
        assert!(set.anchors().is_empty());
    }

    #[test]
    fn toggling_part_from_whole_keeps_the_other_parts() {
        let first = record(1);
        let mut set = WorkSet::new(".", Vec::new());
        set.apply_target(
            WorkSelectionAction::Include,
            WorkSelectionTarget::Whole(first.clone()),
            [],
        );
        set.apply_target(
            WorkSelectionAction::Toggle,
            WorkSelectionTarget::Parts {
                record: first.clone(),
                parts: vec![1],
            },
            [],
        );
        assert_eq!(set.anchors(), vec![first.work_ref.with_part(2)]);
        assert!(!set.contains(&first.work_ref.with_part(1)));
        assert!(set.contains(&first.work_ref.with_part(2)));
    }

    #[test]
    fn scope_target_selects_all_matching_records() {
        let records = vec![record(1), record(2)];
        let target = WorkSelectionTarget::Scope {
            scope: sivtr_core::record::WorkScope::Local,
            kind: None,
            session: Some("session_1".to_string()),
        };
        let mut set = WorkSet::new(".", Vec::new());
        set.apply_target(WorkSelectionAction::Toggle, target, records.clone());
        assert_eq!(
            set.anchors(),
            &records
                .iter()
                .map(|r| r.work_ref.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn unique_anchors_preserves_first_occurrence() {
        let records = [record(1), record(2)];
        let anchors = vec![
            records[0].work_ref.whole(),
            records[1].work_ref.whole(),
            records[0].work_ref.whole(),
        ];

        let unique = unique_anchors(anchors)
            .into_iter()
            .map(|anchor| anchor.to_string())
            .collect::<Vec<_>>();

        assert_eq!(unique, vec!["terminal/session_1/1", "terminal/session_1/2"]);
    }

    #[test]
    fn rejects_empty_discrete_selector_segment() {
        let error = parse_selector("1,,2", "@hits[1,,2]").expect_err("selector rejects empty");
        assert!(error.to_string().contains("empty selector segment"));
    }
}
