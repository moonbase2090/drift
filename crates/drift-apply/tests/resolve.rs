use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use drift_apply::{
    execute, pending_prune, recorded_bases, undo, undo_with_bases, undo_with_bases_probed,
    HyprlandCheck, HyprlandStatus, Roots, UndoOutcome, UndoRequest, WriteFollowup,
};
use drift_core::{prepare, Action, Prepared, Registry, RelPath, ScanReport, Status};
use drift_fixtures::{hash_tree, MaterializedRoot, RootBuilder};
use drift_source::{
    build_report, load_resolve_inputs, ReadPaths, ResolvePaths, ScanPaths, SourcePaths,
};
use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct World {
    root: MaterializedRoot,
}

struct Builder {
    files: RootBuilder,
    listed: String,
    mtree: String,
}

impl Builder {
    fn new() -> Self {
        let mut files = RootBuilder::new();
        for keep in [
            "state/.keep",
            "runtime/.keep",
            "home/.local/share/.keep",
            "home/.local/state/.keep",
        ] {
            files.file(keep, b"").unwrap();
        }
        Self {
            files,
            listed: String::from("%FILES%\nusr/share/omarchy/config/\n"),
            mtree: String::from("#mtree\n/set type=file uid=0 gid=0 mode=644\n"),
        }
    }

    fn file(
        &mut self,
        rel: &str,
        theirs: &str,
        ours: Option<&str>,
        base: Option<&str>,
    ) -> &mut Self {
        self.listed
            .push_str(&format!("usr/share/omarchy/config/{rel}\n"));
        self.mtree.push_str(&format!(
            "./usr/share/omarchy/config/{rel} time=1.0 size=1 sha256digest={}\n",
            sha(theirs.as_bytes())
        ));
        self.files
            .file(format!("usr/share/omarchy/config/{rel}"), theirs)
            .unwrap();
        if let Some(ours) = ours {
            self.files
                .file(format!("home/.config/{rel}"), ours)
                .unwrap();
        }
        if let Some(base) = base {
            self.files
                .file(format!("home/.local/share/drift/bases/config/{rel}"), base)
                .unwrap();
        }
        self
    }

    fn build(mut self) -> World {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(self.mtree.as_bytes()).unwrap();
        let dir = "var/lib/pacman/local/omarchy-settings-4.5.0-3";
        self.files
            .file(
                format!("{dir}/desc"),
                "%NAME%\nomarchy-settings\n\n%VERSION%\n4.5.0-3\n",
            )
            .unwrap()
            .file(format!("{dir}/files"), self.listed.clone())
            .unwrap()
            .file(format!("{dir}/mtree"), encoder.finish().unwrap())
            .unwrap();
        World {
            root: self.files.materialize().unwrap(),
        }
    }
}

impl World {
    fn path(&self, rel: &str) -> PathBuf {
        self.root.path().join(rel)
    }

    fn roots(&self) -> Roots {
        Roots {
            live: self.path("home"),
            state: self.path("state"),
            omarchy_lock: self.path("runtime/omarchy-update.lock"),
            pacman_lock: self.path("runtime/db.lck"),
        }
    }

    fn share(&self) -> PathBuf {
        self.path("home/.local/share/drift")
    }

    fn paths(&self) -> ScanPaths {
        let root = self.root.path().to_path_buf();
        ScanPaths {
            source: SourcePaths::new(
                ReadPaths::new(&root, root.join("usr/share/omarchy")),
                root.join("home"),
                root.join("skel"),
            ),
            resolve: ResolvePaths {
                share: self.share(),
                state: root.join("home/.local/state/drift"),
                snapshots: root.join("var/lib/drift/snapshots"),
                cache: root.join("var/cache/pacman/pkg"),
                log: root.join("var/log/pacman.log"),
            },
        }
    }

    fn report(&self) -> ScanReport {
        build_report(&Registry::v1(), &self.paths(), &BTreeSet::new()).unwrap()
    }

    fn prepared(&self, action: Action, id: &str, targets: &[&str]) -> (Prepared, Vec<String>) {
        let rels: Vec<RelPath> = targets
            .iter()
            .map(|path| RelPath::new(path).unwrap())
            .collect();
        let loaded =
            load_resolve_inputs(&Registry::v1(), &self.paths(), &BTreeSet::new(), &rels).unwrap();
        let prune = pending_prune(&self.path("state")).unwrap();
        let refs: Vec<&str> = prune.iter().map(String::as_str).collect();
        let prepared = prepare(
            action,
            &loaded.inputs,
            id,
            "2026-06-01T00:00:00Z",
            "4.5.0-3",
            &refs,
        )
        .unwrap();
        (prepared, loaded.unknown)
    }
}

