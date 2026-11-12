use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use drift_core::{Registry, ScanReport};
use drift_fixtures::{
    case_dirs, fixtures_dir, hash_tree, materialize_case, MaterializedCase, RootBuilder,
};
use drift_source::{build_report, ReadPaths, ResolvePaths, ScanPaths, SourcePaths};
use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn gzip(text: &str) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(text.as_bytes()).unwrap();
    encoder.finish().unwrap()
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).unwrap();
        if meta.is_dir() {
            walk(&path, base, out);
        } else if meta.is_file() {
            let rel = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            out.push((rel, fs::read(&path).unwrap()));
        }
    }
}

fn package_root(version: &str, templates: &Path) -> drift_fixtures::MaterializedRoot {
    let mut files = Vec::new();
    walk(templates, templates, &mut files);
    files.sort();
    let mut listed = String::from("%FILES%\nusr/share/omarchy/config/\n");
    let mut mtree = String::from("#mtree\n/set type=file uid=0 gid=0 mode=644\n");
    for (rel, bytes) in &files {
        listed.push_str(&format!("usr/share/omarchy/config/{rel}\n"));
        mtree.push_str(&format!(
            "./usr/share/omarchy/config/{rel} time=1.0 size={} sha256digest={}\n",
            bytes.len(),
            digest(bytes)
        ));
    }
    let dir = format!("var/lib/pacman/local/omarchy-settings-{version}");
    let mut builder = RootBuilder::new();
    builder
        .file(
            format!("{dir}/desc"),
            format!("%NAME%\nomarchy-settings\n\n%VERSION%\n{version}\n"),
        )
        .unwrap()
        .file(format!("{dir}/files"), listed)
        .unwrap()
        .file(format!("{dir}/mtree"), gzip(&mtree))
        .unwrap();
    builder.materialize().unwrap()
}

struct Setup {
    case: MaterializedCase,
    package: drift_fixtures::MaterializedRoot,
}

fn setup(directory: &Path) -> Setup {
    let case = materialize_case(directory).unwrap();
    let package = package_root("4.5.0-3", &case.trees.template_dir());
    Setup { case, package }
}

fn paths(setup: &Setup) -> ScanPaths {
    let home = setup.case.trees.home().to_path_buf();
    let root = setup.package.path().to_path_buf();
    ScanPaths {
        source: SourcePaths::new(
            ReadPaths::new(&root, setup.case.trees.omarchy_path()),
            &home,
            root.join("skel"),
        ),
        resolve: ResolvePaths {
            share: home.join(".local/share/drift"),
            state: home.join(".local/state/drift"),
            snapshots: root.join("var/lib/drift/snapshots"),
            cache: root.join("var/cache/pacman/pkg"),
            log: root.join("var/log/pacman.log"),
        },
    }
}

fn report(setup: &Setup) -> ScanReport {
    build_report(&Registry::v1(), &paths(setup), &BTreeSet::new()).unwrap()
}

fn golden_path(directory: &Path) -> PathBuf {
    let rel = directory.strip_prefix(fixtures_dir()).unwrap();
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(rel)
        .with_extension("json")
}

#[test]
fn every_fixture_case_matches_its_golden_json() {
    let mut checked = 0;
    for directory in case_dirs().unwrap() {
        let setup = setup(&directory);
        let actual = report(&setup).to_json();
        let path = golden_path(&directory);
        let expected =
            fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert_eq!(actual, expected, "{}", directory.display());
        checked += 1;
    }
    assert!(checked >= 30, "{checked}");
}

#[test]
fn every_fixture_case_agrees_with_its_expected_statuses() {
    for directory in case_dirs().unwrap() {
        let setup = setup(&directory);
        let report = report(&setup);
        for expected in &setup.case.case.expected.files {
            let Some(status) = &expected.status else {
                continue;
            };
            let live = format!(".config/{}", expected.path);
            let Some(entry) = report.entries.iter().find(|entry| entry.path == live) else {
                assert_eq!(status, "user-only", "{live} in {}", directory.display());
                assert!(
                    !directory_has_template(&setup, &expected.path),
                    "{live} in {}",
                    directory.display()
                );
                continue;
            };
            assert_eq!(
                entry.status.label(),
                status,
                "{} in {}",
                live,
                directory.display()
            );
            assert_eq!(
                entry.badge.map(|badge| badge.label().to_string()),
                expected.badge.clone(),
                "{} in {}",
                live,
                directory.display()
            );
        }
    }
}

