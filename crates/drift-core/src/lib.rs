mod config;
mod error;
mod path;
mod registry;
mod status;

pub use config::{parse_user_roots, registry_from_user_config, USER_CONFIG_PATH};
pub use error::Error;
pub use path::RelPath;
pub use registry::{
    Mapping, Origin, Registry, Root, BUILTIN_CONFIG_ROOT, VOLATILE_CHROMIUM_PREFERENCES,
};
pub use status::{
    content_treatment, Badge, BaseProvenance, HashOnlyReason, Status, Treatment, MAX_DIFF_BYTES,
};