struct Quiet;

impl HyprlandCheck for Quiet {
    fn config_errors(&self) -> HyprlandStatus {
        HyprlandStatus::Clean
    }
}

struct Broken;

impl HyprlandCheck for Broken {
    fn config_errors(&self) -> HyprlandStatus {
        HyprlandStatus::Errors("line 3: bad".to_string())
    }
}

struct Interleaved<'a> {
    base: PathBuf,
    seen: std::cell::RefCell<Option<String>>,
    other: &'a dyn Fn(),
}

impl HyprlandCheck for Interleaved<'_> {
    fn config_errors(&self) -> HyprlandStatus {
        *self.seen.borrow_mut() = std::fs::read_to_string(&self.base).ok();
        (self.other)();
        HyprlandStatus::Clean
    }
}

fn status_of(report: &ScanReport, path: &str) -> (Status, String) {
    let entry = report
        .entries
        .iter()
        .find(|entry| entry.path == path)
        .unwrap_or_else(|| panic!("{path} not in the report"));
    (entry.status, entry.base.label().to_string())
}

fn read(world: &World, rel: &str) -> String {
    fs::read_to_string(world.path(rel)).unwrap()
}

fn transactions(world: &World) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(world.path("state/backups"))
        .map(|dir| {
            dir.flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn keep_mine_writes_no_config_and_survives_a_rescan_as_an_acknowledged_override() {
    let mut builder = Builder::new();
    builder.file(
        "hypr/a.lua",
        "upstream new\n",
        Some("mine\n"),
        Some("upstream old\n"),
    );
    let world = builder.build();
    assert_eq!(
        status_of(&world.report(), ".config/hypr/a.lua").0,
        Status::Conflict
    );
    let before = hash_tree(&world.path("home/.config")).unwrap();

    let (prepared, unknown) = world.prepared(Action::KeepMine, "txn-1", &[".config/hypr/a.lua"]);
    assert!(unknown.is_empty());
    let done = execute(&world.roots(), &world.share(), &prepared, &Quiet).unwrap();
    assert_eq!(done.checked.report.applied, [".config/hypr/a.lua"]);
    assert_eq!(done.recorded, ["config/hypr/a.lua"]);
    assert!(done.unrecorded.is_empty());

    assert_eq!(hash_tree(&world.path("home/.config")).unwrap(), before);
    assert_eq!(read(&world, "home/.config/hypr/a.lua"), "mine\n");
    assert_eq!(
        read(&world, "home/.local/share/drift/bases/config/hypr/a.lua"),
        "upstream new\n"
    );
    for _ in 0..2 {
        let report = world.report();
        assert_eq!(
            status_of(&report, ".config/hypr/a.lua"),
            (Status::LocalOverride, "reviewed".to_string())
        );
        assert_eq!(report.drifted(), 0);
    }
    assert_eq!(transactions(&world), ["txn-1"]);
}

#[test]
fn keep_mine_on_a_deleted_file_keeps_it_deleted_and_acknowledged() {
    let mut builder = Builder::new();
    builder.file("hypr/a.lua", "upstream new\n", None, Some("upstream old\n"));
    let world = builder.build();
    let (prepared, _) = world.prepared(Action::KeepMine, "txn-1", &[".config/hypr/a.lua"]);
    execute(&world.roots(), &world.share(), &prepared, &Quiet).unwrap();
    assert!(!world.path("home/.config/hypr/a.lua").exists());
    assert_eq!(
        read(&world, "home/.local/share/drift/bases/config/hypr/a.lua"),
        "upstream new\n"
    );
}

#[test]
fn take_upstream_replaces_the_file_and_backs_up_the_original() {
    let mut builder = Builder::new();
    builder.file(
        "hypr/a.lua",
        "upstream new\n",
        Some("mine\n"),
        Some("upstream old\n"),
    );
    builder.file("hypr/b.lua", "fresh\n", None, None);
    let world = builder.build();
    let (prepared, _) = world.prepared(
        Action::TakeUpstream,
        "txn-1",
        &[".config/hypr/a.lua", ".config/hypr/b.lua"],
    );
    execute(&world.roots(), &world.share(), &prepared, &Quiet).unwrap();
    assert_eq!(read(&world, "home/.config/hypr/a.lua"), "upstream new\n");
    assert_eq!(read(&world, "home/.config/hypr/b.lua"), "fresh\n");
    assert_eq!(
        read(&world, "state/backups/txn-1/.config/hypr/a.lua"),
        "mine\n"
    );
    assert!(!world
        .path("state/backups/txn-1/.config/hypr/b.lua")
        .exists());
    let report = world.report();
    assert_eq!(
        status_of(&report, ".config/hypr/a.lua").0,
        Status::Unchanged
    );
    assert_eq!(
        status_of(&report, ".config/hypr/b.lua").0,
        Status::Unchanged
    );
}

#[test]
fn merge_writes_the_clean_three_way_result() {
    let mut builder = Builder::new();
    builder.file(
        "hypr/a.lua",
        "one\ntwo\nTHREE\n",
        Some("ONE\ntwo\nthree\n"),
        Some("one\ntwo\nthree\n"),
    );
    let world = builder.build();
    assert_eq!(
        status_of(&world.report(), ".config/hypr/a.lua").0,
        Status::UpstreamChanged
    );
    let (prepared, _) = world.prepared(Action::Merge, "txn-1", &[".config/hypr/a.lua"]);
    execute(&world.roots(), &world.share(), &prepared, &Quiet).unwrap();
    assert_eq!(read(&world, "home/.config/hypr/a.lua"), "ONE\ntwo\nTHREE\n");
    assert_eq!(
        read(&world, "state/backups/txn-1/.config/hypr/a.lua"),
        "ONE\ntwo\nthree\n"
    );
    assert_eq!(
        status_of(&world.report(), ".config/hypr/a.lua"),
        (Status::LocalOverride, "reviewed".to_string())
    );
}

#[test]
fn a_bulk_action_is_one_transaction_and_one_undo_restores_every_file() {
    let mut builder = Builder::new();
    for name in ["a", "b", "c"] {
        builder.file(
            &format!("hypr/{name}.lua"),
            &format!("{name} new\n"),
            Some(&format!("{name} mine\n")),
            Some(&format!("{name} old\n")),
        );
    }
    let world = builder.build();
    let (prepared, _) = world.prepared(
        Action::TakeUpstream,
        "txn-1",
        &[
            ".config/hypr/a.lua",
            ".config/hypr/b.lua",
            ".config/hypr/c.lua",
        ],
    );
    let done = execute(&world.roots(), &world.share(), &prepared, &Quiet).unwrap();
    assert_eq!(done.checked.report.applied.len(), 3);
    assert_eq!(transactions(&world), ["txn-1"]);
    for name in ["a", "b", "c"] {
        assert_eq!(
            read(&world, &format!("home/.config/hypr/{name}.lua")),
            format!("{name} new\n")
        );
    }
    let outcome = undo(
        &world.roots(),
        &UndoRequest {
            target: "txn-1",
            id: "undo-1",
            time: "2026-06-02T00:00:00Z",
            package_version: "4.5.0-3",
            restore_anyway: &[],
            fail_at: None,
        },
    )
    .unwrap();
    assert!(matches!(outcome, UndoOutcome::Restored(_)));
    for name in ["a", "b", "c"] {
        assert_eq!(
            read(&world, &format!("home/.config/hypr/{name}.lua")),
            format!("{name} mine\n")
        );
    }
    assert_eq!(transactions(&world), ["txn-1", "undo-1"]);
}

#[test]
fn unknown_and_unsupported_files_are_reported_and_do_not_stop_the_rest() {
    let mut builder = Builder::new();
    builder.file("hypr/a.lua", "new\n", Some("mine\n"), Some("old\n"));
    builder.file("hypr/b.lua", "x\n", Some("x\n"), None);
    builder.file("hypr/c.lua", "new\n", Some("mine\n"), None);
    builder
        .files
        .file("home/.config/hypr/stray.lua", "s\n")
        .unwrap();
    let world = builder.build();
    let (prepared, unknown) = world.prepared(
        Action::Merge,
        "txn-1",
        &[
            ".config/hypr/a.lua",
            ".config/hypr/b.lua",
            ".config/hypr/c.lua",
            ".config/hypr/stray.lua",
            ".config/elsewhere/none.lua",
        ],
    );
    assert_eq!(unknown, [".config/elsewhere/none.lua"]);
    let reasons: Vec<(String, &str)> = prepared
        .skipped
        .iter()
        .map(|skip| (skip.path.clone(), skip.reason.label()))
        .collect();
    assert_eq!(
        reasons,
        [
            (".config/hypr/a.lua".to_string(), "has conflicting hunks"),
            (".config/hypr/b.lua".to_string(), "nothing to do"),
            (
                ".config/hypr/c.lua".to_string(),
                "has no base to merge from"
            ),
            (".config/hypr/stray.lua".to_string(), "has no template"),
        ]
    );
    assert!(prepared.plan.is_none());
    let error = execute(&world.roots(), &world.share(), &prepared, &Quiet).unwrap_err();
    assert!(
        error.message.contains("nothing to apply"),
        "{}",
        error.message
    );
    assert!(transactions(&world).is_empty());
}

#[test]
fn the_base_is_committed_with_the_file_before_the_hyprland_check_so_a_concurrent_acknowledgement_wins(
) {
    let mut builder = Builder::new();
    builder.file("hypr/a.lua", "new\n", Some("mine\n"), Some("old\n"));
    builder.file("hypr/b.lua", "new b\n", Some("mine b\n"), Some("old b\n"));
    let world = builder.build();
    let (prepared, _) = world.prepared(Action::TakeUpstream, "txn-1", &[".config/hypr/a.lua"]);
    let base = world.path("home/.local/share/drift/bases/config/hypr/a.lua");
    let other = || {
        let (second, _) = world.prepared(Action::KeepMine, "txn-2", &[".config/hypr/b.lua"]);
        execute(&world.roots(), &world.share(), &second, &Quiet)
            .expect("the apply lock must be free while the checker runs");
        std::fs::write(&base, "newer\n").unwrap();
    };
    let checker = Interleaved {
        base: base.clone(),
        seen: std::cell::RefCell::new(None),
        other: &other,
    };
    let done = execute(&world.roots(), &world.share(), &prepared, &checker).unwrap();
    assert!(done.unrecorded.is_empty());
    assert_eq!(checker.seen.borrow().as_deref(), Some("new\n"));
    assert_eq!(std::fs::read_to_string(&base).unwrap(), "newer\n");
}

#[test]
fn a_hyprland_config_error_after_a_write_offers_an_undo() {
    let mut builder = Builder::new();
    builder.file("hypr/a.lua", "new\n", Some("mine\n"), Some("old\n"));
    let world = builder.build();
    let (prepared, _) = world.prepared(Action::TakeUpstream, "txn-1", &[".config/hypr/a.lua"]);
    let done = execute(&world.roots(), &world.share(), &prepared, &Broken).unwrap();
    assert_eq!(
        done.checked.followup,
        WriteFollowup::OfferUndo {
            transaction: "txn-1".to_string(),
            errors: "line 3: bad".to_string()
        }
    );
}

#[test]
fn keep_mine_never_triggers_the_hyprland_check() {
    let mut builder = Builder::new();
    builder.file("hypr/a.lua", "new\n", Some("mine\n"), Some("old\n"));
    let world = builder.build();
    let (prepared, _) = world.prepared(Action::KeepMine, "txn-1", &[".config/hypr/a.lua"]);
    let done = execute(&world.roots(), &world.share(), &prepared, &Broken).unwrap();
    assert_eq!(done.checked.followup, WriteFollowup::Clear);
}

#[test]
fn a_file_edited_after_the_plan_aborts_the_write_and_records_no_base() {
    let mut builder = Builder::new();
    builder.file("hypr/a.lua", "new\n", Some("mine\n"), Some("old\n"));
    let world = builder.build();
    let (prepared, _) = world.prepared(Action::TakeUpstream, "txn-1", &[".config/hypr/a.lua"]);
    fs::write(world.path("home/.config/hypr/a.lua"), "edited meanwhile\n").unwrap();
    let error = execute(&world.roots(), &world.share(), &prepared, &Quiet).unwrap_err();
    assert!(error.message.contains("stale"), "{}", error.message);
    assert_eq!(
        read(&world, "home/.config/hypr/a.lua"),
        "edited meanwhile\n"
    );
    assert_eq!(
        read(&world, "home/.local/share/drift/bases/config/hypr/a.lua"),
        "old\n"
    );
}

fn undo_request<'a>(target: &'a str, id: &'a str, forced: &'a [String]) -> UndoRequest<'a> {
    UndoRequest {
        target,
        id,
        time: "2026-06-02T00:00:00Z",
        package_version: "4.5.0-3",
        restore_anyway: forced,
        fail_at: None,
    }
}

