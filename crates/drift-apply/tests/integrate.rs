use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use drift_apply::{
    integrate, undo, IntegrateMode, IntegrateOutcome, Roots, UndoOutcome, UndoRequest,
};
use drift_core::{HOOK_PATH, MENU_PATH};

static TEMPS: AtomicU64 = AtomicU64::new(0);

const STOCK: &str = "{\n  // Extend the menu.\n  // \"about\": {\"label\": \"About\"},\n}\n";

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let n = TEMPS.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "drift-integrate-{}-{n}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for dir in [
            "live/extensions",
            "live/hooks/post-update.d",
            "state",
            "runtime",
        ] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        fs::write(root.join("live").join(MENU_PATH), STOCK).unwrap();
        Self { root }
    }

    fn roots(&self) -> Roots {
        Roots {
            live: self.root.join("live"),
            state: self.root.join("state"),
            omarchy_lock: self.root.join("runtime/omarchy-update.lock"),
            pacman_lock: self.root.join("runtime/db.lck"),
        }
    }

    fn live(&self, path: &str) -> PathBuf {
        self.root.join("live").join(path)
    }

    fn transactions(&self) -> usize {
        fs::read_dir(self.root.join("state/backups"))
            .map(|dir| dir.count())
            .unwrap_or(0)
    }

    fn go(
        &self,
        id: &str,
        mode: IntegrateMode,
        answer: bool,
    ) -> (Result<IntegrateOutcome, String>, String) {
        let mut out = Vec::new();
        let mut asked = 0;
        let result = integrate(
            &self.roots(),
            id,
            "2026-06-01T00:00:00Z",
            mode,
            &mut out,
            &mut || {
                asked += 1;
                answer
            },
        );
        if mode != IntegrateMode::Confirm {
            assert_eq!(asked, 0);
        }
        (result, String::from_utf8(out).unwrap())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn integrating_adds_the_row_and_the_hook_and_records_a_transaction() {
    let fix = Fixture::new();
    let (result, shown) = fix.go("integrate-1", IntegrateMode::Yes, false);
    let IntegrateOutcome::Applied(report) = result.unwrap() else {
        panic!("expected an apply");
    };
    assert_eq!(report.applied, [MENU_PATH, HOOK_PATH]);
    let menu = fs::read_to_string(fix.live(MENU_PATH)).unwrap();
    assert!(menu.starts_with("{\n  // Extend the menu.\n"), "{menu}");
    assert!(menu.contains("\"update.drift\""), "{menu}");
    assert_eq!(
        fs::read_to_string(fix.live(HOOK_PATH)).unwrap(),
        "#!/bin/bash\ndrift notify\n"
    );
    let mode = fs::metadata(fix.live(HOOK_PATH))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o755);
    assert!(
        shown.contains("--- a/extensions/omarchy-menu.jsonc"),
        "{shown}"
    );
    assert!(
        shown.contains("--- /dev/null\n+++ b/hooks/post-update.d/drift"),
        "{shown}"
    );
    assert!(shown.contains("new file, mode 0755"), "{shown}");
    assert!(fix
        .root
        .join("state/backups/integrate-1/manifest.json")
        .is_file());
    assert!(fix
        .root
        .join("state/backups/integrate-1/extensions/omarchy-menu.jsonc")
        .is_file());
}

#[test]
fn running_twice_changes_nothing_and_records_nothing() {
    let fix = Fixture::new();
    fix.go("integrate-1", IntegrateMode::Yes, false).0.unwrap();
    let menu = fs::read(fix.live(MENU_PATH)).unwrap();
    let hook = fs::read(fix.live(HOOK_PATH)).unwrap();
    let (result, shown) = fix.go("integrate-2", IntegrateMode::Yes, false);
    assert_eq!(result.unwrap(), IntegrateOutcome::AlreadyDone);
    assert!(shown.contains("already integrated"), "{shown}");
    assert_eq!(fs::read(fix.live(MENU_PATH)).unwrap(), menu);
    assert_eq!(fs::read(fix.live(HOOK_PATH)).unwrap(), hook);
    assert_eq!(fix.transactions(), 1);
    let (result, _) = fix.go("integrate-3", IntegrateMode::Confirm, false);
    assert_eq!(result.unwrap(), IntegrateOutcome::AlreadyDone);
}

