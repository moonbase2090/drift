use std::path::Path;
use std::process::Command;

use drift_fixtures::RootBuilder;

const ALPHA: &[u8] = b"alpha\n";
const SHELL: &[u8] = b"{\"theme\":\"dark\"}\n";

fn desc(version: &str) -> String {
    format!("%NAME%\nomarchy-settings\n\n%VERSION%\n{version}\n\n%ARCH%\nx86_64\n")
}

fn files(paths: &[&str]) -> String {
    let mut text = String::from("%FILES%\n");
    for path in paths {
        text.push_str(path);
        text.push('\n');
    }
    text
}

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_drift"))
        .current_dir(root)
        .env("HOME", root)
        .env("OMARCHY_PATH", root.join("usr/share/omarchy"))
        .env("DRIFT_PACMAN_ROOT", root)
        .env("DRIFT_SNAPSHOT_DIR", root.join("var/lib/drift/snapshots"))
        .env_remove("XDG_CONFIG_HOME")
        .args(args)
        .output()
        .unwrap()
}

fn regular_files(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    let mut children = std::fs::read_dir(dir)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let path = child.path();
        let meta = std::fs::symlink_metadata(&path).unwrap();
        let kind = meta.file_type();
        assert!(!kind.is_symlink(), "{}", path.display());
        if kind.is_dir() {
            walk(root, &path, out);
        } else if kind.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            out.push((rel, std::fs::read(&path).unwrap()));
        }
    }
}

#[test]
fn snapshot_pre_copies_a_fixture_and_writes_only_under_the_store() {
    let version = "4.4.0-1";
    let dir = format!("var/lib/pacman/local/omarchy-settings-{version}");
    let mut builder = RootBuilder::new();
    builder
        .file(format!("{dir}/desc"), desc(version))
        .unwrap()
        .file(
            format!("{dir}/files"),
            files(&[
                "usr/share/omarchy/config/hypr/a.lua",
                "usr/share/omarchy/config/omarchy/shell.json",
            ]),
        )
        .unwrap()
        .file("usr/share/omarchy/config/hypr/a.lua", ALPHA)
        .unwrap()
        .file("usr/share/omarchy/config/omarchy/shell.json", SHELL)
        .unwrap()
        .file("usr/share/omarchy/bin/tool", b"tool\n")
        .unwrap()
        .link("usr/share/omarchy/config/hypr/out", "/outside")
        .unwrap()
        .file("var/lib/drift/untouched", b"stay\n")
        .unwrap()
        .file("var/lib/drift/snapshots/.keep", b"keep\n")
        .unwrap();
    let tree = builder.materialize().unwrap();
    let output = run(tree.path(), &["snapshot", "--pre"]);
    assert!(
        output.status.success(),
        "stdout {}\nstderr {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let store = tree.path().join("var/lib/drift/snapshots");
    let mut names = std::fs::read_dir(&store)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, vec![".keep".to_string(), version.to_string()]);
    assert_eq!(
        regular_files(&store.join(version)),
        vec![
            ("config/hypr/a.lua".to_string(), ALPHA.to_vec()),
            ("config/omarchy/shell.json".to_string(), SHELL.to_vec()),
        ]
    );
    assert_eq!(
        std::fs::read(tree.path().join("usr/share/omarchy/config/hypr/a.lua")).unwrap(),
        ALPHA
    );
    assert_eq!(
        std::fs::read(tree.path().join("usr/share/omarchy/bin/tool")).unwrap(),
        b"tool\n"
    );
    assert_eq!(
        std::fs::read(tree.path().join("var/lib/drift/untouched")).unwrap(),
        b"stay\n"
    );
    assert_eq!(std::fs::read(store.join(".keep")).unwrap(), b"keep\n");
}

#[test]
fn a_missing_package_exits_two_and_leaves_the_store_unchanged() {
    let mut builder = RootBuilder::new();
    builder
        .file("usr/share/omarchy/config/hypr/a.lua", ALPHA)
        .unwrap()
        .file("var/lib/drift/untouched", b"stay\n")
        .unwrap()
        .file("var/lib/drift/snapshots/.keep", b"keep\n")
        .unwrap();
    let tree = builder.materialize().unwrap();
    let output = run(tree.path(), &["snapshot", "--pre"]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("omarchy-settings is not installed"),
        "{stderr}"
    );
    let store = tree.path().join("var/lib/drift/snapshots");
    let names = std::fs::read_dir(&store)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, vec![".keep".to_string()]);
    assert_eq!(std::fs::read(store.join(".keep")).unwrap(), b"keep\n");
    assert_eq!(
        std::fs::read(tree.path().join("var/lib/drift/untouched")).unwrap(),
        b"stay\n"
    );
    assert_eq!(
        std::fs::read(tree.path().join("usr/share/omarchy/config/hypr/a.lua")).unwrap(),
        ALPHA
    );
}
