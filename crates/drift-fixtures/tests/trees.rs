use std::ffi::OsStr;
use std::fs::{self, File};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use drift_fixtures::{
    case_dirs, hash_tree, materialize_case, FixtureBuilder, Node, Presence, Root, BADGES, STATUSES,
};

const SAME: &[u8] = b"same\n";
const BASE_CHANGED: &[u8] = b"base\n";
const OURS_CHANGED: &[u8] = b"ours\n";
const THEIRS_CHANGED: &[u8] = b"theirs\n";

const PADDING_8: &str = "return {\n    padding = 8,\n}\n";
const PADDING_12: &str = "return {\n    padding = 12,\n}\n";
const PADDING_4: &str = "return {\n    padding = 4,\n}\n";
const MERGED_BASE: &str = "alpha\nbravo\ncharlie\n";
const MERGED_OURS: &str = "alpha\nours\ncharlie\n";
const MERGED_THEIRS: &str = "alpha\nbravo\ntheirs\n";
const CONFLICT_BASE: &str = "alpha\nmiddle\nomega\n";
const CONFLICT_OURS: &str = "alpha\nours-middle\nomega\n";
const CONFLICT_THEIRS: &str = "alpha\ntheirs-middle\nomega\n";

#[test]
fn builder_lays_out_config_templates_and_bases() {
    let trees = FixtureBuilder::new()
        .put(Root::Config, "hypr/looknfeel.lua", Node::file(b"local\n"))
        .unwrap()
        .put(
            Root::Templates,
            "hypr/looknfeel.lua",
            Node::file_mode(b"upstream\n", 0o640),
        )
        .unwrap()
        .put(
            Root::Bases,
            "hypr/looknfeel.lua",
            Node::symlink("looknfeel.lua"),
        )
        .unwrap()
        .put(Root::Config, "hypr/removed.lua", Node::absent())
        .unwrap()
        .materialize()
        .unwrap();

    assert_eq!(
        fs::read(trees.path(Root::Config, "hypr/looknfeel.lua")).unwrap(),
        b"local\n"
    );
    assert_eq!(
        fs::read(trees.path(Root::Templates, "hypr/looknfeel.lua")).unwrap(),
        b"upstream\n"
    );
    assert_eq!(
        mode_bits(&trees.path(Root::Templates, "hypr/looknfeel.lua")),
        0o640
    );
    assert_eq!(
        fs::read_link(trees.path(Root::Bases, "hypr/looknfeel.lua")).unwrap(),
        PathBuf::from("looknfeel.lua")
    );
    assert!(trees
        .path(Root::Config, "hypr/removed.lua")
        .symlink_metadata()
        .is_err());
    assert!(trees.config_dir().starts_with(trees.home()));
    assert!(trees.template_dir().starts_with(trees.omarchy_path()));
    assert!(trees.base_dir().starts_with(trees.home()));
    assert_ne!(trees.home(), trees.omarchy_path());
}

#[test]
fn cases_reproduce_bytes_links_and_absence() {
    let cases = case_dirs().unwrap();
    assert!(cases.len() >= 27 + 9);
    for case_dir in cases {
        let before = hash_tree(&case_dir).unwrap();
        let materialized = materialize_case(&case_dir).unwrap();
        let after = hash_tree(&case_dir).unwrap();
        assert_eq!(before, after, "{}", case_dir.display());
        compare_tree(
            &case_dir.join("ours"),
            &materialized.trees.config_dir(),
            &case_dir,
        );
        compare_tree(
            &case_dir.join("theirs"),
            &materialized.trees.template_dir(),
            &case_dir,
        );
        compare_tree(
            &case_dir.join("base"),
            &materialized.trees.base_dir(),
            &case_dir,
        );
        assert_absent(&case_dir, &materialized.trees);
        assert!(materialized
            .trees
            .home()
            .join("expected.json")
            .symlink_metadata()
            .is_err());
        assert!(materialized
            .trees
            .omarchy_path()
            .join("layout.json")
            .symlink_metadata()
            .is_err());
    }
}

