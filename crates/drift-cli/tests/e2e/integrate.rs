use std::os::unix::fs::PermissionsExt;

use crate::harness::*;
use crate::system::System;

const MENU: &str = "home/.config/omarchy/extensions/omarchy-menu.jsonc";
const HOOK: &str = "home/.config/omarchy/hooks/post-update.d/drift";

fn run(system: &System, args: &[&str]) -> (i32, String) {
    let mut session = system.launch(args, (110, 30));
    let code = session.wait_exit();
    (code, session.screen().text())
}

fn path(system: &System, rel: &str) -> std::path::PathBuf {
    system.root().join(rel)
}

#[test]
fn integrate_writes_the_menu_row_and_an_executable_hook() {
    let system = System::world();
    let (code, shown) = run(&system, &["integrate", "--yes"]);
    assert_eq!(code, 0, "{shown}");
    assert!(shown.contains("Integrated 2 file(s) as transaction integrate-"));
    let hook = std::fs::metadata(path(&system, HOOK)).unwrap();
    assert_eq!(hook.permissions().mode() & 0o777, 0o755);
    let menu = std::fs::read_to_string(path(&system, MENU)).unwrap();
    assert!(menu.contains("drift"), "{menu}");
    let (code, shown) = run(&system, &["integrate", "--yes"]);
    assert_eq!(code, 0, "{shown}");
    assert!(shown.contains("already integrated"), "{shown}");
}

#[test]
fn undo_after_integrate_removes_what_integrate_added() {
    let system = System::world();
    assert_eq!(run(&system, &["integrate", "--yes"]).0, 0);
    assert!(path(&system, HOOK).exists());
    assert!(path(&system, MENU).exists());
    let (code, shown) = run(&system, &["undo", "--yes"]);
    assert_eq!(code, 0, "{shown}");
    assert!(!path(&system, HOOK).exists(), "{shown}");
    assert!(!path(&system, MENU).exists(), "{shown}");
}

#[test]
fn the_history_lists_an_integrate_transaction_and_its_undo_plan_names_the_files() {
    let system = System::world();
    assert_eq!(run(&system, &["integrate", "--yes"]).0, 0);
    let mut session = system.open();
    session.wait_for_text("looknfeel.lua");
    session.press("H");
    session.wait_for("the integrate transaction", |screen| {
        screen.contains("integrate-") && screen.contains("Undo and force")
    });
    session.press(ENTER);
    let plan = session.wait_for("the undo plan", |screen| {
        screen.contains("Apply plan undo-")
    });
    let flat = plan.text().split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains(".config/omarchy/hooks/post-update.d/drift"),
        "{}",
        plan.text()
    );
    assert!(
        flat.contains(".config/omarchy/extensions/omarchy-menu.jsonc"),
        "{}",
        plan.text()
    );
    session.press("y");
    session.wait_for("the undo applied", |screen| {
        !screen.contains("Apply plan") && screen.contains("UNDONE: Undid integrate-")
    });
    assert!(!path(&system, HOOK).exists());
    assert!(!path(&system, MENU).exists());
}
