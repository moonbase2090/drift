use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use drift_core::{Action, Plan, PlannedFile};

const KEPT_TRANSACTIONS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStep {
    Backup,
    Temp,
    Sync,
    Rename,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailAt {
    File { index: usize, step: FileStep },
    Manifest,
    Prune,
}

pub struct Roots {
    pub live: PathBuf,
    pub state: PathBuf,
    pub omarchy_lock: PathBuf,
    pub pacman_lock: PathBuf,
}

pub struct Payload<'a> {
    pub path: &'a str,
    pub bytes: &'a [u8],
    pub volatile: bool,
    pub allow_volatile: bool,
    pub allow_symlink: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unapplied {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyReport {
    pub applied: Vec<String>,
    pub pruned: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyError {
    pub message: String,
    pub applied: Vec<String>,
    pub unapplied: Vec<Unapplied>,
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApplyError {}

pub struct Request<'a> {
    pub plan: &'a Plan,
    pub files: &'a [Payload<'a>],
    pub fail_at: Option<FailAt>,
}

pub fn apply(roots: &Roots, request: &Request<'_>) -> Result<ApplyReport, ApplyError> {
    if node_exists(&roots.omarchy_lock) {
        return Err(refused(request, "omarchy update lock is held"));
    }
    if node_exists(&roots.pacman_lock) {
        return Err(refused(request, "pacman database lock is held"));
    }
    let _lock = match DriftLock::take(&roots.state) {
        Ok(lock) => lock,
        Err(error) => return Err(refused(request, error.message)),
    };
    preflight(roots, request)?;
    let expected = transactions_to_prune(&roots.state)?;
    if !same_ids(request.plan.prune(), &expected) {
        return Err(refused(
            request,
            "prune list does not match the transactions that would be removed",
        ));
    }
    reserve_transaction(&roots.state, request.plan.id())
        .map_err(|error| refused(request, error.message))?;

    let mut applied = Vec::new();
    for (index, planned) in request.plan.files().iter().enumerate() {
        let payload = payload_for(request, planned.path().as_str())?;
        match write_file(roots, request, index, planned, payload) {
            Ok(()) => applied.push(planned.path().as_str().to_string()),
            Err(error) => {
                return Err(stopped(
                    error.message,
                    applied,
                    request.plan.files(),
                    index,
                    error.reason,
                ));
            }
        }
    }

    if request.fail_at == Some(FailAt::Manifest) {
        return Err(manifest_failure(
            roots,
            request,
            &applied,
            "injected failure during manifest",
        ));
    }
    if let Err(error) = write_manifest(&roots.state, request.plan) {
        return Err(manifest_failure(
            roots,
            request,
            &applied,
            error.message.as_str(),
        ));
    }
    if request.fail_at == Some(FailAt::Prune) {
        return Err(ApplyError {
            message: "injected failure during prune".to_string(),
            applied,
            unapplied: Vec::new(),
        });
    }
    let pruned = remove_transactions(&roots.state, request.plan.prune())?;
    Ok(ApplyReport { applied, pruned })
}

struct DriftLock {
    path: PathBuf,
}

impl DriftLock {
    fn take(state: &Path) -> Result<Self, ApplyError> {
        ensure_real_dir(state, state).map_err(|error| bare(error.message))?;
        let path = state.join("lock");
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => {
                let _ = file.sync_all();
                Ok(Self { path })
            }
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(bare("drift lock is held"))
            }
            Err(source) => Err(bare(format!("io error at {}: {source}", path.display()))),
        }
    }
}

impl Drop for DriftLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

struct FileFailure {
    message: String,
    reason: String,
}