#[test]
fn declining_or_a_dry_run_writes_nothing_at_all() {
    let fix = Fixture::new();
    let (result, shown) = fix.go("integrate-1", IntegrateMode::Confirm, false);
    assert_eq!(result.unwrap(), IntegrateOutcome::Declined);
    assert!(shown.contains("Apply these changes? [y/N]"), "{shown}");
    assert!(shown.contains("Nothing was written"), "{shown}");
    let (result, shown) = fix.go("integrate-1", IntegrateMode::DryRun, true);
    assert_eq!(result.unwrap(), IntegrateOutcome::DryRun);
    assert!(shown.contains("Dry run"), "{shown}");
    assert_eq!(fs::read_to_string(fix.live(MENU_PATH)).unwrap(), STOCK);
    assert!(!fix.live(HOOK_PATH).exists());
    assert_eq!(fix.transactions(), 0);
    assert!(!fix.root.join("state/lock").exists());
}

#[test]
fn confirming_applies() {
    let fix = Fixture::new();
    let (result, _) = fix.go("integrate-1", IntegrateMode::Confirm, true);
    assert!(matches!(result.unwrap(), IntegrateOutcome::Applied(_)));
    assert!(fix.live(HOOK_PATH).is_file());
}

#[test]
fn undo_removes_both_changes() {
    let fix = Fixture::new();
    fix.go("integrate-1", IntegrateMode::Yes, false).0.unwrap();
    let outcome = undo(
        &fix.roots(),
        &UndoRequest {
            target: "integrate-1",
            id: "undo-1",
            time: "2026-06-02T00:00:00Z",
            package_version: "0",
            restore_anyway: &[],
            fail_at: None,
        },
    )
    .unwrap();
    assert!(matches!(outcome, UndoOutcome::Restored(_)));
    assert_eq!(fs::read_to_string(fix.live(MENU_PATH)).unwrap(), STOCK);
    assert!(!fix.live(HOOK_PATH).exists());
    let (result, _) = fix.go("integrate-2", IntegrateMode::Yes, false);
    assert!(matches!(result.unwrap(), IntegrateOutcome::Applied(_)));
}

#[test]
fn a_missing_menu_file_is_created_and_undone() {
    let fix = Fixture::new();
    fs::remove_file(fix.live(MENU_PATH)).unwrap();
    fix.go("integrate-1", IntegrateMode::Yes, false).0.unwrap();
    let menu = fs::read_to_string(fix.live(MENU_PATH)).unwrap();
    assert!(menu.starts_with("{\n  \"update.drift\""), "{menu}");
    undo(
        &fix.roots(),
        &UndoRequest {
            target: "integrate-1",
            id: "undo-1",
            time: "2026-06-02T00:00:00Z",
            package_version: "0",
            restore_anyway: &[],
            fail_at: None,
        },
    )
    .unwrap();
    assert!(!fix.live(MENU_PATH).exists());
    assert!(!fix.live(HOOK_PATH).exists());
}

#[test]
fn a_foreign_hook_or_a_symlinked_file_blocks_everything() {
    let fix = Fixture::new();
    fs::write(fix.live(HOOK_PATH), "#!/bin/sh\necho mine\n").unwrap();
    let (result, _) = fix.go("integrate-1", IntegrateMode::Yes, false);
    assert!(result.unwrap_err().contains("already exists"));
    assert_eq!(fs::read_to_string(fix.live(MENU_PATH)).unwrap(), STOCK);
    assert_eq!(fix.transactions(), 0);

    let linked = Fixture::new();
    fs::write(linked.root.join("real.jsonc"), STOCK).unwrap();
    fs::remove_file(linked.live(MENU_PATH)).unwrap();
    symlink(linked.root.join("real.jsonc"), linked.live(MENU_PATH)).unwrap();
    let (result, _) = linked.go("integrate-1", IntegrateMode::Yes, false);
    assert!(result.unwrap_err().contains("symlink"));
    assert_eq!(
        fs::read_to_string(linked.root.join("real.jsonc")).unwrap(),
        STOCK
    );
    assert!(!linked.live(HOOK_PATH).exists());
}

#[test]
fn an_unparseable_menu_file_is_left_alone() {
    let fix = Fixture::new();
    fs::write(fix.live(MENU_PATH), "{ \"a\": ").unwrap();
    let (result, _) = fix.go("integrate-1", IntegrateMode::Yes, false);
    assert!(result.unwrap_err().contains("cannot integrate"));
    assert_eq!(
        fs::read_to_string(fix.live(MENU_PATH)).unwrap(),
        "{ \"a\": "
    );
    assert!(!fix.live(HOOK_PATH).exists());
}

