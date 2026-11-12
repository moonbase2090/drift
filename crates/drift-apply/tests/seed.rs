use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;

use drift_apply::{scan, StoreRoots};
use drift_core::Registry;
use drift_fixtures::{hash_tree, RootBuilder};
use drift_source::{ReadPaths, SourcePaths};
use flate2::write::GzEncoder;
use flate2::Compression;

const VERSION: &str = "1.2.0-1";
const MARKER: &[u8] = b"x\n";
const MARKER_SHA: &str = "73cb3858a687a8494ca3323053016282f3dad39d42cf62ca4e79dda2aac7d9ac";
const OTHER_SHA: &str = "98ea6e4f216f2fb4b69fff9b3a44842c38686ca685f3f55dc48c5d3fb1107be4";

fn gzip(text: &str) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(text.as_bytes()).unwrap();
    encoder.finish().unwrap()
}

fn mtree(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut text = String::from("#mtree\n/set type=file uid=0 gid=0 mode=644\n");
    for (path, digest) in entries {
        text.push_str(&format!("./{path} time=1.0 size=2 sha256digest={digest}\n"));
    }
    gzip(&text)
}

fn desc(version: &str) -> String {
    format!("%NAME%\nomarchy-settings\n\n%VERSION%\n{version}\n")
}

fn file_list(paths: &[&str]) -> String {
    let mut text = String::from("%FILES%\n");
    for path in paths {
        text.push_str(path);
        text.push('\n');
    }
    text
}

fn package_path(name: &str) -> String {
    format!("usr/share/omarchy/config/hypr/{name}")
}

struct Tree {
    root: drift_fixtures::MaterializedRoot,
}

fn tree(fill: impl FnOnce(&mut RootBuilder)) -> Tree {
    let mut builder = RootBuilder::new();
    let dir = format!("var/lib/pacman/local/omarchy-settings-{VERSION}");
    let names = [
        "same.lua",
        "edited.lua",
        "link.lua",
        "unreadable.lua",
        "mismatch.lua",
        "skel.lua",
    ];
    let listed: Vec<String> = names.iter().copied().map(package_path).collect();
    let listed_refs: Vec<&str> = listed.iter().map(String::as_str).collect();
    let digests = [
        (package_path("same.lua"), MARKER_SHA),
        (package_path("edited.lua"), MARKER_SHA),
        (package_path("link.lua"), MARKER_SHA),
        (package_path("unreadable.lua"), MARKER_SHA),
        (package_path("mismatch.lua"), OTHER_SHA),
        (package_path("skel.lua"), MARKER_SHA),
    ];
    let digest_refs: Vec<(&str, &str)> = digests
        .iter()
        .map(|(path, digest)| (path.as_str(), *digest))
        .collect();
    builder
        .file(format!("{dir}/desc"), desc(VERSION))
        .unwrap()
        .file(format!("{dir}/files"), file_list(&listed_refs))
        .unwrap()
        .file(format!("{dir}/mtree"), mtree(&digest_refs))
        .unwrap();
    fill(&mut builder);
    Tree {
        root: builder.materialize().unwrap(),
    }
}

fn paths(tree: &Tree) -> SourcePaths {
    SourcePaths::new(
        ReadPaths::new(tree.root.path(), tree.root.path().join("usr/share/omarchy")),
        tree.root.path().join("home"),
        tree.root.path().join("skel"),
    )
}

fn roots(tree: &Tree) -> StoreRoots {
    StoreRoots {
        share: tree.root.path().join("share"),
        state: tree.root.path().join("state"),
    }
}

fn mode_of(path: &std::path::Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
}

fn config_digest(home: &std::path::Path) -> drift_fixtures::TreeDigest {
    let unreadable = home.join(".config/hypr/unreadable.lua");
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).unwrap();
    let digest = hash_tree(home).unwrap();
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();
    digest
}