fn directory_has_template(setup: &Setup, rel: &str) -> bool {
    let parent = Path::new(rel).parent().unwrap_or(Path::new(""));
    fs::read_dir(setup.case.trees.template_dir().join(parent))
        .map(|dir| dir.flatten().any(|entry| entry.path().is_file()))
        .unwrap_or(false)
}

#[test]
fn scanning_writes_nothing() {
    for directory in case_dirs().unwrap() {
        let setup = setup(&directory);
        let before = (
            hash_tree(setup.case.trees.home()).unwrap(),
            hash_tree(setup.case.trees.omarchy_path()).unwrap(),
            hash_tree(setup.package.path()).unwrap(),
        );
        let first = report(&setup);
        let second = report(&setup);
        let after = (
            hash_tree(setup.case.trees.home()).unwrap(),
            hash_tree(setup.case.trees.omarchy_path()).unwrap(),
            hash_tree(setup.package.path()).unwrap(),
        );
        assert_eq!(before, after, "{}", directory.display());
        assert_eq!(first, second);
    }
}

struct World {
    root: drift_fixtures::MaterializedRoot,
}

struct WorldBuilder {
    builder: RootBuilder,
    templates: Vec<(String, Vec<u8>)>,
    unwritten: Vec<String>,
    reviewed: Option<String>,
}

impl WorldBuilder {
    fn new() -> Self {
        Self {
            builder: RootBuilder::new(),
            templates: Vec::new(),
            unwritten: Vec::new(),
            reviewed: None,
        }
    }

    fn template(&mut self, rel: &str, bytes: &[u8]) -> &mut Self {
        self.templates.push((rel.to_string(), bytes.to_vec()));
        self
    }

    fn listed_only(&mut self, rel: &str, bytes: &[u8]) -> &mut Self {
        self.unwritten.push(rel.to_string());
        self.template(rel, bytes)
    }

    fn live(&mut self, rel: &str, bytes: &[u8]) -> &mut Self {
        self.builder
            .file(format!("home/.config/{rel}"), bytes)
            .unwrap();
        self
    }

    fn link(&mut self, rel: &str, target: &str) -> &mut Self {
        self.builder
            .link(format!("home/.config/{rel}"), target)
            .unwrap();
        self
    }

    fn base(&mut self, rel: &str, bytes: &[u8]) -> &mut Self {
        self.builder
            .file(format!("home/.local/share/drift/bases/config/{rel}"), bytes)
            .unwrap();
        self
    }

    fn file(&mut self, path: &str, bytes: &[u8]) -> &mut Self {
        self.builder.file(path, bytes).unwrap();
        self
    }

    fn reviewed(&mut self, version: &str) -> &mut Self {
        self.reviewed = Some(version.to_string());
        self
    }

    fn build(&mut self, version: &str) -> World {
        let mut listed = String::from("%FILES%\nusr/share/omarchy/config/\n");
        let mut mtree = String::from("#mtree\n/set type=file uid=0 gid=0 mode=644\n");
        for (rel, bytes) in &self.templates {
            listed.push_str(&format!("usr/share/omarchy/config/{rel}\n"));
            mtree.push_str(&format!(
                "./usr/share/omarchy/config/{rel} time=1.0 size={} sha256digest={}\n",
                bytes.len(),
                digest(bytes)
            ));
            if !self.unwritten.contains(rel) {
                self.builder
                    .file(format!("usr/share/omarchy/config/{rel}"), bytes)
                    .unwrap();
            }
        }
        let dir = format!("var/lib/pacman/local/omarchy-settings-{version}");
        self.builder
            .file(
                format!("{dir}/desc"),
                format!("%NAME%\nomarchy-settings\n\n%VERSION%\n{version}\n"),
            )
            .unwrap()
            .file(format!("{dir}/files"), listed)
            .unwrap()
            .file(format!("{dir}/mtree"), gzip(&mtree))
            .unwrap();
        if let Some(reviewed) = &self.reviewed {
            self.builder
                .file("home/.local/state/drift/reviewed-version", reviewed)
                .unwrap();
        }
        World {
            root: self.builder.materialize().unwrap(),
        }
    }
}