#[test]
fn a_held_update_lock_refuses_the_write() {
    let fix = Fixture::new();
    let held = fs::File::create(fix.root.join("runtime/omarchy-update.lock")).unwrap();
    assert_eq!(
        unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    let (result, _) = fix.go("integrate-1", IntegrateMode::Yes, false);
    assert!(result.unwrap_err().contains("omarchy update lock"));
    assert_eq!(fs::read_to_string(fix.live(MENU_PATH)).unwrap(), STOCK);
    assert!(!fix.live(HOOK_PATH).exists());
}

fn set_mode(path: &std::path::Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

fn mode_of(path: &std::path::Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn a_non_executable_hook_with_the_right_content_is_fixed_not_skipped() {
    let fix = Fixture::new();
    fix.go("integrate-1", IntegrateMode::Yes, false).0.unwrap();
    let hook = fix.live(HOOK_PATH);
    set_mode(&hook, 0o644);
    let (result, shown) = fix.go("integrate-2", IntegrateMode::Yes, false);
    let IntegrateOutcome::Applied(report) = result.unwrap() else {
        panic!("expected a fix");
    };
    assert_eq!(report.applied, [HOOK_PATH]);
    assert_eq!(mode_of(&hook), 0o755);
    assert_eq!(
        fs::read_to_string(&hook).unwrap(),
        "#!/bin/bash\ndrift notify\n"
    );
    assert!(shown.contains("mode 0644 -> 0755"), "{shown}");
    assert!(!shown.contains("already integrated"), "{shown}");
}

#[test]
fn fixing_the_hook_mode_can_be_undone_and_is_declinable() {
    let fix = Fixture::new();
    fix.go("integrate-1", IntegrateMode::Yes, false).0.unwrap();
    let hook = fix.live(HOOK_PATH);
    set_mode(&hook, 0o600);
    let (result, shown) = fix.go("integrate-2", IntegrateMode::DryRun, false);
    assert_eq!(result.unwrap(), IntegrateOutcome::DryRun);
    assert!(shown.contains("mode 0600 -> 0755"), "{shown}");
    assert_eq!(mode_of(&hook), 0o600);
    let (result, _) = fix.go("integrate-2", IntegrateMode::Confirm, false);
    assert_eq!(result.unwrap(), IntegrateOutcome::Declined);
    assert_eq!(mode_of(&hook), 0o600);
    fix.go("integrate-3", IntegrateMode::Yes, false).0.unwrap();
    assert_eq!(mode_of(&hook), 0o755);
    undo(
        &fix.roots(),
        &UndoRequest {
            target: "integrate-3",
            id: "undo-1",
            time: "2026-06-02T00:00:00Z",
            package_version: "0",
            restore_anyway: &[],
            fail_at: None,
        },
    )
    .unwrap();
    assert_eq!(mode_of(&hook), 0o600);
}

#[test]
fn an_owner_executable_hook_is_left_alone() {
    let fix = Fixture::new();
    fix.go("integrate-1", IntegrateMode::Yes, false).0.unwrap();
    let hook = fix.live(HOOK_PATH);
    for mode in [0o700, 0o750, 0o755, 0o775, 0o500] {
        set_mode(&hook, mode);
        let (result, _) = fix.go("integrate-2", IntegrateMode::Yes, false);
        assert_eq!(result.unwrap(), IntegrateOutcome::AlreadyDone, "{mode:o}");
        assert_eq!(mode_of(&hook), mode);
    }
    for mode in [0o644, 0o610, 0o444] {
        set_mode(&hook, mode);
        let (result, _) = fix.go("integrate-3", IntegrateMode::DryRun, false);
        assert_eq!(result.unwrap(), IntegrateOutcome::DryRun, "{mode:o}");
    }
    for mode in [0o011, 0o000] {
        set_mode(&hook, mode);
        let (result, _) = fix.go("integrate-4", IntegrateMode::Yes, false);
        assert!(result.is_err(), "{mode:o}");
        assert_eq!(mode_of(&hook), mode);
    }
}

#[test]
fn a_menu_file_with_an_escaped_id_is_left_alone() {
    let fix = Fixture::new();
    let text = "{\n  \"update\\u002edrift\": {\"label\": \"Mine\"}\n}\n";
    fs::write(fix.live(MENU_PATH), text).unwrap();
    let (result, _) = fix.go("integrate-1", IntegrateMode::Yes, false);
    let IntegrateOutcome::Applied(report) = result.unwrap() else {
        panic!("the hook is still new");
    };
    assert_eq!(report.applied, [HOOK_PATH]);
    assert_eq!(fs::read_to_string(fix.live(MENU_PATH)).unwrap(), text);
}
