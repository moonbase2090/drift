use std::path::Path;
use std::process::Command;

#[test]
fn the_bar_widget_model_passes_its_node_tests() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = match Command::new("node")
        .arg("--test")
        .arg("plugin-tests/status.test.mjs")
        .current_dir(&root)
        .output()
    {
        Ok(output) => output,
        Err(error) if std::env::var_os("DRIFT_SKIP_JS_TESTS").is_some() => {
            eprintln!("skipping the widget model tests: node is not available ({error})");
            return;
        }
        Err(error) => panic!(
            "node is needed for the widget model tests ({error}); set DRIFT_SKIP_JS_TESTS=1 to skip them"
        ),
    };
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