impl World {
    fn paths(&self) -> ScanPaths {
        let root = self.root.path().to_path_buf();
        ScanPaths {
            source: SourcePaths::new(
                ReadPaths::new(&root, root.join("usr/share/omarchy")),
                root.join("home"),
                root.join("skel"),
            ),
            resolve: ResolvePaths {
                share: root.join("home/.local/share/drift"),
                state: root.join("home/.local/state/drift"),
                snapshots: root.join("var/lib/drift/snapshots"),
                cache: root.join("var/cache/pacman/pkg"),
                log: root.join("var/log/pacman.log"),
            },
        }
    }

    fn report(&self) -> ScanReport {
        build_report(&Registry::v1(), &self.paths(), &BTreeSet::new()).unwrap()
    }
}

fn status_of(report: &ScanReport, path: &str) -> String {
    report
        .entries
        .iter()
        .find(|entry| entry.path == path)
        .map(|entry| entry.status.label().to_string())
        .unwrap_or_else(|| panic!("{path} not in {:?}", report.entries))
}

#[test]
fn the_package_version_and_reviewed_version_are_reported() {
    let world = WorldBuilder::new()
        .template("hypr/a.lua", b"x\n")
        .live("hypr/a.lua", b"x\n")
        .reviewed("4.4.0-1")
        .build("4.5.0-3");
    let report = world.report();
    assert_eq!(report.package_version, "4.5.0-3");
    assert_eq!(report.reviewed_version.as_deref(), Some("4.4.0-1"));
    assert!(report.update_pending());
    let none = WorldBuilder::new()
        .template("hypr/a.lua", b"x\n")
        .build("4.5.0-3")
        .report();
    assert_eq!(none.reviewed_version, None);
}

#[test]
fn a_snapshot_base_is_found_and_labelled() {
    let world = WorldBuilder::new()
        .template("hypr/a.lua", b"one\ntwo\nthree\nTHEIRS\n")
        .live("hypr/a.lua", b"ONE\ntwo\nthree\nfour\n")
        .file(
            "var/lib/drift/snapshots/4.4.0-1/config/hypr/a.lua",
            b"one\ntwo\nthree\nfour\n",
        )
        .reviewed("4.4.0-1")
        .build("4.5.0-3");
    let report = world.report();
    let entry = &report.entries[0];
    assert_eq!(entry.status.label(), "upstream changed");
    assert_eq!(entry.base.label(), "snapshot");
    assert_eq!(
        entry.badge.map(|badge| badge.label()),
        Some("merges cleanly")
    );
}

#[test]
fn a_reviewed_base_wins_over_a_snapshot() {
    let world = WorldBuilder::new()
        .template("hypr/a.lua", b"two\n")
        .live("hypr/a.lua", b"one\n")
        .base("hypr/a.lua", b"one\n")
        .file(
            "var/lib/drift/snapshots/4.4.0-1/config/hypr/a.lua",
            b"zero\n",
        )
        .reviewed("4.4.0-1")
        .build("4.5.0-3");
    let report = world.report();
    assert_eq!(report.entries[0].status.label(), "upstream changed");
    assert_eq!(report.entries[0].base.label(), "reviewed");
}

#[test]
fn files_that_are_not_text_or_are_too_large_are_hash_only_and_cannot_merge() {
    let big: Vec<u8> = (0..20_000)
        .flat_map(|n| format!("line {n:05} {}\n", "x".repeat(50)).into_bytes())
        .collect();
    assert!(big.len() > 1024 * 1024);
    let mut big_ours = big.clone();
    big_ours[0] = b'L';
    let mut big_theirs = big.clone();
    let last = big_theirs.len() - 3;
    big_theirs[last] = b'y';
    let world = WorldBuilder::new()
        .template("hypr/bin.dat", &[0xff, 0xfe, 2])
        .live("hypr/bin.dat", &[0xff, 0xfe, 3])
        .base("hypr/bin.dat", &[0xff, 0xfe, 1])
        .template("hypr/big.txt", &big_theirs)
        .live("hypr/big.txt", &big_ours)
        .base("hypr/big.txt", &big)
        .template("hypr/same.dat", &[0xff, 1])
        .live("hypr/same.dat", &[0xff, 1])
        .build("4.5.0-3");
    let report = world.report();
    assert_eq!(status_of(&report, ".config/hypr/bin.dat"), "conflict");
    assert_eq!(status_of(&report, ".config/hypr/big.txt"), "conflict");
    assert_eq!(status_of(&report, ".config/hypr/same.dat"), "unchanged");
    let note = &report
        .entries
        .iter()
        .find(|entry| entry.path == ".config/hypr/big.txt")
        .unwrap()
        .note;
    assert!(note.contains("larger than 1 MiB"), "{note}");
    let note = &report
        .entries
        .iter()
        .find(|entry| entry.path == ".config/hypr/bin.dat")
        .unwrap()
        .note;
    assert!(note.contains("not UTF-8"), "{note}");
}

