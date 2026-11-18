use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

use crate::harness::Session;
use crate::system::System;

const FIRST: &str = "Item { property string revision: \"one\" }\n";
const SECOND: &str = "Item { property string revision: \"two\" }\n";
const EDITED: &str = "Item { property string revision: \"mine\" }\n";
const INSTALL_COMMANDS: &str = "omarchy-shell <shell> <rescanPlugins>\nomarchy <plugin> <enable> <drift.status>\nomarchy <bar> <put> <drift.status>\n";

fn source(system: &System) -> PathBuf {
    system.root().join("plugin-source")
}
fn installed(system: &System) -> PathBuf {
    system.home().join(".config/omarchy/plugins/drift.status")
}
fn record(system: &System) -> PathBuf {
    system
        .home()
        .join(".local/state/drift/widget-installed.json")
}
fn log(system: &System) -> String {
    fs::read_to_string(system.root().join("commands.log")).unwrap_or_default()
}

fn fixture() -> System {
    let system = System::builder().build();
    system.put(
        "plugin-source/manifest.json",
        include_bytes!("../../../../plugin/manifest.json"),
    );
    system.put("plugin-source/BarWidget.qml", FIRST);
    system.put("plugin-source/nested/payload.txt", b"nested content\n");
    for name in ["omarchy", "omarchy-shell"] {
        system.write(&format!("bin/{name}"), format!("#!/bin/sh\n{{ printf '{name}'; printf ' <%s>' \"$@\"; printf '\\n'; }} >> \"$DRIFT_ROOT/commands.log\"\n"));
    }
    system
}

fn launch(system: &System, verb: &str, flags: &[&str]) -> Session {
    let path = source(system);
    let mut args = vec!["widget", verb, "--source", path.to_str().unwrap()];
    args.extend_from_slice(flags);
    system.launch(&args, (180, 40))
}

fn run(system: &System, verb: &str, flags: &[&str], expected: i32) -> String {
    let mut session = launch(system, verb, flags);
    let code = session.wait_exit();
    let text = session.screen().text();
    assert_eq!(code, expected, "{verb}: {text}");
    text
}

fn assert_copy(system: &System, text: &str) {
    assert_eq!(
        fs::read_to_string(installed(system).join("BarWidget.qml")).unwrap(),
        text
    );
    assert_eq!(
        fs::read(installed(system).join("manifest.json")).unwrap(),
        fs::read(source(system).join("manifest.json")).unwrap()
    );
    assert_eq!(
        fs::read(installed(system).join("nested/payload.txt")).unwrap(),
        b"nested content\n"
    );
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(record(system)).unwrap()).unwrap();
    assert_eq!(value["files"].as_object().unwrap().len(), 3);
    let hash: String = Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(value["files"]["BarWidget.qml"], hash);
}

fn backups(system: &System) -> Vec<PathBuf> {
    fs::read_dir(installed(system).parent().unwrap())
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".drift.status.bak.")
        })
        .map(|entry| entry.path())
        .collect()
}

#[test]
fn widget_install_copies_records_enables_and_places_the_plugin() {
    let system = fixture();
    let text = run(&system, "install", &["--yes"], 0);
    assert!(text.contains("Copied the widget"), "{text}");
    assert_copy(&system, FIRST);
    assert_eq!(log(&system), INSTALL_COMMANDS);
    let previous = fs::read(record(&system)).unwrap();
    let text = run(&system, "install", &["--yes"], 0);
    assert!(text.contains("already up to date"), "{text}");
    assert_eq!(fs::read(record(&system)).unwrap(), previous);
    assert!(backups(&system).is_empty());
}

