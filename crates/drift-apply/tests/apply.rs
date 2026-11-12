use std::fs::{self, File};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use drift_apply::{apply, ApplyError, FailAt, FileStep, Payload, Request, Roots};
use drift_core::{Action, Plan, PlannedFile};

const BEFORE: &str = "73cb3858a687a8494ca3323053016282f3dad39d42cf62ca4e79dda2aac7d9ac";
const AFTER: &str = "98ea6e4f216f2fb4b69fff9b3a44842c38686ca685f3f55dc48c5d3fb1107be4";

static TEMPS: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let n = TEMPS.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("drift-apply-{nanos}-{n}"));
        fs::create_dir_all(root.join("live")).unwrap();
        fs::create_dir_all(root.join("state")).unwrap();
        fs::create_dir_all(root.join("runtime")).unwrap();
        fs::create_dir_all(root.join("pacman")).unwrap();
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

fn planned(
    path: &str,
    before: Option<&str>,
    after: Option<&str>,
    mode: Option<u32>,
    backup: Option<&str>,
) -> PlannedFile {
    PlannedFile::new(path, Action::TakeUpstream, before, after, mode, backup).unwrap()
}

fn one_plan(file: PlannedFile, prune: &[&str]) -> Plan {
    Plan::new(
        "txn-7",
        "2026-04-02T00:00:00Z",
        "4.5.0-3",
        vec![file],
        prune,
    )
    .unwrap()
}

fn payload<'a>(path: &'a str, bytes: &'a [u8]) -> Payload<'a> {
    Payload {
        path,
        bytes,
        volatile: false,
        allow_volatile: false,
        allow_symlink: false,
    }
}

fn write_live(path: &Path, bytes: &[u8], mode: u32) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

fn mode_of(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
}

fn apply_one(
    fix: &Fixture,
    plan: &Plan,
    bytes: &[u8],
) -> Result<drift_apply::ApplyReport, ApplyError> {
    let files = [payload("config/hypr/monitors.lua", bytes)];
    apply(
        &fix.roots(),
        &Request {
            plan,
            files: &files,
            fail_at: None,
        },
    )
}

#[test]
fn a_file_is_backed_up_and_replaced_without_following_links() {
    let fix = Fixture::new();
    let path = "config/hypr/monitors.lua";
    write_live(&fix.live(path), b"x\n", 0o644);
    let plan = one_plan(
        planned(
            path,
            Some(BEFORE),
            Some(AFTER),
            Some(0o640),
            Some("backups/txn-7/config/hypr/monitors.lua"),
        ),
        &[],
    );
    let report = apply_one(&fix, &plan, b"hi\n").unwrap();
    assert_eq!(report.applied, [path]);
    assert!(report.pruned.is_empty());
    assert_eq!(fs::read(fix.live(path)).unwrap(), b"hi\n");
    assert_eq!(mode_of(&fix.live(path)), 0o640);
    let backup = fix
        .root
        .join("state/backups/txn-7/config/hypr/monitors.lua");
    assert_eq!(fs::read(backup).unwrap(), b"x\n");
    let manifest = fs::read_to_string(fix.root.join("state/backups/txn-7/manifest.json")).unwrap();
    assert!(manifest.contains("txn-7"));
    assert!(manifest.contains(BEFORE));
    assert!(manifest.contains(AFTER));
    assert!(!fix.roots().state.join("lock").exists());
    assert!(leftovers(&fix.roots().live).is_empty());
}

#[test]
fn a_stale_hash_leaves_every_unwritten_file_unchanged() {
    let fix = Fixture::new();
    let first = "config/hypr/monitors.lua";
    let second = "config/hypr/looknfeel.lua";
    write_live(&fix.live(first), b"x\n", 0o644);
    write_live(&fix.live(second), b"x\n", 0o644);
    let plan = Plan::new(
        "txn-7",
        "2026-04-02T00:00:00Z",
        "4.5.0-3",
        vec![
            planned(first, Some(BEFORE), Some(AFTER), Some(0o644), None),
            planned(second, Some(AFTER), Some(BEFORE), Some(0o644), None),
        ],
        &[],
    )
    .unwrap();
    let files = [payload(first, b"hi\n"), payload(second, b"x\n")];
    let error = apply(
        &fix.roots(),
        &Request {
            plan: &plan,
            files: &files,
            fail_at: None,
        },
    )
    .unwrap_err();
    assert!(error.message.contains("stale hash"), "{}", error.message);
    assert_eq!(error.applied, [first]);
    assert_eq!(error.unapplied[0].path, second);
    assert!(error.unapplied[0].reason.contains("stale hash"));
    assert_eq!(fs::read(fix.live(first)).unwrap(), b"hi\n");
    assert_eq!(fs::read(fix.live(second)).unwrap(), b"x\n");
}

