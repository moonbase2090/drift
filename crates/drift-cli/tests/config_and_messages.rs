use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::Path;

use flate2::{write::GzEncoder, Compression};
use sha2::{Digest, Sha256};
use std::process::{Command, Output};

use drift_fixtures::{hash_tree, MaterializedRoot, RootBuilder};

const USER_ROOT: &str = "[[root]]\nid = \"apps\"\ntemplate = \"applications\"\nlive = \".apps\"\n";

fn snapshot_fixture(configs: &[(&str, &str)]) -> MaterializedRoot {
    let mut builder = RootBuilder::new();
    builder
        .file("usr/share/omarchy/config/hypr/a.lua", "builtin\n")
        .unwrap()
        .file("usr/share/omarchy/applications/probe.desktop", "custom\n")
        .unwrap()
        .file(
            "var/lib/pacman/local/omarchy-settings-1-1/desc",
            "%NAME%\nomarchy-settings\n\n%VERSION%\n1-1\n",
        )
        .unwrap()
        .file(
            "var/lib/pacman/local/omarchy-settings-1-1/files",
            "%FILES%\nusr/share/omarchy/config/hypr/a.lua\n",
        )
        .unwrap();
    let digest = format!("{:x}", Sha256::digest(b"builtin\n"));
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    write!(
        encoder,
        "#mtree\n./usr/share/omarchy/config/hypr/a.lua type=file size=8 sha256digest={digest}\n"
    )
    .unwrap();
    builder
        .file(
            "var/lib/pacman/local/omarchy-settings-1-1/mtree",
            encoder.finish().unwrap(),
        )
        .unwrap()
        .file("home/.config/omarchy/.keep", "")
        .unwrap()
        .file("home/.config/hypr/a.lua", "builtin\n")
        .unwrap()
        .file("runtime/omarchy-update.lock", "held or leftover\n")
        .unwrap();
    for (path, text) in configs {
        builder.file(*path, *text).unwrap();
    }
    builder.materialize().unwrap()
}

fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_drift"));
    command
        .env("HOME", root.join("home"))
        .env("OMARCHY_PATH", root.join("usr/share/omarchy"))
        .env("DRIFT_PACMAN_ROOT", root)
        .env("DRIFT_ROOT", root)
        .env("DRIFT_SNAPSHOT_DIR", root.join("snapshots"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_RUNTIME_DIR", root.join("runtime"))
        .env_remove("XDG_CONFIG_HOME");
    command
}

fn success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn snapshot_uses_xdg_config_instead_of_a_broken_home_config() {
    let root = snapshot_fixture(&[
        ("custom/drift/config.toml", USER_ROOT),
        ("home/.config/drift/config.toml", "invalid = ["),
    ]);
    let output = command(root.path())
        .env("XDG_CONFIG_HOME", root.path().join("custom"))
        .args(["snapshot", "--pre"])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(
        std::fs::read(root.path().join("snapshots/1-1/applications/probe.desktop")).unwrap(),
        b"custom\n"
    );
    assert_eq!(
        std::fs::read(root.path().join("snapshots/1-1/config/hypr/a.lua")).unwrap(),
        b"builtin\n"
    );
}

#[test]
fn snapshot_falls_back_to_home_for_unset_empty_or_relative_xdg_config() {
    for value in [None, Some(""), Some("relative")] {
        let root = snapshot_fixture(&[
            ("home/.config/drift/config.toml", USER_ROOT),
            ("relative/drift/config.toml", "invalid = ["),
        ]);
        let mut cmd = command(root.path());
        cmd.current_dir(root.path());
        if let Some(value) = value {
            cmd.env("XDG_CONFIG_HOME", value);
        }
        success(&cmd.args(["snapshot", "--pre"]).output().unwrap());
        assert_eq!(
            std::fs::read(root.path().join("snapshots/1-1/applications/probe.desktop")).unwrap(),
            b"custom\n"
        );
    }
}

#[test]
fn a_missing_xdg_config_does_not_load_the_home_config() {
    let root = snapshot_fixture(&[("home/.config/drift/config.toml", USER_ROOT)]);
    success(
        &command(root.path())
            .env("XDG_CONFIG_HOME", root.path().join("missing"))
            .args(["snapshot", "--pre"])
            .output()
            .unwrap(),
    );
    assert!(!root.path().join("snapshots/1-1/applications").exists());
    assert_eq!(
        std::fs::read(root.path().join("snapshots/1-1/config/hypr/a.lua")).unwrap(),
        b"builtin\n"
    );
}

#[test]
fn malformed_xdg_config_names_the_selected_file_and_writes_nothing() {
    let root = snapshot_fixture(&[
        ("custom/drift/config.toml", "invalid = ["),
        ("home/.config/drift/config.toml", USER_ROOT),
    ]);
    let before = hash_tree(root.path()).unwrap();
    let output = command(root.path())
        .env("XDG_CONFIG_HOME", root.path().join("custom"))
        .args(["snapshot", "--pre"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains(
            &root
                .path()
                .join("custom/drift/config.toml")
                .display()
                .to_string()
        ),
        "{error}"
    );
    assert!(error.contains("invalid configuration"), "{error}");
    assert_eq!(hash_tree(root.path()).unwrap(), before);
}

#[test]
fn dimensions_report_out_of_range_numbers_as_range_errors() {
    for flag in ["--width", "--height"] {
        for value in [
            "65536",
            "999999999999999999999999999999999999",
            "-1",
            "-65536",
            "-999999999999999999999999999999999999",
        ] {
            let output = Command::new(env!("CARGO_BIN_EXE_drift"))
                .args([flag, value])
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(2));
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(
                error.contains(&format!("{flag} must be between 1 and 65535")),
                "{error}"
            );
        }
    }
}

struct HeldLock(std::fs::File);

impl HeldLock {
    fn new(root: &Path) -> Self {
        let file = std::fs::File::open(root.join("runtime/omarchy-update.lock")).unwrap();
        assert_eq!(
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        Self(file)
    }
}

impl Drop for HeldLock {
    fn drop(&mut self) {
        assert_eq!(unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) }, 0);
    }
}

#[test]
fn an_already_integrated_install_reports_the_held_lock_without_writing() {
    let root = snapshot_fixture(&[]);
    success(
        &command(root.path())
            .args(["integrate", "--yes"])
            .output()
            .unwrap(),
    );
    let before = hash_tree(root.path()).unwrap();
    let held = HeldLock::new(root.path());
    let output = command(root.path())
        .args(["integrate", "--yes"])
        .output()
        .unwrap();
    success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("already integrated"));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("lock is actually held"), "{error}");
    assert_eq!(hash_tree(root.path()).unwrap(), before);
    drop(held);
    let output = command(root.path())
        .args(["integrate", "--yes"])
        .output()
        .unwrap();
    success(&output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("unlocked"));
    assert_eq!(hash_tree(root.path()).unwrap(), before);
}

#[test]
fn resolving_an_unchanged_file_reports_the_held_lock_without_writing() {
    let root = snapshot_fixture(&[]);
    let before = hash_tree(root.path()).unwrap();
    let _held = HeldLock::new(root.path());
    let output = command(root.path())
        .args(["resolve", ".config/hypr/a.lua", "--upstream", "--yes"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("nothing to do"), "{error}");
    assert!(error.contains("lock is actually held"), "{error}");
    assert_eq!(hash_tree(root.path()).unwrap(), before);
}
