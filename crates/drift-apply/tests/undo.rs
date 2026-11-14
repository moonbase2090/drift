use std::collections::BTreeMap;
use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use drift_apply::{
    apply, history, inspect_undo, undo, ApplyError, FailAt, FileStep, Payload, Request, Roots,
    UndoOutcome, UndoRequest, UndoStatus,
};
use drift_core::{Action, Plan, PlannedFile};
use sha2::{Digest, Sha256};

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
        let root = std::env::temp_dir().join(format!("drift-undo-{nanos}-{n}"));
        for dir in ["live/config", "state", "runtime", "pacman"] {
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

    fn state(&self, path: &str) -> PathBuf {
        self.root.join("state").join(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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

type Tree = BTreeMap<String, (Vec<u8>, u32, bool)>;

fn tree(fix: &Fixture) -> Tree {
    let mut out = BTreeMap::new();
    let mut pending = vec![fix.root.join("live")];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).unwrap();
            let rel = path
                .strip_prefix(fix.root.join("live"))
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if meta.file_type().is_symlink() {
                let target = fs::read_link(&path).unwrap();
                out.insert(rel, (target.as_os_str().as_bytes().to_vec(), 0, true));
            } else if meta.is_dir() {
                pending.push(path);
            } else {
                out.insert(rel, (fs::read(&path).unwrap(), mode_of(&path), false));
            }
        }
    }
    out
}

struct Edit<'a> {
    path: &'a str,
    bytes: &'a [u8],
    mode: Option<u32>,
}

fn edit<'a>(path: &'a str, bytes: &'a [u8], mode: Option<u32>) -> Edit<'a> {
    Edit { path, bytes, mode }
}