fn write_file(
    roots: &Roots,
    request: &Request<'_>,
    index: usize,
    planned: &PlannedFile,
    payload: &Payload<'_>,
) -> Result<(), FileFailure> {
    let dest = roots.live.join(planned.path().as_str());
    confirm_hash(&dest, planned, payload.allow_symlink).map_err(|message| FileFailure {
        reason: message.clone(),
        message,
    })?;
    if planned.action() == Action::KeepMine {
        return Ok(());
    }
    if fails(request, index, FileStep::Backup) {
        return Err(injected(FileStep::Backup));
    }
    if node_exists(&dest) {
        backup_original(roots, request.plan.id(), planned, &dest)?;
    }
    if fails(request, index, FileStep::Temp) {
        return Err(injected(FileStep::Temp));
    }
    let parent = dest.parent().ok_or_else(|| FileFailure {
        message: format!("path is outside the tree {}", dest.display()),
        reason: "path is outside the tree".to_string(),
    })?;
    ensure_real_dir(&roots.live, parent).map_err(file_failure)?;
    if is_symlink(&dest).map_err(file_failure)? && !payload.allow_symlink {
        return Err(FileFailure {
            message: format!("refusing to follow a symlink {}", dest.display()),
            reason: "refusing to follow a symlink".to_string(),
        });
    }
    let mode = planned
        .mode()
        .unwrap_or(existing_mode(&dest).unwrap_or(0o644));
    let temp_path = parent.join(format!(".drift-{}-{index}.tmp", request.plan.id()));
    let mut temp = TempFile::create(&temp_path).map_err(file_failure)?;
    temp.write_all(payload.bytes).map_err(file_failure)?;
    set_mode(temp.path(), mode).map_err(file_failure)?;
    if fails(request, index, FileStep::Sync) {
        return Err(injected(FileStep::Sync));
    }
    temp.sync().map_err(file_failure)?;
    if fails(request, index, FileStep::Rename) {
        return Err(injected(FileStep::Rename));
    }
    fs::rename(temp.path(), &dest).map_err(|source| file_failure(bare_io(&dest, source)))?;
    temp.forget();
    if let Some(parent) = dest.parent() {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

fn backup_original(
    roots: &Roots,
    txn: &str,
    planned: &PlannedFile,
    source: &Path,
) -> Result<(), FileFailure> {
    let dest = backup_path(&roots.state, txn, planned)?;
    if node_exists(&dest) {
        return Err(FileFailure {
            message: "backup already exists".to_string(),
            reason: "backup already exists".to_string(),
        });
    }
    let bytes = read_original(source)?;
    let mode = existing_mode(source).unwrap_or(0o644);
    write_bytes(&roots.state, &dest, &bytes, mode).map_err(file_failure)
}

fn backup_path(state: &Path, txn: &str, planned: &PlannedFile) -> Result<PathBuf, FileFailure> {
    let relative = match planned.backup() {
        Some(path) => path.as_str().to_string(),
        None => format!("backups/{txn}/{}", planned.path().as_str()),
    };
    let dest = state.join(&relative);
    if !dest.starts_with(state) {
        return Err(FileFailure {
            message: format!("backup path is outside the state directory {relative}"),
            reason: "backup path is outside the state directory".to_string(),
        });
    }
    Ok(dest)
}

fn reserve_transaction(state: &Path, id: &str) -> Result<(), ApplyError> {
    let root = state.join("backups");
    ensure_real_dir(state, &root)?;
    let dir = root.join(id);
    if dir.parent() != Some(root.as_path()) {
        return Err(bare("transaction id is outside the backup directory"));
    }
    match fs::symlink_metadata(&dir) {
        Ok(_) => Err(bare("transaction already exists")),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            match fs::create_dir(&dir) {
                Ok(()) => Ok(()),
                Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                    Err(bare("transaction already exists"))
                }
                Err(source) => Err(bare_io(&dir, source)),
            }
        }
        Err(source) => Err(bare_io(&dir, source)),
    }
}

fn manifest_failure(
    roots: &Roots,
    request: &Request<'_>,
    applied: &[String],
    message: &str,
) -> ApplyError {
    let still = restore_live(roots, request.plan, applied);
    let mut unapplied = Vec::new();
    for file in request.plan.files() {
        let path = file.path().as_str();
        if still.iter().any(|kept| kept == path) {
            continue;
        }
        unapplied.push(Unapplied {
            path: path.to_string(),
            reason: message.to_string(),
        });
    }
    ApplyError {
        message: message.to_string(),
        applied: still,
        unapplied,
    }
}

