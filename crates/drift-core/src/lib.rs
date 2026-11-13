mod classify;
mod config;
mod error;
mod file;
mod file_diff;
mod integrate;
mod merge;
mod path;
mod plan;
mod plan_view;
mod registry;
mod report;
mod resolve_plan;
mod rows;
mod status;
mod tree_view;

pub use classify::{classify, Base, Classification, MergeOutcome, Versions};
pub use config::{parse_user_roots, registry_from_user_config, USER_CONFIG_PATH};
pub use error::{Error, Location};
pub use file::{LiveFile, LiveState, LiveTreatment, Template, Verification};
pub use file_diff::{DiffContent, FileDiff};
pub use integrate::{
    add_menu_row, hook_content, unified_diff, HOOK_MODE, HOOK_PATH, MENU_ID, MENU_PATH,
};
pub use merge::{merge3, Choice, Hunk, HunkKind, Merge, Segment};
pub use path::RelPath;
pub use plan::{Action, ObservedFile, Plan, PlannedFile};
pub use plan_view::{DiffSummary, FileContents, FileView, HashOnly, PlanView, MAX_DIFF_LINES};
pub use registry::{
    Mapping, Origin, Registry, Root, BUILTIN_CONFIG_ROOT, VOLATILE_CHROMIUM_PREFERENCES,
};
pub use report::{counts_as_drift, BackupSummary, ScanEntry, ScanReport, SCHEMA};
pub use resolve_plan::{
    prepare, BaseRecord, Prepared, ResolveInput, Skip, SkipReason, Write as PlannedWrite,
};
pub use rows::{
    DiffRows, LineEnding, LineNumbers, Mark, Row, RowCache, Span, MAX_SPAN_GRAPHEMES, TAB_WIDTH,
};
pub use status::{
    content_treatment, Badge, BaseProvenance, HashOnlyReason, Status, Treatment, MAX_DIFF_BYTES,
};
pub use tree_view::{RowKind, TreeRow, TreeView};
