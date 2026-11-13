use std::path::Path;
use std::process::Command;

#[test]
fn the_first_update_scenario_passes_in_an_isolated_root() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("bash")
        .arg(root.join("scripts/first-update.sh"))
        .env("DRIFT_BIN", env!("CARGO_BIN_EXE_drift"))
        .output()
        .unwrap();
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