fn restore_live(roots: &Roots, plan: &Plan, applied: &[String]) -> Vec<String> {
    let mut still = Vec::new();
    for path in applied.iter().rev() {
        let Some(planned) = plan
            .files()
            .iter()
            .find(|file| file.path().as_str() == path)
        else {
            still.push(path.clone());
            continue;
        };
        if planned.action() == Action::KeepMine {
            continue;
        }
        let dest = roots.live.join(path.as_str());
        if planned.sha256_before().is_none() {
            match fs::remove_file(&dest) {
                Ok(()) => {}
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => still.push(path.clone()),
            }
            continue;
        }
        let backup = match backup_path(&roots.state, plan.id(), planned) {
            Ok(path) => path,
            Err(_) => {
                still.push(path.clone());
                continue;
            }
        };
        match restore_backup(&roots.live, &dest, &backup) {
            Ok(()) => {}
            Err(_) => still.push(path.clone()),
        }
    }
    still.reverse();
    still
}

fn restore_backup(anchor: &Path, dest: &Path, backup: &Path) -> Result<(), ApplyError> {
    let bytes = read_original(backup).map_err(|error| bare(error.message))?;
    let mode = existing_mode(backup).unwrap_or(0o644);
    write_bytes(anchor, dest, &bytes, mode)
}

fn read_original(source: &Path) -> Result<Vec<u8>, FileFailure> {
    if is_symlink(source).map_err(file_failure)? {
        let target = fs::read_link(source).map_err(|error| file_failure(bare_io(source, error)))?;
        return Ok(target.as_os_str().as_bytes().to_vec());
    }
    fs::read(source).map_err(|error| file_failure(bare_io(source, error)))
}

fn write_bytes(anchor: &Path, dest: &Path, bytes: &[u8], mode: u32) -> Result<(), ApplyError> {
    let parent = dest
        .parent()
        .ok_or_else(|| bare("path is outside the tree"))?;
    ensure_real_dir(anchor, parent)?;
    if is_symlink(dest)? {
        return Err(bare(format!(
            "refusing to follow a symlink {}",
            dest.display()
        )));
    }
    let temp_path = parent.join(".drift-backup.tmp");
    let mut temp = TempFile::create(&temp_path)?;
    temp.write_all(bytes)?;
    set_mode(temp.path(), mode)?;
    temp.sync()?;
    fs::rename(temp.path(), dest).map_err(|source| bare_io(dest, source))?;
    temp.forget();
    Ok(())
}

fn confirm_hash(dest: &Path, planned: &PlannedFile, allow_symlink: bool) -> Result<(), String> {
    let current = match fs::symlink_metadata(dest) {
        Ok(meta) if meta.file_type().is_symlink() => {
            if allow_symlink {
                return Ok(());
            }
            return Err("refusing to follow a symlink".to_string());
        }
        Ok(_) => Some(hash_file(dest).map_err(|error| error.to_string())?),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => return Err(source.to_string()),
    };
    if current.as_deref() != planned.sha256_before() {
        return Err(format!("stale hash for {}", planned.path().as_str()));
    }
    Ok(())
}

fn preflight(roots: &Roots, request: &Request<'_>) -> Result<(), ApplyError> {
    ensure_real_dir(&roots.live, &roots.live)?;
    ensure_real_dir(&roots.state, &roots.state)?;
    for planned in request.plan.files() {
        let path = planned.path().as_str();
        let payload = payload_for(request, path)?;
        if payload.volatile && !payload.allow_volatile {
            return Err(refused(request, "refusing to write a volatile file"));
        }
        if let Some(expected) = planned.sha256_after() {
            if sha256_hex(payload.bytes) != expected {
                return Err(refused(
                    request,
                    format!("SHA-256 digest does not match {path}"),
                ));
            }
        }
        let dest = roots.live.join(path);
        if !dest.starts_with(&roots.live) {
            return Err(refused(request, "path is outside the tree"));
        }
        refuse_linked_ancestors(&roots.live, &dest)?;
        if is_symlink(&dest)? && !payload.allow_symlink {
            return Err(refused(request, "refusing to follow a symlink"));
        }
    }
    Ok(())
}