fn do_apply(fix: &Fixture, id: &str, n: u32, edits: &[Edit<'_>]) -> Result<(), ApplyError> {
    do_apply_with(fix, id, n, edits, false)
}

fn do_apply_with(
    fix: &Fixture,
    id: &str,
    n: u32,
    edits: &[Edit<'_>],
    allow_symlink: bool,
) -> Result<(), ApplyError> {
    let mut files = Vec::new();
    for item in edits {
        let before = fs::read(fix.live(item.path)).ok().map(|bytes| sha(&bytes));
        files.push(
            PlannedFile::new(
                item.path,
                Action::TakeUpstream,
                before.as_deref(),
                Some(&sha(item.bytes)),
                item.mode,
                None,
            )
            .unwrap(),
        );
    }
    let plan = Plan::new(
        id,
        &format!("2026-04-02T00:00:{n:02}Z"),
        "4.5.0-3",
        files,
        &[],
    )
    .unwrap();
    let payloads: Vec<Payload<'_>> = edits
        .iter()
        .map(|item| Payload {
            path: item.path,
            bytes: item.bytes,
            volatile: false,
            allow_volatile: false,
            allow_symlink,
        })
        .collect();
    apply(
        &fix.roots(),
        &Request {
            plan: &plan,
            files: &payloads,
            fail_at: None,
        },
    )
    .map(|_| ())
}

fn request<'a>(target: &'a str, id: &'a str) -> UndoRequest<'a> {
    UndoRequest {
        target,
        id,
        time: "2026-05-01T00:00:00Z",
        package_version: "4.5.0-3",
        restore_anyway: &[],
        fail_at: None,
    }
}

fn do_undo(fix: &Fixture, target: &str, id: &str) -> UndoOutcome {
    undo(&fix.roots(), &request(target, id)).unwrap()
}

fn restored(outcome: UndoOutcome) -> Vec<String> {
    match outcome {
        UndoOutcome::Restored(report) => report.applied,
        other => panic!("expected a restore, got {other:?}"),
    }
}

const A: &str = "config/a.conf";
const B: &str = "config/b.conf";
const C: &str = "config/c.conf";

#[test]
fn undo_restores_bytes_and_mode_and_removes_what_apply_created() {
    let fix = Fixture::new();
    write_live(&fix.live(A), b"mine\n", 0o640);
    let before = tree(&fix);
    do_apply(
        &fix,
        "txn-1",
        1,
        &[
            edit(A, b"theirs\n", Some(0o600)),
            edit(B, b"new\n", Some(0o755)),
        ],
    )
    .unwrap();
    assert_eq!(mode_of(&fix.live(A)), 0o600);
    assert_ne!(tree(&fix), before);

    let applied = restored(do_undo(&fix, "txn-1", "undo-1"));
    assert_eq!(applied, [A, B]);
    assert_eq!(tree(&fix), before);
    assert_eq!(mode_of(&fix.live(A)), 0o640);
    assert!(!fix.live(B).exists());
}

#[test]
fn undo_is_recorded_as_a_transaction_with_its_own_backups() {
    let fix = Fixture::new();
    write_live(&fix.live(A), b"mine\n", 0o644);
    do_apply(&fix, "txn-1", 1, &[edit(A, b"theirs\n", None)]).unwrap();
    restored(do_undo(&fix, "txn-1", "undo-1"));

    let manifest = fs::read_to_string(fix.state("backups/undo-1/manifest.json")).unwrap();
    let plan = Plan::from_json(&manifest).unwrap();
    assert_eq!(plan.id(), "undo-1");
    assert_eq!(plan.time(), "2026-05-01T00:00:00Z");
    assert_eq!(plan.files().len(), 1);
    let file = &plan.files()[0];
    assert_eq!(file.action(), Action::Undo);
    assert_eq!(file.sha256_before(), Some(sha(b"theirs\n").as_str()));
    assert_eq!(file.sha256_after(), Some(sha(b"mine\n").as_str()));
    assert_eq!(file.mode(), Some(0o644));
    assert_eq!(
        fs::read(fix.state("backups/undo-1/config/a.conf")).unwrap(),
        b"theirs\n"
    );
    assert!(!fix.state("backups/undo-1/pending.json").exists());
    assert!(!fix.state("lock").exists());
}

#[test]
fn undo_of_undo_returns_to_the_applied_state() {
    let fix = Fixture::new();
    write_live(&fix.live(A), b"mine\n", 0o640);
    do_apply(
        &fix,
        "txn-1",
        1,
        &[
            edit(A, b"theirs\n", Some(0o600)),
            edit(B, b"new\n", Some(0o755)),
        ],
    )
    .unwrap();
    let applied_state = tree(&fix);
    restored(do_undo(&fix, "txn-1", "undo-1"));
    restored(do_undo(&fix, "undo-1", "undo-2"));
    assert_eq!(tree(&fix), applied_state);
    assert_eq!(mode_of(&fix.live(B)), 0o755);
    restored(do_undo(&fix, "undo-2", "undo-3"));
    assert!(!fix.live(B).exists());
    assert_eq!(fs::read(fix.live(A)).unwrap(), b"mine\n");
}

#[test]
fn a_random_sequence_of_actions_undoes_to_every_earlier_tree_and_back() {
    let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = move |bound: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % bound
    };
    let paths = [A, B, C, "config/d.conf", "config/e.conf"];
    let modes = [0o600, 0o640, 0o644, 0o755];
    let fix = Fixture::new();
    for path in &paths[..3] {
        let bytes = vec![b'a' + next(26) as u8; next(40) as usize + 1];
        write_live(&fix.live(path), &bytes, modes[next(4) as usize]);
    }
    let mut states = vec![tree(&fix)];
    let steps = 14;
    for step in 1..=steps {
        let count = next(3) + 1;
        let mut chosen: Vec<&str> = Vec::new();
        while chosen.len() < count as usize {
            let path = paths[next(5) as usize];
            if !chosen.contains(&path) {
                chosen.push(path);
            }
        }
        let contents: Vec<Vec<u8>> = chosen
            .iter()
            .map(|_| {
                (0..next(30))
                    .map(|_| b"ab\n \r\t\x00\xff"[next(8) as usize])
                    .collect()
            })
            .collect();
        let edits: Vec<Edit<'_>> = chosen
            .iter()
            .zip(&contents)
            .map(|(path, bytes)| {
                let mode = if next(2) == 0 {
                    Some(modes[next(4) as usize])
                } else {
                    None
                };
                edit(path, bytes, mode)
            })
            .collect();
        do_apply(&fix, &format!("txn-{step:02}"), step, &edits).unwrap();
        states.push(tree(&fix));
    }
    for step in (1..=steps).rev() {
        let outcome = do_undo(&fix, &format!("txn-{step:02}"), &format!("undo-{step:02}"));
        assert!(matches!(outcome, UndoOutcome::Restored(_)), "step {step}");
        assert_eq!(tree(&fix), states[step as usize - 1], "undo of step {step}");
    }
    for step in 1..=steps {
        let outcome = do_undo(&fix, &format!("undo-{step:02}"), &format!("redo-{step:02}"));
        assert!(matches!(outcome, UndoOutcome::Restored(_)), "redo {step}");
        assert_eq!(tree(&fix), states[step as usize], "redo of step {step}");
    }
}

#[test]
fn an_external_edit_warns_and_writes_nothing() {
    let fix = Fixture::new();
    write_live(&fix.live(A), b"mine\n", 0o644);
    do_apply(
        &fix,
        "txn-1",
        1,
        &[edit(A, b"theirs\n", None), edit(B, b"new\n", None)],
    )
    .unwrap();
    write_live(&fix.live(A), b"edited later\n", 0o600);
    let snapshot = tree(&fix);

    let outcome = do_undo(&fix, "txn-1", "undo-1");
    let UndoOutcome::Diverged(items) = outcome else {
        panic!("expected a warning");
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].path, A);
    let UndoStatus::Diverged(found) = &items[0].status else {
        panic!("expected a divergence");
    };
    assert_eq!(found.expected.as_deref(), Some(sha(b"theirs\n").as_str()));
    assert_eq!(
        found.found.as_deref(),
        Some(sha(b"edited later\n").as_str())
    );
    assert_eq!(tree(&fix), snapshot);
    assert!(!fix.state("backups/undo-1").exists());
    assert!(!fix.state("lock").exists());
}