#[test]
fn widget_sync_updates_shipped_files_and_yes_cannot_overwrite_local_edits_but_force_backs_them_up()
{
    let system = fixture();
    run(&system, "install", &["--yes"], 0);
    system.put("commands.log", b"");
    system.put("plugin-source/BarWidget.qml", SECOND);
    run(&system, "sync", &["--yes"], 0);
    assert_copy(&system, SECOND);
    assert_eq!(log(&system), "omarchy-shell <shell> <rescanPlugins>\n");
    system.put(
        "home/.config/omarchy/plugins/drift.status/BarWidget.qml",
        EDITED,
    );
    let previous = fs::read(record(&system)).unwrap();
    let commands = log(&system);
    let text = run(&system, "sync", &["--yes"], 1);
    assert!(
        text.contains("Not overwriting your edits") && text.contains("--force"),
        "{text}"
    );
    assert_eq!(
        fs::read_to_string(installed(&system).join("BarWidget.qml")).unwrap(),
        EDITED
    );
    assert_eq!(fs::read(record(&system)).unwrap(), previous);
    assert_eq!(log(&system), commands);
    assert!(backups(&system).is_empty());
    run(&system, "sync", &["--yes", "--force"], 0);
    assert_copy(&system, SECOND);
    let kept = backups(&system);
    assert_eq!(kept.len(), 1);
    assert_eq!(
        fs::read_to_string(kept[0].join("BarWidget.qml")).unwrap(),
        EDITED
    );
    assert_eq!(
        log(&system),
        format!("{commands}omarchy-shell <shell> <rescanPlugins>\n")
    );
}

#[test]
fn widget_sync_reads_the_pty_answer_before_replacing_an_edited_copy() {
    let system = fixture();
    run(&system, "install", &["--yes"], 0);
    system.put(
        "home/.config/omarchy/plugins/drift.status/BarWidget.qml",
        EDITED,
    );
    let mut cancel = launch(&system, "sync", &[]);
    cancel.wait_for_text("Continue? [y/N]");
    assert_eq!(
        fs::read_to_string(installed(&system).join("BarWidget.qml")).unwrap(),
        EDITED
    );
    cancel.press("n\r");
    assert_eq!(cancel.wait_exit(), 1);
    assert!(backups(&system).is_empty());
    let mut confirm = launch(&system, "sync", &[]);
    confirm.wait_for_text("Continue? [y/N]");
    confirm.press("y\r");
    assert_eq!(confirm.wait_exit(), 0);
    assert_copy(&system, FIRST);
    let kept = backups(&system);
    assert_eq!(kept.len(), 1);
    assert_eq!(
        fs::read_to_string(kept[0].join("BarWidget.qml")).unwrap(),
        EDITED
    );
}

#[test]
fn widget_remove_decline_keeps_the_copy_then_yes_removes_it_and_preserves_edits() {
    let system = fixture();
    run(&system, "install", &["--yes"], 0);
    system.put(
        "home/.config/omarchy/plugins/drift.status/BarWidget.qml",
        EDITED,
    );
    system.put("commands.log", b"");
    let previous = fs::read(record(&system)).unwrap();
    let mut cancel = launch(&system, "remove", &[]);
    cancel.wait_for_text("Continue? [y/N]");
    cancel.press("n\r");
    assert_eq!(cancel.wait_exit(), 1);
    assert_eq!(fs::read(record(&system)).unwrap(), previous);
    assert_eq!(log(&system), "");
    assert!(installed(&system).is_dir());
    run(&system, "remove", &["--yes"], 0);
    assert!(!installed(&system).exists());
    assert!(!record(&system).exists());
    assert_eq!(
        fs::read_to_string(source(&system).join("BarWidget.qml")).unwrap(),
        FIRST
    );
    let kept = backups(&system);
    assert_eq!(kept.len(), 1);
    assert_eq!(
        fs::read_to_string(kept[0].join("BarWidget.qml")).unwrap(),
        EDITED
    );
    assert_eq!(
        log(&system),
        "omarchy <plugin> <disable> <drift.status>\nomarchy-shell <shell> <rescanPlugins>\n"
    );
}

#[test]
fn widget_dry_run_writes_nothing_and_no_place_skips_the_bar_command() {
    let system = fixture();
    let text = run(&system, "install", &["--dry-run"], 0);
    assert!(
        text.contains("would run") && text.contains("Dry run"),
        "{text}"
    );
    assert!(!installed(&system).exists());
    assert!(!record(&system).exists());
    assert_eq!(log(&system), "");
    run(&system, "install", &["--yes", "--no-place"], 0);
    assert_copy(&system, FIRST);
    assert_eq!(
        log(&system),
        "omarchy-shell <shell> <rescanPlugins>\nomarchy <plugin> <enable> <drift.status>\n"
    );
}