#[test]
fn a_wrong_after_hash_writes_nothing() {
    let fix = Fixture::new();
    let path = "config/hypr/monitors.lua";
    write_live(&fix.live(path), b"x\n", 0o644);
    let plan = one_plan(
        planned(path, Some(BEFORE), Some(BEFORE), Some(0o644), None),
        &[],
    );
    let error = apply_one(&fix, &plan, b"hi\n").unwrap_err();
    assert!(error.message.contains("SHA-256"), "{}", error.message);
    assert!(error.applied.is_empty());
    assert_eq!(error.unapplied[0].path, path);
    assert_eq!(fs::read(fix.live(path)).unwrap(), b"x\n");
}

#[test]
fn symlink_targets_and_volatile_files_are_refused_without_an_override() {
    let fix = Fixture::new();
    let outside = fix.root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("victim"), b"original").unwrap();
    fs::create_dir_all(fix.live("config/hypr")).unwrap();
    symlink(outside.join("victim"), fix.live("config/hypr/monitors.lua")).unwrap();
    let path = "config/hypr/monitors.lua";
    let plan = one_plan(planned(path, None, Some(AFTER), Some(0o644), None), &[]);
    let error = apply_one(&fix, &plan, b"hi\n").unwrap_err();
    assert!(
        error.message.contains("refusing to follow a symlink"),
        "{}",
        error.message
    );
    assert_eq!(fs::read(outside.join("victim")).unwrap(), b"original");
    assert!(fix
        .live(path)
        .symlink_metadata()
        .unwrap()
        .file_type()
        .is_symlink());

    let regular = Fixture::new();
    let file = "config/hypr/monitors.lua";
    write_live(&regular.live(file), b"x\n", 0o644);
    let plan = one_plan(
        planned(file, Some(BEFORE), Some(AFTER), Some(0o644), None),
        &[],
    );
    let files = [Payload {
        volatile: true,
        ..payload(file, b"hi\n")
    }];
    let error = apply(
        &regular.roots(),
        &Request {
            plan: &plan,
            files: &files,
            fail_at: None,
        },
    )
    .unwrap_err();
    assert!(error.message.contains("volatile"), "{}", error.message);
    assert_eq!(fs::read(regular.live(file)).unwrap(), b"x\n");
}

#[test]
fn an_override_replaces_a_symlink_node_and_a_volatile_file() {
    let fix = Fixture::new();
    let outside = fix.root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("victim"), b"original").unwrap();
    symlink(&outside, fix.root.join("linked")).unwrap();
    symlink(fix.root.join("linked"), fix.live("config")).unwrap();
    let path = "config/victim";
    let plan = one_plan(planned(path, None, Some(AFTER), Some(0o644), None), &[]);
    let files = [payload(path, b"hi\n")];
    let error = apply(
        &fix.roots(),
        &Request {
            plan: &plan,
            files: &files,
            fail_at: None,
        },
    )
    .unwrap_err();
    assert!(
        error.message.contains("refusing to follow a symlink"),
        "{}",
        error.message
    );
    assert_eq!(fs::read(outside.join("victim")).unwrap(), b"original");

    let leaf = Fixture::new();
    fs::create_dir_all(leaf.live("config/hypr")).unwrap();
    symlink(outside.join("victim"), leaf.live(path)).unwrap();
    let plan = one_plan(planned(path, None, Some(AFTER), Some(0o644), None), &[]);
    let files = [Payload {
        allow_symlink: true,
        ..payload(path, b"hi\n")
    }];
    apply(
        &leaf.roots(),
        &Request {
            plan: &plan,
            files: &files,
            fail_at: None,
        },
    )
    .unwrap();
    assert!(!leaf
        .live(path)
        .symlink_metadata()
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(leaf.live(path)).unwrap(), b"hi\n");
    assert_eq!(fs::read(outside.join("victim")).unwrap(), b"original");

    let volatile = Fixture::new();
    write_live(&volatile.live(path), b"x\n", 0o644);
    let plan = one_plan(
        planned(path, Some(BEFORE), Some(AFTER), Some(0o644), None),
        &[],
    );
    let files = [Payload {
        volatile: true,
        allow_volatile: true,
        ..payload(path, b"hi\n")
    }];
    apply(
        &volatile.roots(),
        &Request {
            plan: &plan,
            files: &files,
            fail_at: None,
        },
    )
    .unwrap();
    assert_eq!(fs::read(volatile.live(path)).unwrap(), b"hi\n");
}

#[test]
fn foreign_locks_and_our_lock_refuse_the_write() {
    let path = "config/hypr/monitors.lua";
    let cases = [
        ("runtime/omarchy-update.lock", "omarchy update lock"),
        ("pacman/db.lck", "pacman database lock"),
        ("state/lock", "drift lock is held"),
    ];
    for (lock, phrase) in cases {
        let fix = Fixture::new();
        write_live(&fix.live(path), b"x\n", 0o644);
        let lock_path = fix.root.join(lock);
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        File::create(&lock_path).unwrap();
        let plan = one_plan(
            planned(path, Some(BEFORE), Some(AFTER), Some(0o644), None),
            &[],
        );
        let error = apply_one(&fix, &plan, b"hi\n").unwrap_err();
        assert!(error.message.contains(phrase), "{}", error.message);
        assert!(error.applied.is_empty());
        assert_eq!(error.unapplied[0].path, path);
        assert_eq!(fs::read(fix.live(path)).unwrap(), b"x\n");
    }
}