fn base(world: &World, rel: &str) -> Option<String> {
    fs::read_to_string(world.path(&format!("home/.local/share/drift/bases/config/{rel}"))).ok()
}

#[test]
fn undoing_a_take_upstream_makes_the_file_read_as_upstream_changed_again() {
    let mut builder = Builder::new();
    builder.file("hypr/a.lua", "upstream new\n", Some("old\n"), Some("old\n"));
    let world = builder.build();
    assert_eq!(
        status_of(&world.report(), ".config/hypr/a.lua"),
        (Status::UpstreamChanged, "reviewed".to_string())
    );
    let (prepared, _) = world.prepared(Action::TakeUpstream, "txn-1", &[".config/hypr/a.lua"]);
    execute(&world.roots(), &world.share(), &prepared, &Quiet).unwrap();
    assert_eq!(
        base(&world, "hypr/a.lua").as_deref(),
        Some("upstream new\n")
    );
    assert_eq!(
        status_of(&world.report(), ".config/hypr/a.lua").0,
        Status::Unchanged
    );

    let done = undo_with_bases(
        &world.roots(),
        &world.share(),
        &undo_request("txn-1", "undo-1", &[]),
    )
    .unwrap();
    assert!(matches!(done.outcome, UndoOutcome::Restored(_)));
    assert_eq!(done.bases.restored, ["config/hypr/a.lua"]);
    assert!(done.bases.failed.is_empty());
    assert_eq!(read(&world, "home/.config/hypr/a.lua"), "old\n");
    assert_eq!(base(&world, "hypr/a.lua").as_deref(), Some("old\n"));
    assert_eq!(
        status_of(&world.report(), ".config/hypr/a.lua"),
        (Status::UpstreamChanged, "reviewed".to_string())
    );
}

