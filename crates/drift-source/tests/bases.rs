use std::path::{Path, PathBuf};

use drift_core::RelPath;
use drift_fixtures::RootBuilder;
use drift_source::{
    base_path, drift_share, drift_state, read_base, read_reviewed_version, BASES, REVIEWED_VERSION,
};

#[test]
fn a_base_path_sits_under_the_share_directory() {
    let share = drift_share(Path::new("data"));
    let rel = RelPath::new("hypr/a.lua").unwrap();
    assert_eq!(share, PathBuf::from("data/drift"));
    assert_eq!(
        base_path(&share, "config", &rel),
        Some(
            PathBuf::from("data/drift")
                .join(BASES)
                .join("config/hypr/a.lua")
        )
    );
    assert_eq!(base_path(&share, "..", &rel), None);
    assert_eq!(base_path(&share, "config/nested", &rel), None);
    assert_eq!(
        drift_state(Path::new("state")),
        PathBuf::from("state/drift")
    );
}

#[test]
fn read_base_returns_regular_bytes_and_ignores_a_link() {
    let tree = RootBuilder::new()
        .file("share/bases/config/hypr/a.lua", b"x\n")
        .unwrap()
        .file("share/bases/applications/hypr/a.lua", b"hi\n")
        .unwrap()
        .file("secret", b"hidden\n")
        .unwrap()
        .link("share/bases/config/hypr/link.lua", "../../../../secret")
        .unwrap()
        .materialize()
        .unwrap();
    let share = tree.path().join("share");
    let rel = RelPath::new("hypr/a.lua").unwrap();
    assert_eq!(read_base(&share, "config", &rel).unwrap(), b"x\n");
    assert_eq!(read_base(&share, "applications", &rel).unwrap(), b"hi\n");
    let link = RelPath::new("hypr/link.lua").unwrap();
    assert_eq!(read_base(&share, "config", &link), None);
    assert_eq!(read_base(&share, "../secret", &rel), None);
    assert_eq!(
        std::fs::read(tree.path().join("secret")).unwrap(),
        b"hidden\n"
    );
}

#[test]
fn read_reviewed_version_accepts_one_token_and_rejects_the_rest() {
    assert_eq!(version_of(b"1.2.0-1\n").unwrap(), "1.2.0-1");
    assert_eq!(version_of(b"\n"), None);
    assert_eq!(version_of(b"1.2.0-1\nbad\n"), None);
    assert_eq!(version_of(b"1/2\n"), None);
    let tree = RootBuilder::new()
        .file("secret", b"hidden\n")
        .unwrap()
        .link(REVIEWED_VERSION, "secret")
        .unwrap()
        .materialize()
        .unwrap();
    assert_eq!(read_reviewed_version(tree.path()), None);
    assert_eq!(
        std::fs::read(tree.path().join("secret")).unwrap(),
        b"hidden\n"
    );
}

fn version_of(bytes: &[u8]) -> Option<String> {
    let tree = RootBuilder::new()
        .file(REVIEWED_VERSION, bytes)
        .unwrap()
        .materialize()
        .unwrap();
    read_reviewed_version(tree.path())
}
