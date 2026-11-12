use std::fs;
use std::path::{Path, PathBuf};

use drift_core::RelPath;

pub const BASES: &str = "bases";
pub const REVIEWED_VERSION: &str = "reviewed-version";

pub fn drift_share(data_home: &Path) -> PathBuf {
    data_home.join("drift")
}

pub fn drift_state(state_home: &Path) -> PathBuf {
    state_home.join("drift")
}

pub fn base_path(share: &Path, root: &str, rel: &RelPath) -> Option<PathBuf> {
    if !root_segment(root) {
        return None;
    }
    let dest = rel.under(&share.join(BASES).join(root));
    dest.starts_with(share.join(BASES)).then_some(dest)
}

pub fn reviewed_version_path(state: &Path) -> PathBuf {
    state.join(REVIEWED_VERSION)
}

pub fn read_base(share: &Path, root: &str, rel: &RelPath) -> Option<Vec<u8>> {
    let path = base_path(share, root, rel)?;
    if !is_regular_file(&path) {
        return None;
    }
    fs::read(path).ok()
}

fn root_segment(root: &str) -> bool {
    !root.is_empty() && !root.contains('/') && !root.contains('\0') && root != "." && root != ".."
}

pub fn read_reviewed_version(state: &Path) -> Option<String> {
    let path = reviewed_version_path(state);
    if !is_regular_file(&path) {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    let version = text.trim();
    if version.is_empty()
        || version
            .chars()
            .any(|c| c.is_whitespace() || c == '/' || c == '\0')
    {
        return None;
    }
    Some(version.to_string())
}

fn is_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .ok()
        .is_some_and(|meta| meta.is_file())
}