#[test]
fn undoing_a_keep_mine_removes_the_acknowledgement_in_its_own_transaction() {
    let mut builder = Builder::new();
    builder.file(
        "hypr/a.lua",
        "upstream new\n",
        Some("mine\n"),
        Some("upstream old\n"),
    );
    let world = builder.build();
    let (prepared, _) = world.prepared(Action::KeepMine, "txn-1", &[".config/hypr/a.lua"]);
    execute(&world.roots(), &world.share(), &prepared, &Quiet).unwrap();
    assert_eq!(
        status_of(&world.report(), ".config/hypr/a.lua").0,
        Status::LocalOverride
    );

    let done = undo_with_bases(
        &world.roots(),
        &world.share(),
        &undo_request("txn-1", "undo-1", &[]),
    )
    .unwrap();
    assert_eq!(done.bases.restored, ["config/hypr/a.lua"]);
    assert_eq!(
        base(&world, "hypr/a.lua").as_deref(),
        Some("upstream old\n")
    );
    assert_eq!(read(&world, "home/.config/hypr/a.lua"), "mine\n");
    assert_eq!(
        status_of(&world.report(), ".config/hypr/a.lua").0,
        Status::Conflict
    );
    assert_eq!(transactions(&world), ["txn-1", "undo-1"]);
    let history = drift_apply::history(&world.path("state")).unwrap();
    assert!(history.unreadable.is_empty(), "{:?}", history.unreadable);
    assert_eq!(history.entries.len(), 2);
}

