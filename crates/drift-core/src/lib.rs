mod classify;
mod config;
mod error;
mod file;
mod path;
mod plan;
mod registry;
mod status;

pub use classify::{classify, Base, Classification, MergeOutcome, Versions};
pub use config::{parse_user_roots, registry_from_user_config, USER_CONFIG_PATH};
pub use error::{Error, Location};
pub use file::{LiveFile, LiveState, LiveTreatment, Template, Verification};
pub use path::RelPath;
pub use plan::{Action, ObservedFile, Plan, PlannedFile};
pub use registry::{
    Mapping, Origin, Registry, Root, BUILTIN_CONFIG_ROOT, VOLATILE_CHROMIUM_PREFERENCES,
};
pub use status::{
    content_treatment, Badge, BaseProvenance, HashOnlyReason, Status, Treatment, MAX_DIFF_BYTES,
};