fn refuse_linked_ancestors(anchor: &Path, path: &Path) -> Result<(), ApplyError> {
    if path == anchor {
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| bare("path is outside the tree"))?;
    refuse_linked_ancestors(anchor, parent)?;
    if parent != anchor && is_symlink(parent)? {
        return Err(bare("refusing to follow a symlink"));
    }
    Ok(())
}

fn transactions_to_prune(state: &Path) -> Result<Vec<String>, ApplyError> {
    let mut found = list_transactions(state)?;
    found.sort_by(|left, right| (&left.time, &left.id).cmp(&(&right.time, &right.id)));
    if found.len() < KEPT_TRANSACTIONS {
        return Ok(Vec::new());
    }
    let overflow = found.len() + 1 - KEPT_TRANSACTIONS;
    Ok(found
        .into_iter()
        .take(overflow)
        .map(|item| item.id)
        .collect())
}

struct Transaction {
    id: String,
    time: String,
}

fn list_transactions(state: &Path) -> Result<Vec<Transaction>, ApplyError> {
    let root = state.join("backups");
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(bare_io(&root, source)),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| bare_io(&root, source))?;
        let path = entry.path();
        if is_symlink(&path)? || !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        let time = manifest_time(&path.join("manifest.json"));
        found.push(Transaction { id, time });
    }
    Ok(found)
}

fn manifest_time(path: &Path) -> String {
    let Ok(text) = fs::read_to_string(path) else {
        return String::new();
    };
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|value| {
            value
                .get("time")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default()
}

fn remove_transactions(state: &Path, ids: &[String]) -> Result<Vec<String>, ApplyError> {
    let root = state.join("backups");
    let mut removed = Vec::new();
    for id in ids {
        let dir = root.join(id);
        if !dir.starts_with(&root) || dir.parent() != Some(root.as_path()) {
            return Err(bare("refusing to prune outside the backup directory"));
        }
        if !node_exists(&dir) {
            continue;
        }
        if is_symlink(&dir)? {
            return Err(bare("refusing to follow a symlink"));
        }
        fs::remove_dir_all(&dir).map_err(|source| bare_io(&dir, source))?;
        removed.push(id.clone());
    }
    Ok(removed)
}

fn write_manifest(state: &Path, plan: &Plan) -> Result<(), ApplyError> {
    let dest = state.join("backups").join(plan.id()).join("manifest.json");
    let text = plan.to_json().map_err(|error| bare(error.to_string()))?;
    write_bytes(state, &dest, text.as_bytes(), 0o644)
}

fn payload_for<'a>(request: &'a Request<'_>, path: &str) -> Result<&'a Payload<'a>, ApplyError> {
    request
        .files
        .iter()
        .find(|file| file.path == path)
        .ok_or_else(|| bare(format!("missing bytes for {path}")))
}

fn fails(request: &Request<'_>, index: usize, step: FileStep) -> bool {
    request.fail_at == Some(FailAt::File { index, step })
}

fn injected(step: FileStep) -> FileFailure {
    let name = match step {
        FileStep::Backup => "backup",
        FileStep::Temp => "temp",
        FileStep::Sync => "sync",
        FileStep::Rename => "rename",
    };
    FileFailure {
        message: format!("injected failure during {name}"),
        reason: format!("injected failure during {name}"),
    }
}