#[test]
fn a_symlink_to_the_template_counts_as_unchanged_and_other_links_are_read_only_overrides() {
    let world = WorldBuilder::new()
        .template("hypr/a.lua", b"x\n")
        .template("hypr/b.lua", b"y\n")
        .file("elsewhere/b.lua", b"different\n")
        .file("elsewhere/a.lua", b"x\n")
        .link("hypr/a.lua", "../../../elsewhere/a.lua")
        .link("hypr/b.lua", "../../../elsewhere/b.lua")
        .build("4.5.0-3");
    let report = world.report();
    assert_eq!(status_of(&report, ".config/hypr/a.lua"), "unchanged");
    assert_eq!(status_of(&report, ".config/hypr/b.lua"), "local override");
    let entry = report
        .entries
        .iter()
        .find(|entry| entry.path == ".config/hypr/b.lua")
        .unwrap();
    assert!(
        entry.note.contains("read-only symlink to"),
        "{}",
        entry.note
    );
}

#[test]
fn a_dangling_link_and_an_unreadable_live_path_are_overrides_with_a_note() {
    let mut builder = WorldBuilder::new();
    builder
        .template("hypr/a.lua", b"x\n")
        .template("hypr/b.lua", b"y\n")
        .link("hypr/a.lua", "nowhere.lua")
        .live("hypr/b.lua/inner", b"dir\n");
    let report = builder.build("4.5.0-3").report();
    assert_eq!(status_of(&report, ".config/hypr/a.lua"), "local override");
    assert_eq!(status_of(&report, ".config/hypr/b.lua"), "local override");
    let notes: Vec<&str> = report
        .entries
        .iter()
        .map(|entry| entry.note.as_str())
        .collect();
    assert!(
        notes.iter().any(|note| note.contains("read-only symlink")),
        "{notes:?}"
    );
    assert!(
        notes.iter().any(|note| note.contains("unreadable")),
        "{notes:?}"
    );
}

#[test]
fn user_only_files_backups_and_the_ignore_list_are_reported() {
    let world = WorldBuilder::new()
        .template("hypr/a.lua", b"x\n")
        .live("hypr/a.lua", b"x\n")
        .live("hypr/mine.lua", b"m\n")
        .live("hypr/skip.lua", b"s\n")
        .live("hypr/a.lua.bak.1700000000", b"old\n")
        .live("hypr/a.lua.bak.1700000001", b"old2\n")
        .live("other/loose.lua", b"never walked\n")
        .build("4.5.0-3");
    let report = world.report();
    assert_eq!(status_of(&report, ".config/hypr/mine.lua"), "user-only");
    assert_eq!(report.drifted(), 0);
    assert!(report
        .entries
        .iter()
        .all(|entry| !entry.path.contains(".bak.")));
    assert!(report
        .entries
        .iter()
        .all(|entry| !entry.path.contains("other/")));
    assert_eq!(report.backups.len(), 1);
    assert_eq!(report.backups[0].dir, ".config/hypr");
    assert_eq!(report.backups[0].count, 2);

    let ignored: BTreeSet<drift_core::RelPath> =
        [drift_core::RelPath::new(".config/hypr/skip.lua").unwrap()].into();
    let with_ignore = build_report(&Registry::v1(), &world.paths(), &ignored).unwrap();
    assert!(with_ignore
        .entries
        .iter()
        .all(|entry| !entry.path.ends_with("skip.lua")));
    assert_eq!(with_ignore.ignored, 1);
    assert_eq!(report.ignored, 0);
}

#[test]
fn the_volatile_file_carries_its_warning() {
    let world = WorldBuilder::new()
        .template("chromium/Default/Preferences", b"{}\n")
        .live("chromium/Default/Preferences", b"{\"a\":1}\n")
        .build("4.5.0-3");
    let report = world.report();
    let entry = &report.entries[0];
    assert!(entry.volatile);
    assert!(
        entry.note.contains("app rewrites this file"),
        "{}",
        entry.note
    );
}