#[test]
fn presence_matrix_matches_each_cell() {
    let mut seen = Vec::new();
    for base in ["absent", "same", "changed"] {
        for ours in ["absent", "same", "changed"] {
            for theirs in ["absent", "same", "changed"] {
                let name = format!("b-{base}_o-{ours}_t-{theirs}");
                let dir = drift_fixtures::fixtures_dir().join("presence").join(&name);
                let materialized = materialize_case(&dir).unwrap();
                expect_side(&materialized, Root::Bases, base, BASE_CHANGED);
                expect_side(&materialized, Root::Config, ours, OURS_CHANGED);
                expect_side(&materialized, Root::Templates, theirs, THEIRS_CHANGED);
                let file = &materialized.case.expected.files[0];
                assert_eq!(file.path, "hypr/note.txt");
                assert_eq!(file.base, Some(presence(base)));
                assert_eq!(file.ours, Some(presence(ours)));
                assert_eq!(file.theirs, Some(presence(theirs)));
                seen.push(name);
            }
        }
    }
    assert_eq!(seen.len(), 27);
}

#[test]
fn status_cases_cover_every_classification() {
    let root = drift_fixtures::fixtures_dir().join("status");
    expect_status_file(
        &root,
        "unchanged",
        "hypr/looknfeel.lua",
        PADDING_8,
        PADDING_8,
        PADDING_8,
    );
    expect_status_file(
        &root,
        "upstream-changed",
        "hypr/looknfeel.lua",
        PADDING_8,
        PADDING_8,
        PADDING_12,
    );
    expect_status_file(
        &root,
        "merges-cleanly",
        "hypr/looknfeel.lua",
        MERGED_BASE,
        MERGED_OURS,
        MERGED_THEIRS,
    );
    expect_status_file(
        &root,
        "local-override",
        "hypr/looknfeel.lua",
        PADDING_8,
        PADDING_4,
        PADDING_8,
    );
    expect_status_file(
        &root,
        "base-unknown",
        "hypr/looknfeel.lua",
        "",
        PADDING_4,
        PADDING_8,
    );
    expect_status_file(
        &root,
        "conflict",
        "hypr/looknfeel.lua",
        CONFLICT_BASE,
        CONFLICT_OURS,
        CONFLICT_THEIRS,
    );
    expect_status_file(
        &root,
        "user-only",
        "hypr/shipped.lua",
        "shipped\n",
        "shipped\n",
        "shipped\n",
    );
    let user_only = materialize_case(&root.join("user-only")).unwrap();
    assert_eq!(
        fs::read(user_only.trees.path(Root::Config, "hypr/custom.lua")).unwrap(),
        b"custom\n"
    );
    assert!(user_only
        .trees
        .path(Root::Templates, "hypr/custom.lua")
        .symlink_metadata()
        .is_err());
    expect_status_file(
        &root,
        "missing",
        "hypr/monitors.lua",
        "monitor\n",
        "",
        "monitor\n",
    );
    expect_status_file(
        &root,
        "missing-no-base",
        "hypr/monitors.lua",
        "",
        "",
        "monitor\n",
    );

    let mut statuses = Vec::new();
    let mut badges = Vec::new();
    for case_dir in case_dirs().unwrap() {
        let loaded = drift_fixtures::load_case(&case_dir).unwrap();
        for file in loaded.expected.files {
            if let Some(status) = file.status {
                assert!(STATUSES.contains(&status.as_str()), "{status}");
                statuses.push(status);
            }
            if let Some(badge) = file.badge {
                assert!(BADGES.contains(&badge.as_str()), "{badge}");
                badges.push(badge);
            }
        }
    }
    for status in STATUSES {
        assert!(statuses.iter().any(|item| item == status), "{status}");
    }
    for badge in BADGES {
        assert!(badges.iter().any(|item| item == badge), "{badge}");
    }
}