#[test]
fn undoing_the_undo_puts_the_reviewed_base_back() {
    let mut builder = Builder::new();
    builder.file(
        "hypr/a.lua",
        "upstream new\n",
        Some("mine\n"),
        Some("upstream old\n"),
    );
    let world = builder.build();
    let (prepared, _) = world.prepared(Action::KeepMine, "txn-1", &[".config/hypr/a.lua"]);
    execute(&world.roots(), &world.share(), &prepared, &Quiet).unwrap();
    undo_with_bases(
        &world.roots(),
        &world.share(),
        &undo_request("txn-1", "undo-1", &[]),
    )
    .unwrap();
    assert_eq!(
        base(&world, "hypr/a.lua").as_deref(),
        Some("upstream old\n")
    );
    undo_with_bases(
        &world.roots(),
        &world.share(),
        &undo_request("undo-1", "undo-2", &[]),
    )
    .unwrap();
    assert_eq!(
        base(&world, "hypr/a.lua").as_deref(),
        Some("upstream new\n")
    );
    assert_eq!(
        status_of(&world.report(), ".config/hypr/a.lua"),
        (Status::LocalOverride, "reviewed".to_string())
    );
}

#[test]
fn a_file_that_had_no_reviewed_base_has_none_again_after_the_undo() {
    let mut builder = Builder::new();
    builder.file("hypr/a.lua", "upstream new\n", Some("mine\n"), None);
    let world = builder.build();
    let (prepared, _) = world.prepared(Action::KeepMine, "txn-1", &[".config/hypr/a.lua"]);
    execute(&world.roots(), &world.share(), &prepared, &Quiet).unwrap();
    assert!(base(&world, "hypr/a.lua").is_some());
    undo_with_bases(
        &world.roots(),
        &world.share(),
        &undo_request("txn-1", "undo-1", &[]),
    )
    .unwrap();
    assert!(base(&world, "hypr/a.lua").is_none());
    assert_eq!(
        status_of(&world.report(), ".config/hypr/a.lua"),
        (Status::LocalOverride, "none".to_string())
    );
}

