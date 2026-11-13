use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use drift_apply::{
    widget_install, widget_remove, widget_sync, OmarchyShell, WidgetMode, WidgetOptions,
    WidgetOutcome, WidgetPaths, RECORD_FILE,
};

static TEMPS: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let n = TEMPS.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "drift-widget-{}-{n}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for dir in ["source", "plugins", "state"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        let fixture = Self { root };
        fixture.ship("manifest.json", "{\"id\": \"drift.status\"}\n");
        fixture.ship("BarWidget.qml", "Item { text: \"one\" }\n");
        fixture
    }

    fn ship(&self, name: &str, text: &str) {
        fs::write(self.root.join("source").join(name), text).unwrap();
    }

    fn unship(&self, name: &str) {
        fs::remove_file(self.root.join("source").join(name)).unwrap();
    }

    fn paths(&self) -> WidgetPaths {
        WidgetPaths {
            source: self.root.join("source"),
            plugins: self.root.join("plugins"),
            state: self.root.join("state"),
        }
    }

    fn installed(&self, name: &str) -> PathBuf {
        self.root.join("plugins/drift.status").join(name)
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.installed(name)).unwrap()
    }

    fn edit(&self, name: &str, text: &str) {
        fs::write(self.installed(name), text).unwrap();
    }

    fn backups(&self) -> Vec<PathBuf> {
        fs::read_dir(self.root.join("plugins"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".drift.status.bak.")
            })
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Fake {
    present: bool,
    runs: Vec<String>,
    fail_on: Option<&'static str>,
}

impl Fake {
    fn new() -> Self {
        Self {
            present: true,
            runs: Vec::new(),
            fail_on: None,
        }
    }
}

impl OmarchyShell for Fake {
    fn present(&self) -> bool {
        self.present
    }

    fn run(&mut self, program: &str, args: &[&str]) -> Result<(), String> {
        let line = format!("{program} {}", args.join(" "));
        if self.fail_on.is_some_and(|needle| line.contains(needle)) {
            return Err("boom".to_string());
        }
        self.runs.push(line);
        Ok(())
    }
}

fn options(mode: WidgetMode) -> WidgetOptions {
    WidgetOptions {
        mode,
        force: false,
        place: true,
    }
}

type Call = fn(
    &WidgetPaths,
    &mut dyn OmarchyShell,
    WidgetOptions,
    &str,
    &mut dyn std::io::Write,
    &mut dyn FnMut() -> bool,
) -> Result<WidgetOutcome, String>;

fn go(
    call: Call,
    fixture: &Fixture,
    omarchy: &mut Fake,
    options: WidgetOptions,
    answer: bool,
) -> (Result<WidgetOutcome, String>, String, usize) {
    let mut out = Vec::new();
    let mut asked = 0;
    let result = call(
        &fixture.paths(),
        omarchy,
        options,
        "20261113T000000",
        &mut out,
        &mut || {
            asked += 1;
            answer
        },
    );
    (result, String::from_utf8(out).unwrap(), asked)
}

#[test]
fn install_copies_the_plugin_then_rescans_enables_and_places_it() {
    let fixture = Fixture::new();
    let mut omarchy = Fake::new();
    let (result, _, asked) = go(
        widget_install,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Confirm),
        false,
    );
    assert_eq!(result, Ok(WidgetOutcome::Installed));
    assert_eq!(asked, 0);
    assert_eq!(fixture.read("BarWidget.qml"), "Item { text: \"one\" }\n");
    assert_eq!(
        fixture.read("manifest.json"),
        "{\"id\": \"drift.status\"}\n"
    );
    assert_eq!(
        omarchy.runs,
        [
            "omarchy-shell shell rescanPlugins",
            "omarchy plugin enable drift.status",
            "omarchy bar put drift.status"
        ]
    );
    assert!(fixture.root.join("state").join(RECORD_FILE).is_file());
}

#[test]
fn no_place_enables_without_putting_the_widget_on_the_bar() {
    let fixture = Fixture::new();
    let mut omarchy = Fake::new();
    let mut flags = options(WidgetMode::Yes);
    flags.place = false;
    let (result, _, _) = go(widget_install, &fixture, &mut omarchy, flags, false);
    assert_eq!(result, Ok(WidgetOutcome::Installed));
    assert!(omarchy.runs.iter().all(|line| !line.contains("bar put")));
    assert!(omarchy
        .runs
        .contains(&"omarchy plugin enable drift.status".to_string()));
}

