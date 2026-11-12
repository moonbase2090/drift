use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use drift_apply::{
    apply_checked, touches_hypr_config, undo, ApplyError, CheckedApply, FailAt, Hyprctl,
    HyprlandCheck, HyprlandStatus, Payload, Request, Roots, UndoOutcome, UndoRequest,
    WriteFollowup,
};
use drift_core::{Action, Plan, PlannedFile};
use sha2::{Digest, Sha256};

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("drift-hypr-{nanos}"));
        for dir in ["live", "state", "runtime", "pacman"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        Self { root }
    }

    fn roots(&self) -> Roots {
        Roots {
            live: self.root.join("live"),
            state: self.root.join("state"),
            omarchy_lock: self.root.join("runtime/omarchy-update.lock"),
            pacman_lock: self.root.join("pacman/db.lck"),
        }
    }

    fn live(&self, path: &str) -> PathBuf {
        self.root.join("live").join(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Scripted {
    status: HyprlandStatus,
    calls: AtomicUsize,
}

impl HyprlandCheck for Scripted {
    fn config_errors(&self) -> HyprlandStatus {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.status.clone()
    }
}

fn write_live(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

struct Write<'a> {
    id: &'a str,
    path: &'a str,
    before: Option<&'a [u8]>,
    after: &'a [u8],
    action: Action,
    fail_at: Option<FailAt>,
}

fn apply_bytes(
    fix: &Fixture,
    write: Write<'_>,
    checker: &dyn HyprlandCheck,
) -> Result<CheckedApply, ApplyError> {
    if let Some(bytes) = write.before {
        write_live(&fix.live(write.path), bytes);
    }
    let prior = write.before.map(sha);
    let payload_bytes: &[u8] = if write.action == Action::KeepMine {
        write.before.expect("keep mine has prior bytes")
    } else {
        write.after
    };
    let file = PlannedFile::new(
        write.path,
        write.action,
        prior.as_deref(),
        Some(&sha(payload_bytes)),
        Some(0o644),
        None,
    )
    .unwrap();
    let plan = Plan::new(write.id, "2026-04-02T00:00:00Z", "4.5.0-3", vec![file], &[]).unwrap();
    let files = [Payload {
        path: write.path,
        bytes: payload_bytes,
        volatile: false,
        allow_volatile: false,
        allow_symlink: false,
    }];
    apply_checked(
        &fix.roots(),
        &Request {
            plan: &plan,
            files: &files,
            fail_at: write.fail_at,
        },
        checker,
    )
}

fn undo_of<'a>(target: &'a str, id: &'a str) -> UndoRequest<'a> {
    UndoRequest {
        target,
        id,
        time: "2026-05-01T00:00:00Z",
        package_version: "4.5.0-3",
        restore_anyway: &[],
        fail_at: None,
    }
}

fn fake_hyprctl(dir: &Path, stdout: &str, code: i32) -> PathBuf {
    fs::write(dir.join("out"), stdout).unwrap();
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$1\" > \"$(dirname \"$0\")/argv\"\nif [ \"$1\" != configerrors ]; then\n  exit 2\nfi\ncat \"$(dirname \"$0\")/out\"\nexit {code}\n"
    );
    let path = dir.join(format!("hyprctl-{code}-{}", stdout.len()));
    fs::write(&path, script).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    path
}

fn argv_of(program: &Path) -> String {
    fs::read_to_string(program.parent().unwrap().join("argv")).unwrap()
}

#[test]
fn only_hypr_config_paths_count() {
    assert!(touches_hypr_config("config/hypr/monitors.lua"));
    assert!(touches_hypr_config(".config/hypr/hyprland.lua"));
    assert!(touches_hypr_config("hypr/looknfeel.lua"));
    assert!(!touches_hypr_config("config/omarchy/shell.json"));
    assert!(!touches_hypr_config("config/hyprland.lua"));
    assert!(!touches_hypr_config("config/hypr"));
    assert!(!touches_hypr_config("other/hypr/note.lua"));
    assert!(!touches_hypr_config("hypr"));
}

#[test]
fn the_shipped_checker_is_hyprctl_configerrors() {
    assert_eq!(Hyprctl::new().program(), std::ffi::OsStr::new("hyprctl"));
}

#[test]
fn a_fake_hyprctl_covers_no_errors_errors_and_a_missing_binary() {
    let fix = Fixture::new();
    for phrase in [
        "",
        "ok\n",
        "  OK  \n",
        "no errors\n",
        "no config errors\n",
        "config has no errors\n",
    ] {
        let program = fake_hyprctl(&fix.root, phrase, 0);
        assert_eq!(
            Hyprctl::at(program.as_os_str()).config_errors(),
            HyprlandStatus::Clean,
            "{phrase:?}"
        );
        assert_eq!(argv_of(&program).trim(), "configerrors");
    }
    let down = fake_hyprctl(&fix.root, "", 1);
    assert_eq!(
        Hyprctl::at(down.as_os_str()).config_errors(),
        HyprlandStatus::Clean
    );
    let broken = fake_hyprctl(&fix.root, "Config error in file hyprland.lua: bad\n", 1);
    assert_eq!(
        Hyprctl::at(broken.as_os_str()).config_errors(),
        HyprlandStatus::Errors("Config error in file hyprland.lua: bad".to_string())
    );
    assert_eq!(
        Hyprctl::at(fix.root.join("missing-hyprctl").as_os_str()).config_errors(),
        HyprlandStatus::Missing
    );
    assert!(matches!(
        Hyprctl::at(fix.root.as_os_str()).config_errors(),
        HyprlandStatus::Errors(_)
    ));
}

#[test]
fn no_errors_leave_the_write_in_place_without_an_offer() {
    let fix = Fixture::new();
    let program = fake_hyprctl(&fix.root, "ok\n", 0);
    let checked = apply_bytes(
        &fix,
        Write {
            id: "txn-clean",
            path: "config/hypr/monitors.lua",
            before: Some(b"mine\n"),
            after: b"theirs\n",
            action: Action::TakeUpstream,
            fail_at: None,
        },
        &Hyprctl::at(program.as_os_str()),
    )
    .unwrap();
    assert_eq!(checked.followup, WriteFollowup::Clear);
    assert!(checked.followup.alert().is_none());
    assert_eq!(
        fs::read(fix.live("config/hypr/monitors.lua")).unwrap(),
        b"theirs\n"
    );
    assert_eq!(argv_of(&program).trim(), "configerrors");
}

#[test]
fn reported_errors_offer_undo_and_that_undo_restores_the_previous_bytes() {
    let fix = Fixture::new();
    let path = "config/hypr/monitors.lua";
    write_live(&fix.live(path), b"mine\n");
    let mode = fs::symlink_metadata(fix.live(path))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    let program = fake_hyprctl(
        &fix.root,
        "Config error in file hyprland.lua at line 3: unknown\n",
        0,
    );
    let checked = apply_bytes(
        &fix,
        Write {
            id: "txn-bad",
            path,
            before: Some(b"mine\n"),
            after: b"theirs\n",
            action: Action::TakeUpstream,
            fail_at: None,
        },
        &Hyprctl::at(program.as_os_str()),
    )
    .unwrap();
    assert_eq!(fs::read(fix.live(path)).unwrap(), b"theirs\n");
    let WriteFollowup::OfferUndo {
        transaction,
        errors,
    } = &checked.followup
    else {
        panic!("{:?}", checked.followup);
    };
    assert_eq!(transaction, "txn-bad");
    assert!(errors.contains("unknown"), "{errors}");
    let alert = checked.followup.alert().expect("alert");
    assert!(alert.contains("Undo this transaction?"), "{alert}");
    assert!(alert.contains(transaction), "{alert}");
    assert!(alert.contains("unknown"), "{alert}");
    let outcome = undo(&fix.roots(), &undo_of(transaction, "undo-bad")).unwrap();
    let UndoOutcome::Restored(report) = outcome else {
        panic!("expected a restore, got {outcome:?}");
    };
    assert_eq!(report.applied, [path]);
    assert_eq!(fs::read(fix.live(path)).unwrap(), b"mine\n");
    assert_eq!(
        fs::symlink_metadata(fix.live(path))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        mode
    );
}

#[test]
fn a_missing_binary_is_skipped_and_the_write_stays() {
    let fix = Fixture::new();
    let checked = apply_bytes(
        &fix,
        Write {
            id: "txn-miss",
            path: "config/hypr/monitors.lua",
            before: Some(b"mine\n"),
            after: b"theirs\n",
            action: Action::TakeUpstream,
            fail_at: None,
        },
        &Hyprctl::at(fix.root.join("missing-hyprctl").as_os_str()),
    )
    .unwrap();
    assert_eq!(checked.followup, WriteFollowup::Clear);
    assert!(checked.followup.alert().is_none());
    assert_eq!(
        fs::read(fix.live("config/hypr/monitors.lua")).unwrap(),
        b"theirs\n"
    );
}

#[test]
fn an_injected_checker_offers_undo_without_a_compositor() {
    let fix = Fixture::new();
    let checker = Scripted {
        status: HyprlandStatus::Errors("bad bind".to_string()),
        calls: AtomicUsize::new(0),
    };
    let checked = apply_bytes(
        &fix,
        Write {
            id: "txn-inject",
            path: ".config/hypr/hyprland.lua",
            before: Some(b"old\n"),
            after: b"new\n",
            action: Action::TakeUpstream,
            fail_at: None,
        },
        &checker,
    )
    .unwrap();
    assert_eq!(checker.calls.load(Ordering::SeqCst), 1);
    let WriteFollowup::OfferUndo {
        transaction,
        errors,
    } = checked.followup
    else {
        panic!("expected an offer");
    };
    assert_eq!(transaction, "txn-inject");
    assert_eq!(errors, "bad bind");
    let outcome = undo(&fix.roots(), &undo_of(&transaction, "undo-inject")).unwrap();
    assert!(matches!(outcome, UndoOutcome::Restored(_)));
    assert_eq!(
        fs::read(fix.live(".config/hypr/hyprland.lua")).unwrap(),
        b"old\n"
    );
}

#[test]
fn a_hypr_prefix_write_asks_the_checker() {
    let fix = Fixture::new();
    let checker = Scripted {
        status: HyprlandStatus::Clean,
        calls: AtomicUsize::new(0),
    };
    let checked = apply_bytes(
        &fix,
        Write {
            id: "txn-prefix",
            path: "hypr/looknfeel.lua",
            before: Some(b"old\n"),
            after: b"new\n",
            action: Action::TakeUpstream,
            fail_at: None,
        },
        &checker,
    )
    .unwrap();
    assert_eq!(checker.calls.load(Ordering::SeqCst), 1);
    assert_eq!(checked.followup, WriteFollowup::Clear);
    assert_eq!(fs::read(fix.live("hypr/looknfeel.lua")).unwrap(), b"new\n");
}

#[test]
fn a_write_outside_hypr_does_not_ask() {
    let fix = Fixture::new();
    let checker = Scripted {
        status: HyprlandStatus::Errors("should not run".to_string()),
        calls: AtomicUsize::new(0),
    };
    let checked = apply_bytes(
        &fix,
        Write {
            id: "txn-other",
            path: "config/omarchy/shell.json",
            before: Some(b"{}\n"),
            after: b"{}\nchanged\n",
            action: Action::TakeUpstream,
            fail_at: None,
        },
        &checker,
    )
    .unwrap();
    assert_eq!(checker.calls.load(Ordering::SeqCst), 0);
    assert_eq!(checked.followup, WriteFollowup::Clear);
    assert_eq!(
        fs::read(fix.live("config/omarchy/shell.json")).unwrap(),
        b"{}\nchanged\n"
    );
}

#[test]
fn keep_mine_does_not_ask() {
    let fix = Fixture::new();
    let checker = Scripted {
        status: HyprlandStatus::Errors("should not run".to_string()),
        calls: AtomicUsize::new(0),
    };
    let checked = apply_bytes(
        &fix,
        Write {
            id: "txn-keep",
            path: "config/hypr/monitors.lua",
            before: Some(b"mine\n"),
            after: b"theirs\n",
            action: Action::KeepMine,
            fail_at: None,
        },
        &checker,
    )
    .unwrap();
    assert_eq!(checker.calls.load(Ordering::SeqCst), 0);
    assert_eq!(checked.followup, WriteFollowup::Clear);
    assert_eq!(
        fs::read(fix.live("config/hypr/monitors.lua")).unwrap(),
        b"mine\n"
    );
}

#[test]
fn a_failed_apply_does_not_ask() {
    let fix = Fixture::new();
    let checker = Scripted {
        status: HyprlandStatus::Errors("should not run".to_string()),
        calls: AtomicUsize::new(0),
    };
    let error = apply_bytes(
        &fix,
        Write {
            id: "txn-fail",
            path: "config/hypr/monitors.lua",
            before: Some(b"mine\n"),
            after: b"theirs\n",
            action: Action::TakeUpstream,
            fail_at: Some(FailAt::Manifest),
        },
        &checker,
    )
    .unwrap_err();
    assert!(error.message.contains("manifest"), "{error}");
    assert_eq!(checker.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        fs::read(fix.live("config/hypr/monitors.lua")).unwrap(),
        b"mine\n"
    );
}
