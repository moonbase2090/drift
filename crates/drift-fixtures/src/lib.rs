mod case;
mod cpu_time;
mod error;
mod hash;
mod root;
mod tree;

pub use case::{
    case_dirs, fixtures_dir, load_case, materialize_case, Case, Expected, ExpectedFile,
    MaterializedCase, Presence,
};
pub use cpu_time::{busy_for, cpu_cost, thread_cpu_time};
pub use error::Error;
pub use hash::{hash_tree, TreeDigest};
pub use root::{MaterializedRoot, RootBuilder};
pub use tree::{Entry, FixtureBuilder, FixtureTrees, Node, Root};

pub const LIVE_CONFIG: &str = ".config";
pub const REVIEWED_BASES: &str = ".local/share/drift/bases/config";
pub const PACKAGE_TEMPLATES: &str = "config";

pub const STATUSES: &[&str] = &[
    "unchanged",
    "upstream changed",
    "local override",
    "conflict",
    "user-only",
    "missing",
];

pub const BADGES: &[&str] = &["merges cleanly", "base unknown", "deleted by you"];