#[test]
fn a_diverged_undo_leaves_the_reviewed_bases_alone() {
    let mut builder = Builder::new();
    builder.file("hypr/a.lua", "upstream new\n", Some("old\n"), Some("old\n"));
    let world = builder.build();
    let (prepared, _) = world.prepared(Action::TakeUpstream, "txn-1", &[".config/hypr/a.lua"]);
    execute(&world.roots(), &world.share(), &prepared, &Quiet).unwrap();
    fs::write(world.path("home/.config/hypr/a.lua"), "edited later\n").unwrap();
    let done = undo_with_bases(
        &world.roots(),
        &world.share(),
        &undo_request("txn-1", "undo-1", &[]),
    )
    .unwrap();
    assert!(matches!(done.outcome, UndoOutcome::Diverged(_)));
    assert!(done.bases.restored.is_empty());
    assert_eq!(
        base(&world, "hypr/a.lua").as_deref(),
        Some("upstream new\n")
    );
    assert!(!world.path("state/backups/undo-1").exists());
}

#[test]
fn undoing_a_transaction_without_a_base_record_behaves_like_the_plain_undo() {
    let mut builder = Builder::new();
    builder.file("hypr/a.lua", "upstream new\n", Some("old\n"), Some("old\n"));
    let world = builder.build();
    let (prepared, _) = world.prepared(Action::TakeUpstream, "txn-1", &[".config/hypr/a.lua"]);
    execute(&world.roots(), &world.share(), &prepared, &Quiet).unwrap();
    fs::remove_file(world.path("state/backups/txn-1/reviewed-bases.json")).unwrap();
    let done = undo_with_bases(
        &world.roots(),
        &world.share(),
        &undo_request("txn-1", "undo-1", &[]),
    )
    .unwrap();
    assert!(matches!(done.outcome, UndoOutcome::Restored(_)));
    assert!(done.bases.restored.is_empty());
    assert_eq!(read(&world, "home/.config/hypr/a.lua"), "old\n");
    assert_eq!(
        base(&world, "hypr/a.lua").as_deref(),
        Some("upstream new\n")
    );
    let error = undo_with_bases(
        &world.roots(),
        &world.share(),
        &undo_request("txn-1", "undo-2", &[]),
    )
    .unwrap_err();
    assert!(
        error.message.contains("nothing to undo"),
        "{}",
        error.message
    );
}

#[test]
fn the_previous_bases_are_recorded_with_the_transaction() {
    let mut builder = Builder::new();
    builder.file("hypr/a.lua", "upstream new\n", Some("old\n"), Some("old\n"));
    let world = builder.build();
    let (prepared, _) = world.prepared(Action::TakeUpstream, "txn-1", &[".config/hypr/a.lua"]);
    execute(&world.roots(), &world.share(), &prepared, &Quiet).unwrap();
    let record = read(&world, "state/backups/txn-1/reviewed-bases.json");
    assert!(record.contains("\"rel\":\"hypr/a.lua\""), "{record}");
    assert!(record.contains("\"root\":\"config\""), "{record}");
    let old_hex: String = "old\n".bytes().map(|byte| format!("{byte:02x}")).collect();
    assert!(record.contains(&old_hex), "{record}");
}