#[test]
fn installing_again_changes_no_file_and_still_enables() {
    let fixture = Fixture::new();
    let mut omarchy = Fake::new();
    let _ = go(
        widget_install,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    omarchy.runs.clear();
    let (result, text, _) = go(
        widget_install,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    assert_eq!(result, Ok(WidgetOutcome::UpToDate));
    assert!(text.contains("already up to date"), "{text}");
    assert!(omarchy
        .runs
        .iter()
        .any(|line| line.contains("plugin enable")));
    assert!(fixture.backups().is_empty());
}

#[test]
fn sync_refreshes_an_unchanged_copy_without_asking() {
    let fixture = Fixture::new();
    let mut omarchy = Fake::new();
    let _ = go(
        widget_install,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    fixture.ship("BarWidget.qml", "Item { text: \"two\" }\n");
    fixture.ship("Extra.js", "// new\n");
    omarchy.runs.clear();
    let (result, _, asked) = go(
        widget_sync,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Confirm),
        false,
    );
    assert_eq!(result, Ok(WidgetOutcome::Synced));
    assert_eq!(asked, 0);
    assert_eq!(fixture.read("BarWidget.qml"), "Item { text: \"two\" }\n");
    assert_eq!(fixture.read("Extra.js"), "// new\n");
    assert_eq!(omarchy.runs, ["omarchy-shell shell rescanPlugins"]);
    assert!(fixture.backups().is_empty());
}

#[test]
fn sync_removes_a_file_no_longer_shipped_unless_the_user_changed_it() {
    let fixture = Fixture::new();
    fixture.ship("Old.js", "// old\n");
    fixture.ship("Mine.js", "// theirs\n");
    let mut omarchy = Fake::new();
    let _ = go(
        widget_install,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    fixture.edit("Mine.js", "// mine\n");
    fixture.unship("Old.js");
    fixture.unship("Mine.js");
    let (result, _, _) = go(
        widget_sync,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    assert_eq!(result, Ok(WidgetOutcome::Synced));
    assert!(!fixture.installed("Old.js").exists());
    assert_eq!(fixture.read("Mine.js"), "// mine\n");
}

#[test]
fn an_edited_file_is_never_overwritten_without_asking() {
    let fixture = Fixture::new();
    let mut omarchy = Fake::new();
    let _ = go(
        widget_install,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    fixture.edit("BarWidget.qml", "Item { text: \"mine\" }\n");
    fixture.ship("BarWidget.qml", "Item { text: \"two\" }\n");
    omarchy.runs.clear();

    let (result, text, asked) = go(
        widget_sync,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        true,
    );
    assert_eq!(result, Ok(WidgetOutcome::Refused));
    assert_eq!(asked, 0);
    assert!(text.contains("--force"), "{text}");
    assert!(
        text.contains("-Item { text: \"mine\" }") || text.contains("mine"),
        "{text}"
    );

    let (result, _, asked) = go(
        widget_sync,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Confirm),
        false,
    );
    assert_eq!(result, Ok(WidgetOutcome::Declined));
    assert_eq!(asked, 1);
    assert_eq!(fixture.read("BarWidget.qml"), "Item { text: \"mine\" }\n");
    assert!(fixture.backups().is_empty());
    assert!(omarchy.runs.is_empty());
}

#[test]
fn confirming_replaces_the_edit_and_keeps_a_backup_of_it() {
    let fixture = Fixture::new();
    let mut omarchy = Fake::new();
    let _ = go(
        widget_install,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    fixture.edit("BarWidget.qml", "Item { text: \"mine\" }\n");
    fixture.ship("BarWidget.qml", "Item { text: \"two\" }\n");
    let (result, text, asked) = go(
        widget_sync,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Confirm),
        true,
    );
    assert_eq!(result, Ok(WidgetOutcome::Synced));
    assert_eq!(asked, 1);
    assert_eq!(fixture.read("BarWidget.qml"), "Item { text: \"two\" }\n");
    let backups = fixture.backups();
    assert_eq!(backups.len(), 1);
    assert_eq!(
        fs::read_to_string(backups[0].join("BarWidget.qml")).unwrap(),
        "Item { text: \"mine\" }\n"
    );
    assert!(text.contains("kept at"), "{text}");
}

#[test]
fn force_replaces_the_edit_without_a_question_and_still_keeps_a_backup() {
    let fixture = Fixture::new();
    let mut omarchy = Fake::new();
    let _ = go(
        widget_install,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    fixture.edit("BarWidget.qml", "Item { text: \"mine\" }\n");
    fixture.ship("BarWidget.qml", "Item { text: \"two\" }\n");
    let mut flags = options(WidgetMode::Yes);
    flags.force = true;
    let (result, _, asked) = go(widget_sync, &fixture, &mut omarchy, flags, false);
    assert_eq!(result, Ok(WidgetOutcome::Synced));
    assert_eq!(asked, 0);
    assert_eq!(fixture.read("BarWidget.qml"), "Item { text: \"two\" }\n");
    assert_eq!(fixture.backups().len(), 1);
}

#[test]
fn a_hand_placed_copy_that_differs_counts_as_edited() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.root.join("plugins/drift.status")).unwrap();
    fixture.edit("BarWidget.qml", "Item { text: \"hand\" }\n");
    let mut omarchy = Fake::new();
    let (result, _, _) = go(
        widget_install,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    assert_eq!(result, Ok(WidgetOutcome::Refused));
    assert_eq!(fixture.read("BarWidget.qml"), "Item { text: \"hand\" }\n");
}

#[test]
fn a_dry_run_shows_the_plan_and_the_commands_and_changes_nothing() {
    let fixture = Fixture::new();
    let mut omarchy = Fake::new();
    omarchy.present = false;
    let (result, text, _) = go(
        widget_install,
        &fixture,
        &mut omarchy,
        options(WidgetMode::DryRun),
        false,
    );
    assert_eq!(result, Ok(WidgetOutcome::DryRun));
    assert!(text.contains("add  drift.status/BarWidget.qml"), "{text}");
    assert!(
        text.contains("would run  omarchy bar put drift.status"),
        "{text}"
    );
    assert!(!fixture.root.join("plugins/drift.status").exists());
    assert!(!fixture.root.join("state").join(RECORD_FILE).exists());
    assert!(omarchy.runs.is_empty());
}

#[test]
fn without_omarchy_install_and_sync_fail_clearly_and_write_nothing() {
    let fixture = Fixture::new();
    let mut omarchy = Fake::new();
    omarchy.present = false;
    let (result, _, _) = go(
        widget_install,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    let message = result.unwrap_err();
    assert!(message.contains("Omarchy is not installed"), "{message}");
    assert!(!fixture.root.join("plugins/drift.status").exists());
}

#[test]
fn sync_before_install_says_to_install() {
    let fixture = Fixture::new();
    let mut omarchy = Fake::new();
    let (result, _, _) = go(
        widget_sync,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    assert!(result.unwrap_err().contains("drift widget install"));
}

#[test]
fn a_failing_omarchy_command_is_reported() {
    let fixture = Fixture::new();
    let mut omarchy = Fake::new();
    omarchy.fail_on = Some("plugin enable");
    let (result, _, _) = go(
        widget_install,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    assert_eq!(result, Err("boom".to_string()));
}

#[test]
fn remove_disables_deletes_the_user_copy_and_leaves_the_source_alone() {
    let fixture = Fixture::new();
    let mut omarchy = Fake::new();
    let _ = go(
        widget_install,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    omarchy.runs.clear();
    let (result, _, asked) = go(
        widget_remove,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Confirm),
        true,
    );
    assert_eq!(result, Ok(WidgetOutcome::Removed));
    assert_eq!(asked, 1);
    assert!(!fixture.root.join("plugins/drift.status").exists());
    assert!(!fixture.root.join("state").join(RECORD_FILE).exists());
    assert!(fixture.root.join("source/BarWidget.qml").is_file());
    assert!(fixture.backups().is_empty());
    assert_eq!(
        omarchy.runs,
        [
            "omarchy plugin disable drift.status",
            "omarchy-shell shell rescanPlugins"
        ]
    );
}

#[test]
fn remove_asks_first_and_keeps_a_backup_of_an_edited_copy() {
    let fixture = Fixture::new();
    let mut omarchy = Fake::new();
    let _ = go(
        widget_install,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    fixture.edit("BarWidget.qml", "Item { text: \"mine\" }\n");
    let (result, _, asked) = go(
        widget_remove,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Confirm),
        false,
    );
    assert_eq!(result, Ok(WidgetOutcome::Declined));
    assert_eq!(asked, 1);
    assert!(fixture.installed("BarWidget.qml").is_file());
    let (result, _, _) = go(
        widget_remove,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    assert_eq!(result, Ok(WidgetOutcome::Removed));
    let backups = fixture.backups();
    assert_eq!(backups.len(), 1);
    assert_eq!(
        fs::read_to_string(backups[0].join("BarWidget.qml")).unwrap(),
        "Item { text: \"mine\" }\n"
    );
}

#[test]
fn removing_what_is_not_installed_is_a_no_op_and_a_dry_run_removes_nothing() {
    let fixture = Fixture::new();
    let mut omarchy = Fake::new();
    let (result, _, _) = go(
        widget_remove,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    assert_eq!(result, Ok(WidgetOutcome::NotInstalled));
    let _ = go(
        widget_install,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    omarchy.runs.clear();
    let (result, text, _) = go(
        widget_remove,
        &fixture,
        &mut omarchy,
        options(WidgetMode::DryRun),
        false,
    );
    assert_eq!(result, Ok(WidgetOutcome::DryRun));
    assert!(text.contains("would run  omarchy plugin disable"), "{text}");
    assert!(fixture.installed("BarWidget.qml").is_file());
    assert!(omarchy.runs.is_empty());
}

fn outside(fixture: &Fixture, text: &str) -> PathBuf {
    let dir = fixture.root.join("outside");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("data"), text).unwrap();
    dir
}

#[test]
fn a_symlinked_directory_inside_the_copy_is_never_written_through() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.root.join("source/assets")).unwrap();
    fixture.ship("assets/data", "SHIPPED\n");
    let target = outside(&fixture, "MINE\n");
    fs::create_dir_all(fixture.root.join("plugins/drift.status")).unwrap();
    symlink(&target, fixture.installed("assets")).unwrap();
    let mut omarchy = Fake::new();
    let (result, _, asked) = go(
        widget_install,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    assert!(result.unwrap_err().contains("symlink"));
    assert_eq!(asked, 0);
    assert_eq!(fs::read_to_string(target.join("data")).unwrap(), "MINE\n");
    assert!(omarchy.runs.is_empty());
    assert!(fixture.backups().is_empty());
}

#[test]
fn a_symlinked_file_at_a_shipped_path_is_never_replaced_or_followed() {
    let fixture = Fixture::new();
    let target = outside(&fixture, "MINE\n");
    fs::create_dir_all(fixture.root.join("plugins/drift.status")).unwrap();
    symlink(target.join("data"), fixture.installed("BarWidget.qml")).unwrap();
    let mut omarchy = Fake::new();
    let (result, _, _) = go(
        widget_install,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    assert!(result.unwrap_err().contains("symlink"));
    assert_eq!(fs::read_to_string(target.join("data")).unwrap(), "MINE\n");
    assert!(fs::symlink_metadata(fixture.installed("BarWidget.qml"))
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn a_symlinked_plugin_directory_is_refused_by_install_and_sync_and_only_unlinked_by_remove() {
    let fixture = Fixture::new();
    let target = outside(&fixture, "MINE\n");
    symlink(&target, fixture.root.join("plugins/drift.status")).unwrap();
    let mut omarchy = Fake::new();
    for call in [widget_install as Call, widget_sync as Call] {
        let (result, _, _) = go(
            call,
            &fixture,
            &mut omarchy,
            options(WidgetMode::Yes),
            false,
        );
        assert!(result.unwrap_err().contains("symlink"));
    }
    assert!(!target.join("BarWidget.qml").exists());
    let (result, _, asked) = go(
        widget_remove,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Confirm),
        true,
    );
    assert_eq!(result, Ok(WidgetOutcome::Removed));
    assert_eq!(asked, 1);
    assert!(fs::symlink_metadata(fixture.root.join("plugins/drift.status")).is_err());
    assert_eq!(fs::read_to_string(target.join("data")).unwrap(), "MINE\n");
    assert!(fixture.backups().is_empty());
    assert!(omarchy
        .runs
        .contains(&"omarchy plugin disable drift.status".to_string()));
}

#[test]
fn a_symlink_the_plugin_does_not_ship_is_left_alone_by_sync() {
    let fixture = Fixture::new();
    let mut omarchy = Fake::new();
    let _ = go(
        widget_install,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    let target = outside(&fixture, "MINE\n");
    symlink(&target, fixture.installed("notes")).unwrap();
    fixture.ship("BarWidget.qml", "Item { text: \"two\" }\n");
    let (result, _, _) = go(
        widget_sync,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    assert_eq!(result, Ok(WidgetOutcome::Synced));
    assert!(fs::symlink_metadata(fixture.installed("notes"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_to_string(target.join("data")).unwrap(), "MINE\n");
}

#[test]
fn a_symlink_planted_at_a_temporary_name_is_not_followed() {
    let fixture = Fixture::new();
    let target = outside(&fixture, "MINE\n");
    fs::create_dir_all(fixture.root.join("plugins/drift.status")).unwrap();
    symlink(
        target.join("data"),
        fixture.installed(".BarWidget.qml.drift-tmp"),
    )
    .unwrap();
    let mut omarchy = Fake::new();
    let (result, _, _) = go(
        widget_install,
        &fixture,
        &mut omarchy,
        options(WidgetMode::Yes),
        false,
    );
    assert_eq!(result, Ok(WidgetOutcome::Synced));
    assert_eq!(fs::read_to_string(target.join("data")).unwrap(), "MINE\n");
}
