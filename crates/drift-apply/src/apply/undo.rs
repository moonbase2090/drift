use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    pub expected: Option<String>,
    pub found: Option<String>,
    pub expected_mode: Option<u32>,
    pub found_mode: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoStatus {
    Restore,
    Remove,
    AlreadyOriginal,
    Kept,
    Diverged(Divergence),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoItem {
    pub path: String,
    pub status: UndoStatus,
}

pub struct UndoRequest<'a> {
    pub target: &'a str,
    pub id: &'a str,
    pub time: &'a str,
    pub package_version: &'a str,
    pub restore_anyway: &'a [String],
    pub fail_at: Option<FailAt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoOutcome {
    Restored(ApplyReport),
    Diverged(Vec<UndoItem>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct History {
    pub entries: Vec<Plan>,
    pub unreadable: Vec<String>,
}

struct Inspected {
    path: String,
    status: UndoStatus,
    current: Option<String>,
    original: Option<String>,
    mode: Option<u32>,
    restore: bool,
    symlink: bool,
    backup: PathBuf,
}

pub fn history(state: &Path) -> Result<History, ApplyError> {
    let mut entries = Vec::new();
    let mut unreadable = Vec::new();
    for found in list_transactions(state)? {
        match load_manifest(state, &found.id) {
            Ok(plan) => entries.push(plan),
            Err(_) => unreadable.push(found.id),
        }
    }
    entries.sort_by(|left, right| (right.time(), right.id()).cmp(&(left.time(), left.id())));
    unreadable.sort();
    Ok(History {
        entries,
        unreadable,
    })
}

pub fn inspect_undo(roots: &Roots, target: &str) -> Result<Vec<UndoItem>, ApplyError> {
    let plan = load_manifest(&roots.state, target)?;
    Ok(inspect_plan(roots, &plan)?
        .into_iter()
        .map(|item| UndoItem {
            path: item.path,
            status: item.status,
        })
        .collect())
}

pub fn undo(roots: &Roots, request: &UndoRequest<'_>) -> Result<UndoOutcome, ApplyError> {
    undo_locked(roots, request, &mut |outcome| outcome)
}

pub(super) fn undo_locked(
    roots: &Roots,
    request: &UndoRequest<'_>,
    under_lock: &mut dyn FnMut(Result<UndoOutcome, ApplyError>) -> Result<UndoOutcome, ApplyError>,
) -> Result<UndoOutcome, ApplyError> {
    if node_exists(&roots.omarchy_lock) {
        return Err(bare("omarchy update lock is held"));
    }
    if node_exists(&roots.pacman_lock) {
        return Err(bare("pacman database lock is held"));
    }
    let _lock = DriftLock::take(&roots.state)?;
    let outcome = undo_held(roots, request);
    under_lock(outcome)
}

fn undo_held(roots: &Roots, request: &UndoRequest<'_>) -> Result<UndoOutcome, ApplyError> {
    ensure_real_dir(&roots.live, &roots.live)?;
    ensure_real_dir(&roots.state, &roots.state)?;
    let target = load_manifest(&roots.state, request.target)?;
    for path in request.restore_anyway {
        if !target
            .files()
            .iter()
            .any(|file| file.path().as_str() == path)
        {
            return Err(bare(format!("{path} is not part of the transaction")));
        }
    }
    let inspected = inspect_plan(roots, &target)?;
    let blocked: Vec<UndoItem> = inspected
        .iter()
        .filter(|item| {
            matches!(item.status, UndoStatus::Diverged(_))
                && !request.restore_anyway.contains(&item.path)
        })
        .map(|item| UndoItem {
            path: item.path.clone(),
            status: item.status.clone(),
        })
        .collect();
    if !blocked.is_empty() {
        return Ok(UndoOutcome::Diverged(blocked));
    }
    let jobs: Vec<&Inspected> = inspected
        .iter()
        .filter(|item| {
            matches!(
                item.status,
                UndoStatus::Diverged(_) | UndoStatus::Restore | UndoStatus::Remove
            )
        })
        .collect();
    if jobs.is_empty() {
        return Err(bare("nothing to undo"));
    }
    let mut files = Vec::with_capacity(jobs.len());
    for job in &jobs {
        files.push(
            PlannedFile::new(
                &job.path,
                Action::Undo,
                job.current.as_deref(),
                job.original.as_deref(),
                job.mode,
                None,
            )
            .map_err(|error| bare(error.to_string()))?,
        );
    }
    let prune = transactions_to_prune(&roots.state)?;
    let prune_refs: Vec<&str> = prune.iter().map(String::as_str).collect();
    let plan = Plan::new(
        request.id,
        request.time,
        request.package_version,
        files,
        &prune_refs,
    )
    .map_err(|error| bare(error.to_string()))?;
    for planned in plan.files() {
        if collides_with_metadata(plan.id(), &backup_relative(plan.id(), planned)) {
            return Err(refused(
                &plan,
                "backup path collides with transaction metadata",
            ));
        }
    }
    reserve_transaction(&roots.state, plan.id()).map_err(|error| refused(&plan, error.message))?;
    write_pending(&roots.state, &plan).map_err(|error| refused(&plan, error.message))?;

    let mut applied = Vec::new();
    for (index, (planned, job)) in plan.files().iter().zip(&jobs).enumerate() {
        match undo_file(roots, request, index, planned, job) {
            Ok(()) => applied.push(planned.path().as_str().to_string()),
            Err(error) => return Err(abort_written(roots, &plan, &applied, index, error)),
        }
    }
    finish(roots, &plan, applied, request.fail_at).map(UndoOutcome::Restored)
}

fn undo_file(
    roots: &Roots,
    request: &UndoRequest<'_>,
    index: usize,
    planned: &PlannedFile,
    job: &Inspected,
) -> Result<(), FileFailure> {
    let dest = roots.live.join(planned.path().as_str());
    confirm_hash(&dest, planned, true).map_err(|message| FileFailure {
        reason: message.clone(),
        message,
    })?;
    if fails(request.fail_at, index, FileStep::Backup) {
        return Err(injected(FileStep::Backup));
    }
    if node_exists(&dest) {
        backup_original(roots, request.id, planned, &dest)?;
    }
    if !job.restore {
        if fails(request.fail_at, index, FileStep::Rename) {
            return Err(injected(FileStep::Rename));
        }
        fs::remove_file(&dest).map_err(|source| file_failure(bare_io(&dest, source)))?;
        sync_parent(&dest);
        return Ok(());
    }
    let backup = &job.backup;
    if job.symlink {
        for step in [FileStep::Temp, FileStep::Sync, FileStep::Rename] {
            if fails(request.fail_at, index, step) {
                return Err(injected(step));
            }
        }
        return restore_symlink(&roots.live, &dest, backup).map_err(file_failure);
    }
    let bytes = read_original(backup)?;
    install(&Install {
        live: &roots.live,
        dest: &dest,
        id: request.id,
        index,
        fail_at: request.fail_at,
        bytes: &bytes,
        mode: job.mode.unwrap_or(0o644),
        allow_symlink: true,
    })
}

fn sync_parent(dest: &Path) {
    if let Some(parent) = dest.parent() {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
}

fn load_manifest(state: &Path, id: &str) -> Result<Plan, ApplyError> {
    if id.is_empty() || id.contains('/') || id.contains('\0') || id == "." || id == ".." {
        return Err(bare(format!("transaction id {id:?}")));
    }
    let dir = state.join("backups").join(id);
    match fs::symlink_metadata(&dir) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(bare("refusing to follow a symlink"))
        }
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Err(bare(format!("{id} is not a transaction"))),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(bare(format!("no such transaction {id}")))
        }
        Err(source) => return Err(bare_io(&dir, source)),
    }
    let manifest = dir.join("manifest.json");
    let text = match fs::read_to_string(&manifest) {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(bare(format!("transaction {id} has no manifest")))
        }
        Err(source) => return Err(bare_io(&manifest, source)),
    };
    let plan = Plan::from_json(&text).map_err(|error| bare(error.to_string()))?;
    if plan.id() != id {
        return Err(bare(format!("manifest does not belong to {id}")));
    }
    Ok(plan)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Node {
    hash: String,
    mode: Option<u32>,
    link: bool,
}

fn observe(dest: &Path) -> Result<Option<Node>, ApplyError> {
    match fs::symlink_metadata(dest) {
        Ok(meta) if meta.file_type().is_symlink() => {
            let target = fs::read_link(dest).map_err(|source| bare_io(dest, source))?;
            Ok(Some(Node {
                hash: sha256_hex(target.as_os_str().as_bytes()),
                mode: None,
                link: true,
            }))
        }
        Ok(meta) if meta.is_file() => Ok(Some(Node {
            hash: hash_file(dest)?,
            mode: Some(meta.permissions().mode() & 0o777),
            link: false,
        })),
        Ok(_) => Err(bare(format!("not a regular file {}", dest.display()))),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(bare_io(dest, source)),
    }
}

fn inspect_plan(roots: &Roots, target: &Plan) -> Result<Vec<Inspected>, ApplyError> {
    let mut items = Vec::with_capacity(target.files().len());
    for planned in target.files() {
        let path = planned.path().as_str().to_string();
        if planned.action() == Action::KeepMine {
            items.push(Inspected {
                path,
                status: UndoStatus::Kept,
                current: None,
                original: None,
                mode: None,
                restore: false,
                symlink: false,
                backup: PathBuf::new(),
            });
            continue;
        }
        let dest = roots.live.join(&path);
        if !dest.starts_with(&roots.live) {
            return Err(bare("path is outside the tree"));
        }
        refuse_linked_ancestors(&roots.live, &dest)?;
        let current_node = observe(&dest)?;
        let backup =
            backup_path(&roots.state, target.id(), planned).map_err(|error| bare(error.message))?;
        let link_digest = recorded_symlink(&roots.state, target.id(), &path)?;
        let symlink = link_digest.is_some();
        let restore = node_exists(&backup);
        if !restore && planned.sha256_before().is_some() {
            return Err(bare(format!("backup is missing for {path}")));
        }
        let original_node = if restore {
            let bytes = read_original(&backup).map_err(|error| bare(error.message))?;
            let digest = sha256_hex(&bytes);
            let recorded = match &link_digest {
                Some(recorded) => Some(recorded.as_str()),
                None => planned.sha256_before(),
            };
            if recorded != Some(digest.as_str()) {
                return Err(bare(format!(
                    "backup does not match its recorded digest for {path}"
                )));
            }
            Some(Node {
                hash: digest,
                mode: if symlink {
                    None
                } else {
                    existing_mode(&backup)
                },
                link: symlink,
            })
        } else {
            None
        };
        let current = current_node.as_ref().map(|node| node.hash.clone());
        let original = original_node.as_ref().map(|node| node.hash.clone());
        let mode = original_node.as_ref().and_then(|node| node.mode);
        let expected_mode = planned.mode();
        let at_expected = match &current_node {
            None => planned.sha256_after().is_none(),
            Some(node) => {
                Some(node.hash.as_str()) == planned.sha256_after()
                    && (node.link || expected_mode.is_none() || node.mode == expected_mode)
            }
        };
        let status = if current_node == original_node {
            UndoStatus::AlreadyOriginal
        } else if at_expected {
            if restore {
                UndoStatus::Restore
            } else {
                UndoStatus::Remove
            }
        } else {
            UndoStatus::Diverged(Divergence {
                expected: planned.sha256_after().map(str::to_string),
                found: current.clone(),
                expected_mode,
                found_mode: current_node.as_ref().and_then(|node| node.mode),
            })
        };
        items.push(Inspected {
            path,
            status,
            current,
            original,
            mode,
            restore,
            symlink,
            backup,
        });
    }
    Ok(items)
}