#[test]
fn each_step_before_rename_leaves_the_original() {
    let steps = [
        (FileStep::Backup, "backup"),
        (FileStep::Temp, "temp"),
        (FileStep::Sync, "sync"),
        (FileStep::Rename, "rename"),
    ];
    for (step, name) in steps {
        let fix = Fixture::new();
        let path = "config/hypr/monitors.lua";
        write_live(&fix.live(path), b"x\n", 0o644);
        let plan = one_plan(
            planned(path, Some(BEFORE), Some(AFTER), Some(0o644), None),
            &[],
        );
        let files = [payload(path, b"hi\n")];
        let error = apply(
            &fix.roots(),
            &Request {
                plan: &plan,
                files: &files,
                fail_at: Some(FailAt::File { index: 0, step }),
            },
        )
        .unwrap_err();
        assert!(error.message.contains(name), "{}", error.message);
        assert!(error.applied.is_empty());
        assert_eq!(error.unapplied[0].path, path);
        assert_eq!(fs::read(fix.live(path)).unwrap(), b"x\n");
        assert!(leftovers(&fix.roots().live).is_empty(), "{name}");
    }
}

#[test]
fn a_manifest_failure_keeps_the_new_bytes_and_skips_the_manifest() {
    let fix = Fixture::new();
    let path = "config/hypr/monitors.lua";
    write_live(&fix.live(path), b"x\n", 0o644);
    let plan = one_plan(
        planned(path, Some(BEFORE), Some(AFTER), Some(0o644), None),
        &[],
    );
    let files = [payload(path, b"hi\n")];
    let error = apply(
        &fix.roots(),
        &Request {
            plan: &plan,
            files: &files,
            fail_at: Some(FailAt::Manifest),
        },
    )
    .unwrap_err();
    assert!(error.message.contains("manifest"), "{}", error.message);
    assert_eq!(error.applied, [path]);
    assert_eq!(fs::read(fix.live(path)).unwrap(), b"hi\n");
    assert!(!fix.root.join("state/backups/txn-7/manifest.json").exists());
}

#[test]
fn the_101st_transaction_prunes_the_oldest_listed_on_the_plan() {
    let fix = Fixture::new();
    for n in 1..=100 {
        let id = format!("old-{n:03}");
        let dir = fix.root.join("state/backups").join(&id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("marker"), &id).unwrap();
        let time = format!("2026-01-01T00:00:{n:05}");
        fs::write(dir.join("manifest.json"), format!(r#"{{"time":"{time}"}}"#)).unwrap();
    }
    let path = "config/hypr/monitors.lua";
    write_live(&fix.live(path), b"x\n", 0o644);
    let wrong = one_plan(
        planned(path, Some(BEFORE), Some(AFTER), Some(0o644), None),
        &["old-002"],
    );
    let error = apply_one(&fix, &wrong, b"hi\n").unwrap_err();
    assert!(error.message.contains("prune list"), "{}", error.message);
    assert_eq!(fs::read(fix.live(path)).unwrap(), b"x\n");
    assert!(fix.root.join("state/backups/old-001/marker").is_file());

    let plan = one_plan(
        planned(path, Some(BEFORE), Some(AFTER), Some(0o644), None),
        &["old-001"],
    );
    let report = apply_one(&fix, &plan, b"hi\n").unwrap();
    assert_eq!(report.pruned, ["old-001"]);
    assert!(!fix.root.join("state/backups/old-001").exists());
    assert!(fix.root.join("state/backups/old-002/marker").is_file());
    assert!(fix.root.join("state/backups/txn-7/manifest.json").is_file());
    let mut ids = Vec::new();
    for entry in fs::read_dir(fix.root.join("state/backups")).unwrap() {
        ids.push(entry.unwrap().file_name().to_string_lossy().into_owned());
    }
    assert_eq!(ids.len(), 100, "{ids:?}");
}

#[test]
fn an_absent_file_is_created_and_an_existing_mode_is_kept() {
    let fix = Fixture::new();
    let path = "config/hypr/monitors.lua";
    let plan = one_plan(planned(path, None, Some(AFTER), Some(0o640), None), &[]);
    apply_one(&fix, &plan, b"hi\n").unwrap();
    assert_eq!(fs::read(fix.live(path)).unwrap(), b"hi\n");
    assert_eq!(mode_of(&fix.live(path)), 0o640);

    let kept = Fixture::new();
    write_live(&kept.live(path), b"x\n", 0o600);
    let plan = one_plan(planned(path, Some(BEFORE), Some(AFTER), None, None), &[]);
    apply_one(&kept, &plan, b"hi\n").unwrap();
    assert_eq!(mode_of(&kept.live(path)), 0o600);
    assert_eq!(fs::read(kept.live(path)).unwrap(), b"hi\n");
}

fn leftovers(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".drift-"))
            {
                found.push(path.clone());
            }
            if path.is_dir() {
                pending.push(path);
            }
        }
    }
    found
}
