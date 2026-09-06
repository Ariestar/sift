use super::model::WorkRecord;
use super::refs::WorkRef;

#[derive(Debug, Clone)]
pub struct WorkRecordIndex {
    records: Vec<WorkRecord>,
}

impl WorkRecordIndex {
    pub fn new(records: Vec<WorkRecord>) -> Self {
        Self { records }
    }

    pub fn records(&self) -> &[WorkRecord] {
        &self.records
    }

    pub fn resolve(&self, reference: &WorkRef) -> Option<&WorkRecord> {
        let whole = reference.whole();
        self.records.iter().find(|record| record.work_ref == whole)
    }

    pub fn resolve_part(&self, reference: &WorkRef) -> Option<&super::model::WorkPart> {
        let seq = reference.part()?;
        self.resolve(reference)
            .and_then(|record| find_part(record, seq))
    }
}

fn find_part(record: &WorkRecord, seq: usize) -> Option<&super::model::WorkPart> {
    record.parts.iter().find(|part| part.seq == seq)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::AgentProvider;
    use crate::record::model::{MessageRole, WorkSessionRef, WorkTime};
    use crate::test_fixtures::message_part;

    #[test]
    fn resolves_records_by_typed_ref() {
        let records = vec![test_record("pi/abcdef12/2", "abcdef12", 2, "hello\nneedle")];
        let index = WorkRecordIndex::new(records);
        let reference = WorkRef::agent(AgentProvider::Pi, "abcdef12", 2);

        assert_eq!(index.resolve(&reference).unwrap().title, "title");
        assert!(index
            .resolve(&WorkRef::agent(AgentProvider::Pi, "abcdef12", 3))
            .is_none());
    }

    #[test]
    fn resolves_parts_by_typed_ref() {
        let record = test_record_with_parts("terminal/current/1", "current", 1, "hello");
        let index = WorkRecordIndex::new(vec![record]);
        let reference = WorkRef::terminal("current", 1).with_part(1);

        assert!(index.resolve_part(&reference).is_some());
    }

    fn test_record(
        _ref_id: &str,
        session_id: &str,
        turn_index: usize,
        combined: &str,
    ) -> WorkRecord {
        let work_ref = WorkRef::agent(AgentProvider::Pi, session_id, turn_index);
        WorkRecord {
            schema_version: 1,
            work_ref,
            session: WorkSessionRef {
                id: session_id.to_string(),
                canonical_id: Some(session_id.to_string()),
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: None,
            title: "title".to_string(),
            parts: vec![message_part(1, MessageRole::Assistant, combined)],
        }
    }

    fn test_record_with_parts(
        _ref_id: &str,
        session_id: &str,
        turn_index: usize,
        text: &str,
    ) -> WorkRecord {
        let work_ref = WorkRef::terminal(session_id, turn_index);
        WorkRecord {
            schema_version: 1,
            work_ref,
            session: WorkSessionRef {
                id: session_id.to_string(),
                canonical_id: Some(session_id.to_string()),
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: None,
            title: "title".to_string(),
            parts: vec![message_part(1, MessageRole::Assistant, text)],
        }
    }
}