#[test]
fn a_template_missing_from_disk_falls_back_to_the_skel_copy() {
    let mut builder = WorldBuilder::new();
    builder
        .listed_only("hypr/a.lua", b"x\n")
        .live("hypr/a.lua", b"x\n")
        .file("skel/.config/hypr/a.lua", b"x\n");
    let report = builder.build("4.5.0-3").report();
    assert_eq!(report.entries[0].status.label(), "unchanged");

    let mut absent = WorldBuilder::new();
    absent
        .listed_only("hypr/a.lua", b"x\n")
        .live("hypr/a.lua", b"x\n");
    let report = absent.build("4.5.0-3").report();
    assert_eq!(report.entries[0].status.label(), "local override");
}

#[test]
fn a_missing_package_is_an_error_and_not_an_empty_report() {
    let mut builder = RootBuilder::new();
    builder.file("home/.config/hypr/a.lua", b"x\n").unwrap();
    let root = builder.materialize().unwrap();
    let base = root.path().to_path_buf();
    let paths = ScanPaths {
        source: SourcePaths::new(
            ReadPaths::new(&base, base.join("usr/share/omarchy")),
            base.join("home"),
            base.join("skel"),
        ),
        resolve: ResolvePaths {
            share: base.join("home/.local/share/drift"),
            state: base.join("home/.local/state/drift"),
            snapshots: base.join("snap"),
            cache: base.join("cache"),
            log: base.join("log"),
        },
    };
    assert!(build_report(&Registry::v1(), &paths, &BTreeSet::new()).is_err());
}

fn fifo(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}

fn report_within(world: World, seconds: u64) -> ScanReport {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let report = world.report();
        let _ = sender.send(report);
    });
    receiver
        .recv_timeout(std::time::Duration::from_secs(seconds))
        .expect("the scan did not finish")
}

#[test]
fn a_fifo_in_place_of_a_live_file_is_unreadable_and_never_opened() {
    let world = WorldBuilder::new()
        .template("hypr/a.lua", b"x\n")
        .template("hypr/b.lua", b"y\n")
        .live("hypr/b.lua", b"y\n")
        .build("4.5.0-3");
    fifo(&world.root.path().join("home/.config/hypr/a.lua"));
    fifo(&world.root.path().join("home/.config/hypr/stray.pipe"));
    let report = report_within(world, 20);
    assert_eq!(status_of(&report, ".config/hypr/a.lua"), "local override");
    assert_eq!(status_of(&report, ".config/hypr/b.lua"), "unchanged");
    assert!(report
        .entries
        .iter()
        .all(|entry| !entry.path.ends_with("stray.pipe")));
    let note = &report
        .entries
        .iter()
        .find(|entry| entry.path == ".config/hypr/a.lua")
        .unwrap()
        .note;
    assert!(note.contains("unreadable"), "{note}");
}

#[test]
fn a_link_to_a_fifo_and_a_fifo_template_are_unreadable_and_never_opened() {
    let world = WorldBuilder::new()
        .template("hypr/a.lua", b"x\n")
        .live("hypr/c.lua", b"c\n")
        .link("hypr/a.lua", "../../../pipe")
        .build("4.5.0-3");
    fifo(&world.root.path().join("pipe"));
    let report = report_within(world, 20);
    assert_eq!(status_of(&report, ".config/hypr/a.lua"), "local override");

    let mut builder = WorldBuilder::new();
    builder
        .listed_only("hypr/t.lua", b"t\n")
        .live("hypr/t.lua", b"t\n");
    let world = builder.build("4.5.0-3");
    fifo(
        &world
            .root
            .path()
            .join("usr/share/omarchy/config/hypr/t.lua"),
    );
    let report = report_within(world, 20);
    assert_eq!(status_of(&report, ".config/hypr/t.lua"), "local override");
}

#[test]
fn a_fifo_in_the_skel_copy_is_never_opened() {
    let mut builder = WorldBuilder::new();
    builder
        .listed_only("hypr/a.lua", b"x\n")
        .live("hypr/a.lua", b"x\n");
    let world = builder.build("4.5.0-3");
    fifo(&world.root.path().join("skel/.config/hypr/a.lua"));
    let report = report_within(world, 20);
    assert_eq!(status_of(&report, ".config/hypr/a.lua"), "local override");
}
