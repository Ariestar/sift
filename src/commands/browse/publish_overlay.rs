//! Publication lifetime picker for the workspace browser.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use sivtr_core::publication::{PublicationDraft, PublicationExpiry};
use sivtr_core::workset::WorkSet;

pub(super) struct PublishOverlay {
    pub(super) selected: usize,
    pub(super) name: String,
    pub(super) focus: OverlayFocus,
    pub(super) set: WorkSet,
    pub(super) draft: PublicationDraft,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OverlayFocus {
    Name,
    Expiry,
}

pub(super) enum OverlayKey {
    Continue,
    Cancel,
    Confirm,
}

pub(super) fn selected_expiry(selected: usize) -> PublicationExpiry {
    PublicationExpiry::PICKER_CHOICES[selected]
}

pub(super) fn handle_key(
    key: KeyEvent,
    selected: &mut usize,
    name: &mut String,
    focus: &mut OverlayFocus,
) -> OverlayKey {
    let last = PublicationExpiry::PICKER_CHOICES.len().saturating_sub(1);
    match key.code {
        KeyCode::Esc => OverlayKey::Cancel,
        KeyCode::Enter => OverlayKey::Confirm,
        KeyCode::Tab | KeyCode::BackTab => {
            *focus = match focus {
                OverlayFocus::Name => OverlayFocus::Expiry,
                OverlayFocus::Expiry => OverlayFocus::Name,
            };
            OverlayKey::Continue
        }
        KeyCode::Up if *focus == OverlayFocus::Expiry => {
            *selected = selected.saturating_sub(1);
            OverlayKey::Continue
        }
        KeyCode::Char('k') if *focus == OverlayFocus::Expiry => {
            *selected = selected.saturating_sub(1);
            OverlayKey::Continue
        }
        KeyCode::Down if *focus == OverlayFocus::Expiry => {
            *selected = (*selected + 1).min(last);
            OverlayKey::Continue
        }
        KeyCode::Char('j') if *focus == OverlayFocus::Expiry => {
            *selected = (*selected + 1).min(last);
            OverlayKey::Continue
        }
        KeyCode::Backspace if *focus == OverlayFocus::Name => {
            name.pop();
            OverlayKey::Continue
        }
        KeyCode::Char(ch)
            if *focus == OverlayFocus::Name
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            name.push(ch);
            OverlayKey::Continue
        }
        _ => OverlayKey::Continue,
    }
}