#[test]
fn restore_anyway_backs_up_the_edited_bytes_first() {
    let fix = Fixture::new();
    write_live(&fix.live(A), b"mine\n", 0o644);
    do_apply(
        &fix,
        "txn-1",
        1,
        &[edit(A, b"theirs\n", None), edit(B, b"new\n", None)],
    )
    .unwrap();
    write_live(&fix.live(A), b"edited later\n", 0o600);

    let force = [A.to_string()];
    let outcome = undo(
        &fix.roots(),
        &UndoRequest {
            restore_anyway: &force,
            ..request("txn-1", "undo-1")
        },
    )
    .unwrap();
    assert_eq!(restored(outcome), [A, B]);
    assert_eq!(fs::read(fix.live(A)).unwrap(), b"mine\n");
    assert_eq!(mode_of(&fix.live(A)), 0o644);
    assert!(!fix.live(B).exists());
    assert_eq!(
        fs::read(fix.state("backups/undo-1/config/a.conf")).unwrap(),
        b"edited later\n"
    );
    assert_eq!(mode_of(&fix.state("backups/undo-1/config/a.conf")), 0o600);

    restored(do_undo(&fix, "undo-1", "undo-2"));
    assert_eq!(fs::read(fix.live(A)).unwrap(), b"edited later\n");
    assert_eq!(mode_of(&fix.live(A)), 0o600);
}

#[test]
fn a_created_file_is_removed_only_when_unchanged() {
    let fix = Fixture::new();
    do_apply(&fix, "txn-1", 1, &[edit(B, b"new\n", None)]).unwrap();
    write_live(&fix.live(B), b"user kept editing\n", 0o644);

    let UndoOutcome::Diverged(items) = do_undo(&fix, "txn-1", "undo-1") else {
        panic!("expected a warning");
    };
    assert_eq!(items[0].path, B);
    assert_eq!(fs::read(fix.live(B)).unwrap(), b"user kept editing\n");

    let force = [B.to_string()];
    let outcome = undo(
        &fix.roots(),
        &UndoRequest {
            restore_anyway: &force,
            ..request("txn-1", "undo-1")
        },
    )
    .unwrap();
    assert_eq!(restored(outcome), [B]);
    assert!(!fix.live(B).exists());
    assert_eq!(
        fs::read(fix.state("backups/undo-1/config/b.conf")).unwrap(),
        b"user kept editing\n"
    );
    restored(do_undo(&fix, "undo-1", "undo-2"));
    assert_eq!(fs::read(fix.live(B)).unwrap(), b"user kept editing\n");
}

#[test]
fn a_deleted_file_is_reported_and_restore_anyway_recreates_it() {
    let fix = Fixture::new();
    write_live(&fix.live(A), b"mine\n", 0o600);
    do_apply(&fix, "txn-1", 1, &[edit(A, b"theirs\n", None)]).unwrap();
    fs::remove_file(fix.live(A)).unwrap();

    let UndoOutcome::Diverged(items) = do_undo(&fix, "txn-1", "undo-1") else {
        panic!("expected a warning");
    };
    let UndoStatus::Diverged(found) = &items[0].status else {
        panic!("expected a divergence");
    };
    assert_eq!(found.found, None);

    let force = [A.to_string()];
    let outcome = undo(
        &fix.roots(),
        &UndoRequest {
            restore_anyway: &force,
            ..request("txn-1", "undo-1")
        },
    )
    .unwrap();
    assert_eq!(restored(outcome), [A]);
    assert_eq!(fs::read(fix.live(A)).unwrap(), b"mine\n");
    assert_eq!(mode_of(&fix.live(A)), 0o600);
    restored(do_undo(&fix, "undo-1", "undo-2"));
    assert!(!fix.live(A).exists());
}

