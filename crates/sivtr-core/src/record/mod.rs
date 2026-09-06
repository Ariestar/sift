pub mod expand;
pub mod index;
pub mod model;
pub mod refs;

pub use expand::{expand_source, resolve_scope_token};
pub use index::WorkRecordIndex;
pub use model::{
    agent_parts, chat_turn_ranges, format_shell_action, format_work_part, is_real_user_block,
    output_blocks_text, MessageRole, Projection, ProjectionSlice, RecordText, RecordTextMode,
    WorkActionStatus, WorkActor, WorkContent, WorkContentBlock, WorkOutcome, WorkPart,
    WorkPartBody, WorkPartKind, WorkRecord, WorkRecordCopyParts, WorkSessionRef, WorkStatus,
    WorkTarget, WorkTime, RECORD_SCHEMA_VERSION,
};
pub use refs::{normalize_scope_name, WorkAt, WorkPath, WorkRef, WorkRefSelector, WorkScope};
