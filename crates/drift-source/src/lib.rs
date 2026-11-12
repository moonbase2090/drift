mod bases;
mod catalog;
mod error;
mod installed;
mod parse;

pub use bases::{
    base_path, drift_share, drift_state, read_base, read_reviewed_version, reviewed_version_path,
    BASES, REVIEWED_VERSION,
};
pub use catalog::{
    enumerate, is_backup_name, scan, BackupGroup, DirReader, Listing, SourcePaths, StdDirReader,
    Tracked,
};
pub use error::Error;
pub use installed::{
    read_package, Check, InstalledPackage, ReadPaths, Template, Upgrade, DEFAULT_OMARCHY_PATH,
    PACKAGE_NAME,
};

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_its_name() {
        assert!(crate_name().starts_with("drift-"));
    }
}