#[test]
fn mode_symlink_and_absent_entries_round_trip() {
    let dir = drift_fixtures::fixtures_dir()
        .join("entries")
        .join("modes-and-links");
    let materialized = materialize_case(&dir).unwrap();
    for root in [Root::Bases, Root::Config, Root::Templates] {
        let path = materialized.trees.path(root, "hypr/secret.lua");
        assert_eq!(fs::read(&path).unwrap(), b"secret\n");
        assert_eq!(mode_bits(&path), 0o600);
    }
    assert_eq!(
        fs::read_link(materialized.trees.path(Root::Config, "hypr/alias.lua")).unwrap(),
        PathBuf::from("secret.lua")
    );
    assert!(materialized
        .trees
        .path(Root::Config, "hypr/removed.lua")
        .symlink_metadata()
        .is_err());
    assert_eq!(
        fs::read(materialized.trees.path(Root::Bases, "hypr/removed.lua")).unwrap(),
        b"gone\n"
    );
    assert_eq!(
        fs::read(materialized.trees.path(Root::Templates, "hypr/removed.lua")).unwrap(),
        b"gone\n"
    );

    let declared = materialize_case(
        &drift_fixtures::fixtures_dir()
            .join("entries")
            .join("layout-symlink"),
    )
    .unwrap();
    assert_eq!(
        fs::read_link(declared.trees.path(Root::Config, "hypr/alias.lua")).unwrap(),
        PathBuf::from("secret.lua")
    );
    assert!(declared
        .trees
        .path(Root::Templates, "hypr/alias.lua")
        .symlink_metadata()
        .is_err());
}

#[test]
fn tree_hash_sees_one_byte_mode_and_mtime_changes() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("note.txt");
    fs::write(&file, b"alpha\n").unwrap();
    let mut baseline = fs::metadata(&file).unwrap().permissions();
    baseline.set_mode(0o644);
    fs::set_permissions(&file, baseline).unwrap();
    stamp_tree(root.path(), 1_700_000_000);
    let original = hash_tree(root.path()).unwrap();

    fs::write(&file, b"alphb\n").unwrap();
    stamp_tree(root.path(), 1_700_000_000);
    let changed_byte = hash_tree(root.path()).unwrap();
    assert_ne!(changed_byte, original);

    fs::write(&file, b"alpha\n").unwrap();
    stamp_tree(root.path(), 1_700_000_000);
    assert_eq!(hash_tree(root.path()).unwrap(), original);

    let mut permissions = fs::metadata(&file).unwrap().permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(&file, permissions).unwrap();
    stamp_tree(root.path(), 1_700_000_000);
    let changed_mode = hash_tree(root.path()).unwrap();
    assert_ne!(changed_mode, original);

    let mut permissions = fs::metadata(&file).unwrap().permissions();
    permissions.set_mode(0o644);
    fs::set_permissions(&file, permissions).unwrap();
    stamp_tree(root.path(), 1_700_000_000);
    assert_eq!(hash_tree(root.path()).unwrap(), original);

    stamp_file(&file, 1_700_000_005);
    stamp_dirs(root.path(), 1_700_000_000);
    let changed_mtime = hash_tree(root.path()).unwrap();
    assert_ne!(changed_mtime, original);

    stamp_tree(root.path(), 1_700_000_000);
    assert_eq!(hash_tree(root.path()).unwrap(), original);
}

#[test]
fn materialize_refuses_to_follow_a_symlink_ancestor() {
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("victim");
    fs::write(&victim, b"original").unwrap();
    let target = outside
        .path()
        .to_str()
        .expect("temporary path is utf-8")
        .to_string();

    let error = FixtureBuilder::new()
        .put(Root::Config, "a", Node::symlink(target))
        .unwrap()
        .put(Root::Config, "a/victim", Node::file(b"pwned"))
        .unwrap()
        .materialize()
        .unwrap_err();

    assert!(
        error.to_string().contains("refusing to follow a symlink"),
        "{error}"
    );
    assert_eq!(fs::read(&victim).unwrap(), b"original");
}

#[test]
fn tree_hash_distinguishes_non_utf8_names() {
    let directory_byte = hash_named_tree(0xff, b"note");
    let other_directory = hash_named_tree(0xfe, b"note");
    assert_ne!(directory_byte, other_directory);

    let file_byte = hash_named_tree(b'd', &[0xff]);
    let other_file = hash_named_tree(b'd', &[0xfe]);
    assert_ne!(file_byte, other_file);
}

#[test]
fn tree_hash_sees_a_rename_when_the_parent_mtime_is_restored() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("parent");
    fs::create_dir(&parent).unwrap();
    let from = parent.join(OsStr::from_bytes(&[0xff]));
    fs::write(&from, b"body\n").unwrap();
    stamp_tree(root.path(), 1_700_000_210);
    let before = hash_tree(root.path()).unwrap();

    let to = parent.join(OsStr::from_bytes(&[0xfe]));
    fs::rename(&from, &to).unwrap();
    stamp_file(&to, 1_700_000_210);
    stamp_dirs(root.path(), 1_700_000_210);
    assert_ne!(hash_tree(root.path()).unwrap(), before);
}