#[test]
fn files_already_back_to_their_original_are_left_alone() {
    let fix = Fixture::new();
    write_live(&fix.live(A), b"mine\n", 0o644);
    do_apply(
        &fix,
        "txn-1",
        1,
        &[edit(A, b"theirs\n", None), edit(B, b"new\n", None)],
    )
    .unwrap();
    write_live(&fix.live(A), b"mine\n", 0o644);
    let items = inspect_undo(&fix.roots(), "txn-1").unwrap();
    assert_eq!(items[0].status, UndoStatus::AlreadyOriginal);
    assert_eq!(items[1].status, UndoStatus::Remove);
    assert_eq!(restored(do_undo(&fix, "txn-1", "undo-1")), [B]);
    let plan =
        Plan::from_json(&fs::read_to_string(fix.state("backups/undo-1/manifest.json")).unwrap())
            .unwrap();
    assert_eq!(plan.files().len(), 1);
    assert!(!fix.state("backups/undo-1/config/a.conf").exists());

    let error = undo(&fix.roots(), &request("txn-1", "undo-2")).unwrap_err();
    assert!(
        error.message.contains("nothing to undo"),
        "{}",
        error.message
    );
    assert!(!fix.state("backups/undo-2").exists());
}

#[test]
fn inspecting_is_read_only_and_names_each_status() {
    let fix = Fixture::new();
    write_live(&fix.live(A), b"mine\n", 0o644);
    write_live(&fix.live(C), b"keep\n", 0o644);
    let plan = Plan::new(
        "txn-1",
        "2026-04-02T00:00:00Z",
        "4.5.0-3",
        vec![
            PlannedFile::new(
                A,
                Action::TakeUpstream,
                Some(&sha(b"mine\n")),
                Some(&sha(b"theirs\n")),
                None,
                None,
            )
            .unwrap(),
            PlannedFile::new(
                B,
                Action::TakeUpstream,
                None,
                Some(&sha(b"new\n")),
                None,
                None,
            )
            .unwrap(),
            PlannedFile::new(
                C,
                Action::KeepMine,
                Some(&sha(b"keep\n")),
                Some(&sha(b"keep\n")),
                None,
                None,
            )
            .unwrap(),
        ],
        &[],
    )
    .unwrap();
    let files = [
        Payload {
            path: A,
            bytes: b"theirs\n",
            volatile: false,
            allow_volatile: false,
            allow_symlink: false,
        },
        Payload {
            path: B,
            bytes: b"new\n",
            volatile: false,
            allow_volatile: false,
            allow_symlink: false,
        },
        Payload {
            path: C,
            bytes: b"keep\n",
            volatile: false,
            allow_volatile: false,
            allow_symlink: false,
        },
    ];
    apply(
        &fix.roots(),
        &Request {
            plan: &plan,
            files: &files,
            fail_at: None,
        },
    )
    .unwrap();
    let snapshot = tree(&fix);
    let state_before = tree_of(&fix.root.join("state"));
    let items = inspect_undo(&fix.roots(), "txn-1").unwrap();
    let statuses: Vec<_> = items
        .iter()
        .map(|item| (item.path.as_str(), item.status.clone()))
        .collect();
    assert_eq!(
        statuses,
        [
            (A, UndoStatus::Restore),
            (B, UndoStatus::Remove),
            (C, UndoStatus::Kept)
        ]
    );
    assert_eq!(tree(&fix), snapshot);
    assert_eq!(tree_of(&fix.root.join("state")), state_before);
}

