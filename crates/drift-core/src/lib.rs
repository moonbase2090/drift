mod config;
mod error;
mod file;
mod merge;
mod path;
mod registry;
mod rows;
mod status;

pub use config::{parse_user_roots, registry_from_user_config, USER_CONFIG_PATH};
pub use error::{Error, Location};
pub use file::{LiveFile, LiveState, LiveTreatment, Template, Verification};
pub use merge::{merge3, Choice, Hunk, HunkKind, Merge, Segment};
pub use path::RelPath;
pub use registry::{
    Mapping, Origin, Registry, Root, BUILTIN_CONFIG_ROOT, VOLATILE_CHROMIUM_PREFERENCES,
};
pub use rows::{
    DiffRows, LineEnding, LineNumbers, Mark, Row, RowCache, Span, MAX_SPAN_GRAPHEMES, TAB_WIDTH,
};
pub use status::{
    content_treatment, Badge, BaseProvenance, HashOnlyReason, Status, Treatment, MAX_DIFF_BYTES,
};