#[test]
fn the_first_scan_seeds_matching_templates_and_the_second_writes_nothing() {
    let built = tree(|builder| {
        for name in ["same.lua", "edited.lua", "link.lua", "mismatch.lua"] {
            builder.file(package_path(name), MARKER).unwrap();
        }
        builder
            .file_with_mode(package_path("unreadable.lua"), MARKER, 0o644)
            .unwrap();
        builder.file("home/.config/hypr/same.lua", MARKER).unwrap();
        builder
            .file("home/.config/hypr/edited.lua", b"hi\n")
            .unwrap();
        builder.file("outside/victim", b"victim\n").unwrap();
        builder
            .link("home/.config/hypr/link.lua", "../../../outside/victim")
            .unwrap();
        builder
            .file_with_mode("home/.config/hypr/unreadable.lua", MARKER, 0o000)
            .unwrap();
        builder
            .file("home/.config/hypr/mismatch.lua", MARKER)
            .unwrap();
        builder.file("home/.config/hypr/skel.lua", MARKER).unwrap();
        builder.file("skel/.config/hypr/skel.lua", MARKER).unwrap();
    });
    let home = built.root.path().join("home");
    let unreadable = home.join(".config/hypr/unreadable.lua");
    let before = config_digest(&home);
    let unreadable_meta = fs::symlink_metadata(&unreadable).unwrap();
    let first = scan(&Registry::v1(), &paths(&built), &roots(&built)).unwrap();
    assert_eq!(first.package_version, VERSION);
    assert_eq!(first.reviewed_version.as_deref(), Some(VERSION));
    assert_eq!(
        first.seeded,
        vec!["hypr/same.lua".to_string(), "hypr/skel.lua".to_string()]
    );
    let same = built.root.path().join("share/bases/hypr/same.lua");
    let skel = built.root.path().join("share/bases/hypr/skel.lua");
    assert_eq!(fs::read(&same).unwrap(), MARKER);
    assert_eq!(fs::read(&skel).unwrap(), MARKER);
    assert_eq!(mode_of(&same), 0o644);
    assert_eq!(mode_of(&skel), 0o644);
    for name in ["edited.lua", "link.lua", "unreadable.lua", "mismatch.lua"] {
        assert!(!built
            .root
            .path()
            .join("share/bases/hypr")
            .join(name)
            .exists());
    }
    assert_eq!(
        fs::read(built.root.path().join("outside/victim")).unwrap(),
        b"victim\n"
    );
    let unreadable_after = fs::symlink_metadata(&unreadable).unwrap();
    assert_eq!(mode_of(&unreadable), 0o000);
    assert_eq!(unreadable_after.len(), unreadable_meta.len());
    assert_eq!(
        unreadable_after.modified().unwrap(),
        unreadable_meta.modified().unwrap()
    );
    assert_eq!(config_digest(&home), before);
    let stamped = fs::symlink_metadata(&same).unwrap().modified().unwrap();
    fs::write(&same, b"tamper\n").unwrap();
    let tampered = fs::symlink_metadata(&same).unwrap().modified().unwrap();
    let second = scan(&Registry::v1(), &paths(&built), &roots(&built)).unwrap();
    assert!(second.seeded.is_empty());
    assert_eq!(second.reviewed_version.as_deref(), Some(VERSION));
    assert_eq!(fs::read(&same).unwrap(), b"tamper\n");
    assert_eq!(
        fs::symlink_metadata(&same).unwrap().modified().unwrap(),
        tampered
    );
    assert_ne!(tampered, stamped);
    assert_eq!(config_digest(&home), before);
}

#[test]
fn a_read_only_state_directory_still_scans_and_seeds_nothing() {
    let built = tree(|builder| {
        builder.file(package_path("same.lua"), MARKER).unwrap();
        builder.file("home/.config/hypr/same.lua", MARKER).unwrap();
    });
    let home = built.root.path().join("home");
    let before = hash_tree(&home).unwrap();
    let state = built.root.path().join("state");
    fs::create_dir(&state).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o555)).unwrap();
    let report = scan(&Registry::v1(), &paths(&built), &roots(&built)).unwrap();
    assert!(report.files.iter().any(|file| file
        .template
        .as_ref()
        .is_some_and(|template| { template.mapping().rel.as_str() == "hypr/same.lua" })));
    assert_eq!(report.reviewed_version, None);
    assert!(report.seeded.is_empty());
    assert!(!built.root.path().join("share/bases/hypr/same.lua").exists());
    assert_eq!(hash_tree(&home).unwrap(), before);
    fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();
}