#[test]
fn tree_hash_sees_a_symlink_target_change() {
    let root = tempfile::tempdir().unwrap();
    let link = root.path().join("alias");
    std::os::unix::fs::symlink("first", &link).unwrap();
    stamp_tree(root.path(), 1_700_000_100);
    let original = hash_tree(root.path()).unwrap();
    fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink("second", &link).unwrap();
    stamp_tree(root.path(), 1_700_000_100);
    assert_ne!(hash_tree(root.path()).unwrap(), original);
}

#[test]
fn production_crates_do_not_depend_on_fixtures() {
    let crates = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let mut checked = 0;
    for entry in fs::read_dir(&crates).unwrap() {
        let entry = entry.unwrap();
        let manifest = entry.path().join("Cargo.toml");
        if entry.file_name() == "drift-fixtures" || !manifest.is_file() {
            continue;
        }
        let text = fs::read_to_string(&manifest).unwrap();
        assert!(
            !production_dependency(&text),
            "{} depends on drift-fixtures",
            manifest.display()
        );
        checked += 1;
    }
    assert!(checked >= 5, "checked {checked} crates");
}

fn expect_status_file(
    root: &Path,
    case_name: &str,
    rel: &str,
    base: &str,
    ours: &str,
    theirs: &str,
) {
    let materialized = materialize_case(&root.join(case_name)).unwrap();
    expect_text(&materialized, Root::Bases, rel, base);
    expect_text(&materialized, Root::Config, rel, ours);
    expect_text(&materialized, Root::Templates, rel, theirs);
    let file = materialized
        .case
        .expected
        .files
        .iter()
        .find(|file| file.path == rel)
        .unwrap();
    assert!(file.status.is_some(), "{case_name}");
}

fn expect_text(materialized: &drift_fixtures::MaterializedCase, root: Root, rel: &str, text: &str) {
    let path = materialized.trees.path(root, rel);
    if text.is_empty() {
        assert!(path.symlink_metadata().is_err(), "{}", path.display());
    } else {
        assert_eq!(
            fs::read(&path).unwrap(),
            text.as_bytes(),
            "{}",
            path.display()
        );
    }
}

fn expect_side(
    materialized: &drift_fixtures::MaterializedCase,
    root: Root,
    kind: &str,
    changed: &[u8],
) {
    let path = materialized.trees.path(root, "hypr/note.txt");
    match kind {
        "absent" => assert!(path.symlink_metadata().is_err(), "{}", path.display()),
        "same" => assert_eq!(fs::read(&path).unwrap(), SAME),
        "changed" => assert_eq!(fs::read(&path).unwrap(), changed),
        _ => panic!("unknown presence {kind}"),
    }
}

fn presence(kind: &str) -> Presence {
    match kind {
        "absent" => Presence::Absent,
        "same" => Presence::Same,
        "changed" => Presence::Changed,
        _ => panic!("unknown presence {kind}"),
    }
}

fn compare_tree(source: &Path, dest_root: &Path, case_dir: &Path) {
    if !source.exists() {
        return;
    }
    walk_compare(source, source, dest_root, case_dir);
}

fn walk_compare(root: &Path, dir: &Path, dest_root: &Path, case_dir: &Path) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap();
        let dest = dest_root.join(rel);
        assert!(dest.starts_with(dest_root));
        assert_ne!(dest, path);
        let file_type = entry.file_type().unwrap();
        if file_type.is_symlink() {
            assert_eq!(fs::read_link(&dest).unwrap(), fs::read_link(&path).unwrap());
            continue;
        }
        if file_type.is_dir() {
            assert!(dest.is_dir(), "{}", dest.display());
            walk_compare(root, &path, dest_root, case_dir);
            continue;
        }
        assert_eq!(
            fs::read(&dest).unwrap(),
            fs::read(&path).unwrap(),
            "{}",
            dest.display()
        );
        let override_mode = layout_mode(case_dir, root, rel);
        let expected_mode = override_mode.unwrap_or_else(|| mode_bits(&path));
        assert_eq!(mode_bits(&dest), expected_mode, "{}", dest.display());
    }
}