fn stopped(
    message: String,
    applied: Vec<String>,
    files: &[PlannedFile],
    index: usize,
    reason: String,
) -> ApplyError {
    let mut unapplied = vec![Unapplied {
        path: files[index].path().as_str().to_string(),
        reason,
    }];
    for file in files.iter().skip(index + 1) {
        unapplied.push(Unapplied {
            path: file.path().as_str().to_string(),
            reason: "not applied".to_string(),
        });
    }
    ApplyError {
        message,
        applied,
        unapplied,
    }
}

fn refused(request: &Request<'_>, message: impl Into<String>) -> ApplyError {
    let message = message.into();
    ApplyError {
        applied: Vec::new(),
        unapplied: request
            .plan
            .files()
            .iter()
            .map(|file| Unapplied {
                path: file.path().as_str().to_string(),
                reason: message.clone(),
            })
            .collect(),
        message,
    }
}

fn bare(message: impl Into<String>) -> ApplyError {
    ApplyError {
        message: message.into(),
        applied: Vec::new(),
        unapplied: Vec::new(),
    }
}

fn bare_io(path: &Path, source: std::io::Error) -> ApplyError {
    bare(format!("io error at {}: {source}", path.display()))
}

fn file_failure(error: ApplyError) -> FileFailure {
    FileFailure {
        reason: error.message.clone(),
        message: error.message,
    }
}

fn same_ids(listed: &[String], expected: &[String]) -> bool {
    let mut listed = listed.to_vec();
    let mut expected = expected.to_vec();
    listed.sort();
    expected.sort();
    listed == expected
}

fn node_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn is_symlink(path: &Path) -> Result<bool, ApplyError> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(meta.file_type().is_symlink()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(bare_io(path, source)),
    }
}

fn existing_mode(path: &Path) -> Option<u32> {
    fs::symlink_metadata(path)
        .ok()
        .map(|meta| meta.permissions().mode() & 0o777)
}

fn set_mode(path: &Path, mode: u32) -> Result<(), ApplyError> {
    let permissions = fs::Permissions::from_mode(mode & 0o777);
    fs::set_permissions(path, permissions).map_err(|source| bare_io(path, source))
}

fn ensure_real_dir(anchor: &Path, dir: &Path) -> Result<(), ApplyError> {
    if dir == anchor {
        return require_real_dir(dir);
    }
    if !dir.starts_with(anchor) {
        return Err(bare("path is outside the tree"));
    }
    let parent = dir
        .parent()
        .ok_or_else(|| bare("path is outside the tree"))?;
    ensure_real_dir(anchor, parent)?;
    match fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_symlink() => Err(bare("refusing to follow a symlink")),
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(bare("not a directory")),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(dir).map_err(|source| bare_io(dir, source))
        }
        Err(source) => Err(bare_io(dir, source)),
    }
}

fn require_real_dir(dir: &Path) -> Result<(), ApplyError> {
    let meta = fs::symlink_metadata(dir).map_err(|source| bare_io(dir, source))?;
    if meta.file_type().is_symlink() {
        return Err(bare("refusing to follow a symlink"));
    }
    if !meta.is_dir() {
        return Err(bare("not a directory"));
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<String, ApplyError> {
    let bytes = fs::read(path).map_err(|source| bare_io(path, source))?;
    Ok(sha256_hex(&bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

struct TempFile {
    path: PathBuf,
    file: Option<File>,
}

impl TempFile {
    fn create(path: &Path) -> Result<Self, ApplyError> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|source| bare_io(path, source))?;
        Ok(Self {
            path: path.to_path_buf(),
            file: Some(file),
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), ApplyError> {
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| bare("temp file is closed"))?;
        file.write_all(bytes)
            .map_err(|source| bare_io(&self.path, source))
    }

    fn sync(&self) -> Result<(), ApplyError> {
        let file = self
            .file
            .as_ref()
            .ok_or_else(|| bare("temp file is closed"))?;
        file.sync_all()
            .map_err(|source| bare_io(&self.path, source))
    }

    fn forget(mut self) {
        self.file.take();
        std::mem::forget(self);
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        self.file.take();
        let _ = fs::remove_file(&self.path);
    }
}