pub(super) fn new(
    set: WorkSet,
    draft: PublicationDraft,
    expires: PublicationExpiry,
) -> PublishOverlay {
    let selected = PublicationExpiry::PICKER_CHOICES
        .iter()
        .position(|choice| *choice == expires)
        .expect("publish expiry must be offered in the overlay");
    PublishOverlay {
        selected,
        name: String::new(),
        focus: OverlayFocus::Expiry,
        set,
        draft,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::message_part;
    use sivtr_core::agents::AgentProvider;
    use sivtr_core::record::{
        MessageRole, WorkRecord, WorkRef, WorkSessionRef, WorkTime, RECORD_SCHEMA_VERSION,
    };

    fn record() -> WorkRecord {
        WorkRecord {
            schema_version: RECORD_SCHEMA_VERSION,
            work_ref: WorkRef::agent(AgentProvider::Codex, "session", 1),
            session: WorkSessionRef {
                id: "session".into(),
                canonical_id: None,
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: None,
            title: "Demo".into(),
            parts: vec![
                message_part(1, MessageRole::User, "hello"),
                message_part(2, MessageRole::Assistant, "reply"),
            ],
        }
    }

    #[test]
    fn handle_key_moves_and_confirms_default_seven_days() {
        let mut selected = PublicationExpiry::picker_default_index();
        assert_eq!(selected_expiry(selected), PublicationExpiry::SevenDays);
        let mut name = String::new();
        let mut focus = OverlayFocus::Expiry;
        assert!(matches!(
            handle_key(
                KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
                &mut selected,
                &mut name,
                &mut focus,
            ),
            OverlayKey::Continue
        ));
        assert_eq!(selected_expiry(selected), PublicationExpiry::ThreeDays);
        assert!(matches!(
            handle_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &mut selected,
                &mut name,
                &mut focus,
            ),
            OverlayKey::Confirm
        ));
        assert!(matches!(
            handle_key(
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                &mut selected,
                &mut name,
                &mut focus,
            ),
            OverlayKey::Cancel
        ));
    }

    #[test]
    fn name_input_is_optional_and_confirm_keeps_the_entered_name() {
        let mut selected = PublicationExpiry::picker_default_index();
        let mut name = String::new();
        let mut focus = OverlayFocus::Expiry;
        assert!(matches!(
            handle_key(
                KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
                &mut selected,
                &mut name,
                &mut focus,
            ),
            OverlayKey::Continue
        ));
        assert_eq!(focus, OverlayFocus::Name);
        assert!(matches!(
            handle_key(
                KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE),
                &mut selected,
                &mut name,
                &mut focus,
            ),
            OverlayKey::Continue
        ));
        assert_eq!(focus, OverlayFocus::Expiry);
        assert!(matches!(
            handle_key(
                KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
                &mut selected,
                &mut name,
                &mut focus,
            ),
            OverlayKey::Continue
        ));
        assert_eq!(focus, OverlayFocus::Name);
        for ch in ['r', 'e', 'v', 'i', 'e', 'w', 'k'] {
            assert!(matches!(
                handle_key(
                    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
                    &mut selected,
                    &mut name,
                    &mut focus,
                ),
                OverlayKey::Continue
            ));
        }
        assert_eq!(name, "reviewk");
        assert!(matches!(
            handle_key(
                KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
                &mut selected,
                &mut name,
                &mut focus,
            ),
            OverlayKey::Continue
        ));
        assert_eq!(name, "review");
        assert!(matches!(
            handle_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &mut selected,
                &mut name,
                &mut focus,
            ),
            OverlayKey::Confirm
        ));
    }

    #[test]
    fn empty_name_confirms_without_a_save_request() {
        let mut selected = PublicationExpiry::picker_default_index();
        let mut name = String::new();
        let mut focus = OverlayFocus::Expiry;
        assert!(matches!(
            handle_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &mut selected,
                &mut name,
                &mut focus,
            ),
            OverlayKey::Confirm
        ));
        assert!(name.is_empty());
    }

    #[test]
    fn new_uses_the_canonical_part_selection() {
        let record = record();
        let selection = WorkSet::from_parts(
            ".",
            vec![record.clone()],
            vec![record.work_ref.with_part(1)],
        );
        let (set, draft) = crate::commands::publish::prepare_picker(
            &selection,
            None,
            PublicationExpiry::SevenDays,
        )
        .expect("publication preparation");
        let overlay = new(set, draft, PublicationExpiry::SevenDays);
        assert_eq!(overlay.draft.snapshot.schema_version(), 2);
        assert_eq!(overlay.draft.item_count(), 1);
        assert_eq!(overlay.set.anchors(), &[record.work_ref.with_part(1)]);
        assert_eq!(overlay.focus, OverlayFocus::Expiry);
        assert!(overlay.name.is_empty());

        let (set, draft) = crate::commands::publish::prepare_picker(
            &selection,
            Some("title"),
            PublicationExpiry::ThreeDays,
        )
        .expect("publication preparation with configured expiry");
        let overlay = new(set, draft, PublicationExpiry::ThreeDays);
        assert_eq!(
            selected_expiry(overlay.selected),
            PublicationExpiry::ThreeDays
        );
    }

    #[test]
    fn prepare_picker_rejects_empty_selection() {
        let selection = WorkSet::from_parts(".", Vec::new(), Vec::new());
        assert!(crate::commands::publish::prepare_picker(
            &selection,
            None,
            PublicationExpiry::SevenDays,
        )
        .is_err());
    }
}