fn assert_absent(case_dir: &Path, trees: &drift_fixtures::FixtureTrees) {
    let layout_path = case_dir.join("layout.json");
    if !layout_path.is_file() {
        return;
    }
    let text = fs::read_to_string(&layout_path).unwrap();
    let layout: serde_json::Value = serde_json::from_str(&text).unwrap();
    let Some(absent) = layout.get("absent").and_then(|value| value.as_array()) else {
        return;
    };
    for item in absent {
        let case_path = item.as_str().unwrap();
        let (tree, rel) = case_path.split_once('/').unwrap();
        let dest = match tree {
            "ours" => trees.config_dir().join(rel),
            "theirs" => trees.template_dir().join(rel),
            "base" => trees.base_dir().join(rel),
            _ => panic!("bad tree in {case_path}"),
        };
        assert!(dest.symlink_metadata().is_err(), "{}", dest.display());
    }
}

fn layout_mode(case_dir: &Path, source_root: &Path, rel: &Path) -> Option<u32> {
    let layout_path = case_dir.join("layout.json");
    if !layout_path.is_file() {
        return None;
    }
    let text = fs::read_to_string(layout_path).ok()?;
    let layout: serde_json::Value = serde_json::from_str(&text).ok()?;
    let tree = source_root.file_name()?.to_str()?;
    let key = format!("{tree}/{}", rel.display());
    let text = layout.get("modes")?.get(key)?.as_str()?;
    u32::from_str_radix(text, 8).ok()
}

fn mode_bits(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
}

fn hash_named_tree(directory: u8, file: &[u8]) -> drift_fixtures::TreeDigest {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join(OsStr::from_bytes(&[directory]));
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join(OsStr::from_bytes(file)), b"body\n").unwrap();
    stamp_tree(root.path(), 1_700_000_200);
    hash_tree(root.path()).unwrap()
}

fn stamp_tree(root: &Path, secs: u64) {
    stamp_files(root, secs);
    stamp_dirs(root, secs);
}

fn stamp_dirs(root: &Path, secs: u64) {
    let mut dirs = vec![root.to_path_buf()];
    let mut index = 0;
    while index < dirs.len() {
        let dir = dirs[index].clone();
        index += 1;
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() && !meta.file_type().is_symlink() {
                dirs.push(path);
            }
        }
    }
    for dir in dirs {
        stamp_path(&dir, secs, true);
    }
}

fn stamp_files(root: &Path, secs: u64) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(path);
            } else {
                stamp_path(&path, secs, false);
            }
        }
    }
}

fn stamp_file(path: &Path, secs: u64) {
    stamp_path(path, secs, false);
}

fn stamp_path(path: &Path, secs: u64, directory: bool) {
    let stamp = UNIX_EPOCH + Duration::from_secs(secs);
    let file = if directory {
        File::open(path).unwrap()
    } else {
        File::options().write(true).open(path).unwrap()
    };
    file.set_modified(stamp).unwrap();
}

fn production_dependency(text: &str) -> bool {
    let mut section = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = trimmed.trim_matches(['[', ']']).to_string();
            if is_named_fixture_section(&section) {
                return true;
            }
            continue;
        }
        if !is_prod_dep_section(&section) {
            continue;
        }
        let Some(key) = trimmed.split('=').next() else {
            continue;
        };
        let key = key.trim().trim_matches('"');
        if key == "drift-fixtures" || key.starts_with("drift-fixtures.") {
            return true;
        }
    }
    false
}

fn is_prod_dep_section(section: &str) -> bool {
    if section.starts_with("workspace.") || section.contains("dev-dependencies") {
        return false;
    }
    section == "dependencies"
        || section.starts_with("dependencies.")
        || section.ends_with(".dependencies")
        || section.contains(".dependencies.")
        || section == "build-dependencies"
        || section.starts_with("build-dependencies.")
        || section.ends_with(".build-dependencies")
}

fn is_named_fixture_section(section: &str) -> bool {
    section == "dependencies.drift-fixtures"
        || section == "build-dependencies.drift-fixtures"
        || section.ends_with(".dependencies.drift-fixtures")
}