#[test]
fn the_base_restore_runs_under_the_same_lock_as_the_file_undo() {
    let mut builder = Builder::new();
    builder.file("hypr/a.lua", "upstream new\n", Some("old\n"), Some("old\n"));
    builder.file("hypr/b.lua", "new b\n", Some("mine b\n"), Some("old b\n"));
    let world = builder.build();
    let (prepared, _) = world.prepared(Action::TakeUpstream, "txn-1", &[".config/hypr/a.lua"]);
    execute(&world.roots(), &world.share(), &prepared, &Quiet).unwrap();
    let mut refused = None;
    let mut between = || {
        let (second, _) = world.prepared(Action::KeepMine, "txn-2", &[".config/hypr/b.lua"]);
        refused = Some(execute(&world.roots(), &world.share(), &second, &Quiet));
    };
    let done = undo_with_bases_probed(
        &world.roots(),
        &world.share(),
        &undo_request("txn-1", "undo-1", &[]),
        &mut between,
    )
    .unwrap();
    assert_eq!(done.bases.restored, ["config/hypr/a.lua"]);
    let error = refused.unwrap().unwrap_err();
    assert!(error.message.contains("lock"), "{}", error.message);
    assert_eq!(base(&world, "hypr/b.lua").as_deref(), Some("old b\n"));
    assert_eq!(base(&world, "hypr/a.lua").as_deref(), Some("old\n"));
}

#[test]
fn undoing_the_oldest_kept_transaction_still_restores_its_bases_when_the_undo_prunes_it() {
    let mut builder = Builder::new();
    builder.file("hypr/a.lua", "upstream new\n", Some("old\n"), Some("old\n"));
    for index in 0..99 {
        builder.file(
            &format!("hypr/f{index}.lua"),
            "new\n",
            Some("mine\n"),
            Some("old\n"),
        );
    }
    let world = builder.build();
    let (first, _) = world.prepared(Action::TakeUpstream, "txn-000", &[".config/hypr/a.lua"]);
    execute(&world.roots(), &world.share(), &first, &Quiet).unwrap();
    for index in 0..99 {
        let id = format!("txn-{:03}", index + 1);
        let target = format!(".config/hypr/f{index}.lua");
        let (next, _) = world.prepared(Action::KeepMine, &id, &[target.as_str()]);
        execute(&world.roots(), &world.share(), &next, &Quiet).unwrap();
    }
    assert_eq!(transactions(&world).len(), 100);
    assert_eq!(
        base(&world, "hypr/a.lua").as_deref(),
        Some("upstream new\n")
    );

    let done = undo_with_bases(
        &world.roots(),
        &world.share(),
        &undo_request("txn-000", "undo-1", &[]),
    )
    .unwrap();
    assert!(matches!(done.outcome, UndoOutcome::Restored(_)));
    assert!(
        !transactions(&world).contains(&"txn-000".to_string()),
        "the oldest transaction was pruned by the undo"
    );
    assert_eq!(done.bases.restored, ["config/hypr/a.lua"]);
    assert_eq!(read(&world, "home/.config/hypr/a.lua"), "old\n");
    assert_eq!(base(&world, "hypr/a.lua").as_deref(), Some("old\n"));

    let again = undo_with_bases(
        &world.roots(),
        &world.share(),
        &undo_request("undo-1", "undo-2", &[]),
    )
    .unwrap();
    assert_eq!(again.bases.restored, ["config/hypr/a.lua"]);
    assert_eq!(
        base(&world, "hypr/a.lua").as_deref(),
        Some("upstream new\n")
    );
}

#[test]
fn the_recorded_bases_of_a_transaction_are_listed_and_empty_without_a_record() {
    let mut builder = Builder::new();
    builder.file("hypr/a.lua", "new\n", Some("mine\n"), Some("old\n"));
    builder.file("hypr/b.lua", "new b\n", Some("mine b\n"), Some("old b\n"));
    let world = builder.build();
    let (prepared, _) = world.prepared(
        Action::KeepMine,
        "txn-1",
        &[".config/hypr/a.lua", ".config/hypr/b.lua"],
    );
    execute(&world.roots(), &world.share(), &prepared, &Quiet).unwrap();
    let mut listed = recorded_bases(&world.path("state"), "txn-1").unwrap();
    listed.sort();
    assert_eq!(listed, ["config/hypr/a.lua", "config/hypr/b.lua"]);
    assert!(recorded_bases(&world.path("state"), "missing")
        .unwrap()
        .is_empty());
}
