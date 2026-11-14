use std::path::Path;
use std::process::Command;

fn scenario(dir: Option<&Path>, bin: &str) -> std::process::Output {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut command = Command::new("bash");
    command
        .arg(root.join("scripts/first-update.sh"))
        .env("DRIFT_BIN", bin);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    command.output().unwrap()
}

#[test]
fn a_relative_binary_path_works_from_another_directory() {
    let bin = Path::new(env!("CARGO_BIN_EXE_drift"));
    let output = scenario(bin.parent(), bin.file_name().unwrap().to_str().unwrap());
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("first-update scenario passed"));
}

#[test]
fn a_missing_binary_is_refused_up_front() {
    let output = scenario(None, "no/such/drift");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not an executable file"));
}

#[test]
fn the_first_update_scenario_passes_in_an_isolated_root() {
    let output = scenario(None, env!("CARGO_BIN_EXE_drift"));
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(
        output.status.success(),
        "{text}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for needle in [
        "== 1. fresh install",
        "(base: snapshot)",
        "3 configs drifted",
        "first-update scenario passed",
    ] {
        assert!(text.contains(needle), "{needle} missing from:\n{text}");
    }
}
