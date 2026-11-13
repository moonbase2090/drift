use std::path::Path;
use std::process::{Command, Output};

use drift_fixtures::RootBuilder;

const FAKE: &str = "#!/bin/sh\necho \"${0##*/} $*\" >> \"$HOME/omarchy.log\"\n";

fn run(home: &Path, args: &[&str]) -> Output {
    let plugin = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugin");
    Command::new(env!("CARGO_BIN_EXE_drift"))
        .arg("widget")
        .args(args)
        .arg("--source")
        .arg(plugin)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_STATE_HOME", home.join(".local/state"))
        .env("PATH", home.join("bin"))
        .output()
        .unwrap()
}

fn log(home: &Path) -> String {
    std::fs::read_to_string(home.join("omarchy.log")).unwrap_or_default()
}

fn scratch() -> drift_fixtures::MaterializedRoot {
    let mut builder = RootBuilder::new();
    builder.file(".config/keep", b"x").unwrap();
    for name in ["bin/omarchy", "bin/omarchy-shell"] {
        builder
            .file_with_mode(name, FAKE.as_bytes(), 0o755)
            .unwrap();
    }
    builder.materialize().unwrap()
}

#[test]
fn install_sync_refusal_and_remove_work_on_a_scratch_config() {
    let root = scratch();
    let home = root.path();
    let plugin = home.join(".config/omarchy/plugins/drift.status");

    let dry = run(home, &["install", "--dry-run"]);
    assert!(dry.status.success());
    assert!(
        String::from_utf8_lossy(&dry.stdout).contains("would run  omarchy bar put drift.status")
    );
    assert!(!plugin.exists());
    assert_eq!(log(home), "");

    let installed = run(home, &["install", "--yes"]);
    assert!(installed.status.success(), "{installed:?}");
    assert!(plugin.join("manifest.json").is_file());
    assert!(plugin.join("BarWidget.qml").is_file());
    assert_eq!(
        log(home),
        "omarchy-shell shell rescanPlugins\nomarchy plugin enable drift.status\nomarchy bar put drift.status\n"
    );

    let synced = run(home, &["sync", "--yes"]);
    assert!(synced.status.success());
    assert!(String::from_utf8_lossy(&synced.stdout).contains("already up to date"));

    root.overwrite(
        ".config/omarchy/plugins/drift.status/BarWidget.qml",
        b"// edited\n",
    )
    .unwrap();
    let refused = run(home, &["install", "--yes"]);
    assert_eq!(refused.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&refused.stdout).contains("--force"));
    assert_eq!(
        std::fs::read_to_string(plugin.join("BarWidget.qml")).unwrap(),
        "// edited\n"
    );

    let forced = run(home, &["sync", "--force", "--yes"]);
    assert!(forced.status.success(), "{forced:?}");
    assert_ne!(
        std::fs::read_to_string(plugin.join("BarWidget.qml")).unwrap(),
        "// edited\n"
    );

    let removed = run(home, &["remove", "--yes"]);
    assert!(removed.status.success());
    assert!(!plugin.exists());
    assert!(log(home).contains("omarchy plugin disable drift.status"));
}

#[test]
fn without_omarchy_on_the_path_it_fails_clearly() {
    let mut builder = RootBuilder::new();
    builder.file(".config/keep", b"x").unwrap();
    let root = builder.materialize().unwrap();
    let output = run(root.path(), &["install", "--yes"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Omarchy is not installed"));
    assert!(!root
        .path()
        .join(".config/omarchy/plugins/drift.status")
        .exists());
}