fn tree_of(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            out.push(path.to_string_lossy().into_owned());
            if path.is_dir() {
                pending.push(path);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_keep_mine_only_transaction_has_nothing_to_undo() {
    let fix = Fixture::new();
    write_live(&fix.live(C), b"keep\n", 0o644);
    let plan = Plan::new(
        "txn-1",
        "2026-04-02T00:00:00Z",
        "4.5.0-3",
        vec![PlannedFile::new(
            C,
            Action::KeepMine,
            Some(&sha(b"keep\n")),
            Some(&sha(b"keep\n")),
            None,
            None,
        )
        .unwrap()],
        &[],
    )
    .unwrap();
    let files = [Payload {
        path: C,
        bytes: b"keep\n",
        volatile: false,
        allow_volatile: false,
        allow_symlink: false,
    }];
    apply(
        &fix.roots(),
        &Request {
            plan: &plan,
            files: &files,
            fail_at: None,
        },
    )
    .unwrap();
    let error = undo(&fix.roots(), &request("txn-1", "undo-1")).unwrap_err();
    assert!(
        error.message.contains("nothing to undo"),
        "{}",
        error.message
    );
}

#[test]
fn unknown_transactions_paths_and_ids_are_refused() {
    let fix = Fixture::new();
    write_live(&fix.live(A), b"mine\n", 0o644);
    do_apply(&fix, "txn-1", 1, &[edit(A, b"theirs\n", None)]).unwrap();
    let snapshot = tree(&fix);

    let error = undo(&fix.roots(), &request("txn-9", "undo-1")).unwrap_err();
    assert!(
        error.message.contains("no such transaction"),
        "{}",
        error.message
    );
    for bad in ["", ".", "..", "a/b", "../txn-1"] {
        assert!(
            undo(&fix.roots(), &request(bad, "undo-1")).is_err(),
            "{bad:?}"
        );
    }
    let stray = ["config/zzz.conf".to_string()];
    let error = undo(
        &fix.roots(),
        &UndoRequest {
            restore_anyway: &stray,
            ..request("txn-1", "undo-1")
        },
    )
    .unwrap_err();
    assert!(error.message.contains("not part of"), "{}", error.message);
    let error = undo(&fix.roots(), &request("txn-1", "txn-1")).unwrap_err();
    assert!(
        error.message.contains("already exists"),
        "{}",
        error.message
    );
    assert_eq!(tree(&fix), snapshot);
}

#[test]
fn a_transaction_without_a_manifest_cannot_be_undone() {
    let fix = Fixture::new();
    fs::create_dir_all(fix.state("backups/half")).unwrap();
    fs::write(fix.state("backups/half/pending.json"), b"{}").unwrap();
    let error = undo(&fix.roots(), &request("half", "undo-1")).unwrap_err();
    assert!(error.message.contains("no manifest"), "{}", error.message);
}

#[test]
fn a_tampered_or_missing_backup_blocks_the_undo() {
    let fix = Fixture::new();
    write_live(&fix.live(A), b"mine\n", 0o644);
    do_apply(&fix, "txn-1", 1, &[edit(A, b"theirs\n", None)]).unwrap();
    let snapshot = tree(&fix);

    fs::write(fix.state("backups/txn-1/config/a.conf"), b"tampered\n").unwrap();
    let error = undo(&fix.roots(), &request("txn-1", "undo-1")).unwrap_err();
    assert!(
        error.message.contains("recorded digest"),
        "{}",
        error.message
    );
    assert_eq!(tree(&fix), snapshot);
    assert!(!fix.state("backups/undo-1").exists());

    fs::remove_file(fix.state("backups/txn-1/config/a.conf")).unwrap();
    let error = undo(&fix.roots(), &request("txn-1", "undo-1")).unwrap_err();
    assert!(
        error.message.contains("backup is missing"),
        "{}",
        error.message
    );
    assert_eq!(tree(&fix), snapshot);
}

#[test]
fn an_unlocked_persistent_update_lock_allows_undo() {
    let fix = Fixture::new();
    write_live(&fix.live(A), b"mine\n", 0o644);
    do_apply(&fix, "txn-1", 1, &[edit(A, b"theirs\n", None)]).unwrap();
    fs::write(fix.roots().omarchy_lock, b"").unwrap();
    restored(do_undo(&fix, "txn-1", "undo-1"));
    assert_eq!(fs::read(fix.live(A)).unwrap(), b"mine\n");
    assert!(fix.roots().omarchy_lock.exists());
}

#[test]
fn foreign_locks_and_our_lock_refuse_the_undo() {
    let fix = Fixture::new();
    write_live(&fix.live(A), b"mine\n", 0o644);
    do_apply(&fix, "txn-1", 1, &[edit(A, b"theirs\n", None)]).unwrap();
    let snapshot = tree(&fix);

    let held = fs::File::create(fix.root.join("runtime/omarchy-update.lock")).unwrap();
    assert_eq!(
        unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    let error = undo(&fix.roots(), &request("txn-1", "undo-1")).unwrap_err();
    assert!(error.message.contains("omarchy"), "{}", error.message);
    assert!(
        error.message.contains(
            &fix.root
                .join("runtime/omarchy-update.lock")
                .display()
                .to_string()
        ),
        "{}",
        error.message
    );
    drop(held);
    fs::remove_file(fix.root.join("runtime/omarchy-update.lock")).unwrap();

    fs::write(fix.root.join("pacman/db.lck"), b"").unwrap();
    let error = undo(&fix.roots(), &request("txn-1", "undo-1")).unwrap_err();
    assert!(error.message.contains("pacman"), "{}", error.message);
    assert!(
        error
            .message
            .contains(&fix.root.join("pacman/db.lck").display().to_string()),
        "{}",
        error.message
    );
    fs::remove_file(fix.root.join("pacman/db.lck")).unwrap();

    fs::write(fix.state("lock"), b"").unwrap();
    let error = undo(&fix.roots(), &request("txn-1", "undo-1")).unwrap_err();
    assert!(error.message.contains("drift lock"), "{}", error.message);
    assert!(fix.state("lock").exists());
    fs::remove_file(fix.state("lock")).unwrap();

    assert_eq!(tree(&fix), snapshot);
    restored(do_undo(&fix, "txn-1", "undo-1"));
}

#[test]
fn every_injected_failure_leaves_the_applied_tree_untouched() {
    let steps = [
        FileStep::Backup,
        FileStep::Temp,
        FileStep::Sync,
        FileStep::Rename,
    ];
    for index in 0..2 {
        for step in steps {
            let removal = index == 1;
            if removal && matches!(step, FileStep::Temp | FileStep::Sync) {
                continue;
            }
            let fix = Fixture::new();
            write_live(&fix.live(A), b"mine\n", 0o640);
            do_apply(
                &fix,
                "txn-1",
                1,
                &[edit(A, b"theirs\n", Some(0o600)), edit(B, b"new\n", None)],
            )
            .unwrap();
            let snapshot = tree(&fix);
            let error = undo(
                &fix.roots(),
                &UndoRequest {
                    fail_at: Some(FailAt::File { index, step }),
                    ..request("txn-1", "undo-1")
                },
            )
            .unwrap_err();
            assert!(error.message.contains("injected"), "{}", error.message);
            assert_eq!(tree(&fix), snapshot, "index {index} {step:?}");
            assert!(error.applied.is_empty(), "index {index} {step:?}");
            assert_eq!(error.unapplied.len(), 2, "index {index} {step:?}");
            assert!(!fix.state("lock").exists());
            assert!(
                leftovers(&fix.root).is_empty(),
                "{:?}",
                leftovers(&fix.root)
            );
            restored(do_undo(&fix, "txn-1", "undo-2"));
        }
    }
}

#[test]
fn a_manifest_failure_rolls_the_undo_back() {
    let fix = Fixture::new();
    write_live(&fix.live(A), b"mine\n", 0o640);
    do_apply(
        &fix,
        "txn-1",
        1,
        &[edit(A, b"theirs\n", Some(0o600)), edit(B, b"new\n", None)],
    )
    .unwrap();
    let snapshot = tree(&fix);
    let error = undo(
        &fix.roots(),
        &UndoRequest {
            fail_at: Some(FailAt::Manifest),
            ..request("txn-1", "undo-1")
        },
    )
    .unwrap_err();
    assert!(error.message.contains("manifest"), "{}", error.message);
    assert_eq!(tree(&fix), snapshot);
    assert!(error.applied.is_empty());
    assert!(!fix.state("backups/undo-1/manifest.json").exists());
}

#[test]
fn a_symlink_original_is_restored_as_a_symlink_and_redone_as_a_file() {
    let fix = Fixture::new();
    fs::write(fix.root.join("outside"), b"elsewhere\n").unwrap();
    symlink(fix.root.join("outside"), fix.live(A)).unwrap();
    let before = tree(&fix);
    do_apply_with(&fix, "txn-1", 1, &[edit(A, b"theirs\n", None)], true).unwrap();
    assert!(!fs::symlink_metadata(fix.live(A))
        .unwrap()
        .file_type()
        .is_symlink());
    let applied_state = tree(&fix);

    restored(do_undo(&fix, "txn-1", "undo-1"));
    assert_eq!(tree(&fix), before);
    assert!(fs::symlink_metadata(fix.live(A))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(fix.root.join("outside")).unwrap(), b"elsewhere\n");

    restored(do_undo(&fix, "undo-1", "undo-2"));
    assert_eq!(tree(&fix), applied_state);
    assert_eq!(fs::read(fix.root.join("outside")).unwrap(), b"elsewhere\n");
}

#[test]
fn a_symlinked_ancestor_is_never_followed() {
    let fix = Fixture::new();
    write_live(&fix.live(A), b"mine\n", 0o644);
    do_apply(&fix, "txn-1", 1, &[edit(A, b"theirs\n", None)]).unwrap();
    let elsewhere = fix.root.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::write(elsewhere.join("a.conf"), b"theirs\n").unwrap();
    fs::remove_dir_all(fix.live("config")).unwrap();
    symlink(&elsewhere, fix.live("config")).unwrap();
    let error = undo(&fix.roots(), &request("txn-1", "undo-1")).unwrap_err();
    assert!(error.message.contains("symlink"), "{}", error.message);
    assert_eq!(fs::read(elsewhere.join("a.conf")).unwrap(), b"theirs\n");
}

#[test]
fn history_lists_newest_first_and_reports_unreadable_entries() {
    let fix = Fixture::new();
    assert!(history(&fix.root.join("state")).unwrap().entries.is_empty());
    write_live(&fix.live(A), b"mine\n", 0o644);
    do_apply(&fix, "txn-1", 1, &[edit(A, b"one\n", None)]).unwrap();
    do_apply(
        &fix,
        "txn-2",
        2,
        &[edit(A, b"two\n", None), edit(B, b"b\n", None)],
    )
    .unwrap();
    restored(do_undo(&fix, "txn-2", "undo-1"));
    fs::create_dir_all(fix.state("backups/half")).unwrap();
    fs::write(fix.state("backups/half/pending.json"), b"{}").unwrap();
    fs::create_dir_all(fix.state("backups/odd")).unwrap();
    fs::write(fix.state("backups/odd/manifest.json"), b"not json").unwrap();
    fs::write(fix.state("backups/stray-file"), b"x").unwrap();

    let found = history(&fix.root.join("state")).unwrap();
    let ids: Vec<&str> = found.entries.iter().map(Plan::id).collect();
    assert_eq!(ids, ["undo-1", "txn-2", "txn-1"]);
    assert_eq!(found.unreadable, ["half", "odd"]);
    assert_eq!(found.entries[0].files()[0].action(), Action::Undo);
    assert_eq!(found.entries[1].files().len(), 2);
    assert_eq!(found.entries[2].package_version(), "4.5.0-3");
}

#[test]
fn history_breaks_time_ties_by_id() {
    let fix = Fixture::new();
    write_live(&fix.live(A), b"mine\n", 0o644);
    do_apply(&fix, "txn-a", 1, &[edit(A, b"one\n", None)]).unwrap();
    do_apply(&fix, "txn-b", 1, &[edit(A, b"two\n", None)]).unwrap();
    let ids: Vec<String> = history(&fix.root.join("state"))
        .unwrap()
        .entries
        .iter()
        .map(|plan| plan.id().to_string())
        .collect();
    assert_eq!(ids, ["txn-b", "txn-a"]);
}

#[test]
fn undo_prunes_the_oldest_when_it_makes_the_101st() {
    let fix = Fixture::new();
    for n in 1..100 {
        let id = format!("old-{n:03}");
        let dir = fix.state(&format!("backups/{id}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("marker"), &id).unwrap();
        let time = format!("2026-01-01T00:00:{n:05}");
        fs::write(dir.join("manifest.json"), format!(r#"{{"time":"{time}"}}"#)).unwrap();
    }
    write_live(&fix.live(A), b"mine\n", 0o644);
    do_apply(&fix, "txn-1", 1, &[edit(A, b"theirs\n", None)]).unwrap();
    let outcome = do_undo(&fix, "txn-1", "undo-1");
    let UndoOutcome::Restored(report) = outcome else {
        panic!("expected a restore");
    };
    assert_eq!(report.pruned, ["old-001"]);
    assert!(!fix.state("backups/old-001").exists());
    assert!(fix.state("backups/old-002/marker").is_file());
    assert!(fix.state("backups/undo-1/manifest.json").is_file());
    assert_eq!(fs::read_dir(fix.state("backups")).unwrap().count(), 100);
}

fn leftovers(root: &Path) -> Vec<PathBuf> {
    tree_of(root)
        .into_iter()
        .map(PathBuf::from)
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".drift-"))
        })
        .collect()
}

#[test]
fn a_mode_only_change_is_undone_and_a_later_chmod_is_a_divergence() {
    let fix = Fixture::new();
    write_live(&fix.live(A), b"same\n", 0o644);
    do_apply(&fix, "txn-1", 1, &[edit(A, b"same\n", Some(0o600))]).unwrap();
    assert_eq!(mode_of(&fix.live(A)), 0o600);
    let items = inspect_undo(&fix.roots(), "txn-1").unwrap();
    assert_eq!(items[0].status, UndoStatus::Restore);

    fs::set_permissions(fix.live(A), fs::Permissions::from_mode(0o666)).unwrap();
    let UndoOutcome::Diverged(items) = do_undo(&fix, "txn-1", "undo-1") else {
        panic!("expected a warning");
    };
    let UndoStatus::Diverged(found) = &items[0].status else {
        panic!("expected a divergence");
    };
    assert_eq!(found.expected, found.found);
    assert_eq!(found.expected_mode, Some(0o600));
    assert_eq!(found.found_mode, Some(0o666));
    assert_eq!(mode_of(&fix.live(A)), 0o666);

    fs::set_permissions(fix.live(A), fs::Permissions::from_mode(0o600)).unwrap();
    restored(do_undo(&fix, "txn-1", "undo-1"));
    assert_eq!(mode_of(&fix.live(A)), 0o644);
    assert_eq!(fs::read(fix.live(A)).unwrap(), b"same\n");
    restored(do_undo(&fix, "undo-1", "undo-2"));
    assert_eq!(mode_of(&fix.live(A)), 0o600);
}

#[test]
fn an_original_with_the_same_bytes_and_a_different_mode_is_not_already_original() {
    let fix = Fixture::new();
    write_live(&fix.live(A), b"same\n", 0o644);
    do_apply(&fix, "txn-1", 1, &[edit(A, b"same\n", Some(0o600))]).unwrap();
    fs::set_permissions(fix.live(A), fs::Permissions::from_mode(0o644)).unwrap();
    let items = inspect_undo(&fix.roots(), "txn-1").unwrap();
    assert_eq!(items[0].status, UndoStatus::AlreadyOriginal);
    fs::set_permissions(fix.live(A), fs::Permissions::from_mode(0o640)).unwrap();
    let items = inspect_undo(&fix.roots(), "txn-1").unwrap();
    assert!(matches!(items[0].status, UndoStatus::Diverged(_)));
}

#[test]
fn a_tampered_symlink_backup_is_refused_and_nothing_is_written() {
    let fix = Fixture::new();
    fs::write(fix.root.join("outside"), b"elsewhere\n").unwrap();
    symlink(fix.root.join("outside"), fix.live(A)).unwrap();
    do_apply_with(&fix, "txn-1", 1, &[edit(A, b"theirs\n", None)], true).unwrap();
    let nodes = fs::read_to_string(fix.state("backups/txn-1/nodes.json")).unwrap();
    assert!(
        nodes.contains(&sha(fix.root.join("outside").as_os_str().as_bytes())),
        "{nodes}"
    );
    let snapshot = tree(&fix);

    fs::write(fix.state("backups/txn-1/config/a.conf"), b"/etc/passwd").unwrap();
    let error = undo(&fix.roots(), &request("txn-1", "undo-1")).unwrap_err();
    assert!(
        error.message.contains("recorded digest"),
        "{}",
        error.message
    );
    assert_eq!(tree(&fix), snapshot);
    assert!(!fix.state("backups/undo-1").exists());

    fs::write(
        fix.state("backups/txn-1/nodes.json"),
        br#"{"config/a.conf":"symlink"}"#,
    )
    .unwrap();
    fs::write(
        fix.state("backups/txn-1/config/a.conf"),
        fix.root.join("outside").as_os_str().as_bytes(),
    )
    .unwrap();
    let error = undo(&fix.roots(), &request("txn-1", "undo-1")).unwrap_err();
    assert!(
        error.message.contains("recorded digest"),
        "{}",
        error.message
    );
    assert_eq!(tree(&fix), snapshot);
}

#[test]
fn a_missing_state_directory_is_created_on_the_first_write() {
    let fix = Fixture::new();
    fs::remove_dir_all(fix.state("")).unwrap();
    write_live(&fix.live(A), b"mine\n", 0o644);
    do_apply(&fix, "txn-1", 1, &[edit(A, b"theirs\n", None)]).unwrap();
    assert!(fix.state("backups/txn-1/manifest.json").is_file());
    restored(do_undo(&fix, "txn-1", "undo-1"));
    assert_eq!(fs::read(fix.live(A)).unwrap(), b"mine\n");
}
