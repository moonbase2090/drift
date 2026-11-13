use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

use drift_core::{Registry, RelPath};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotReport {
    pub version: String,
    pub files: u64,
}

#[derive(Debug)]
pub enum SnapshotError {
    Version(String),
    Escape(PathBuf),
    Io { path: PathBuf, source: io::Error },
}

impl SnapshotError {
    fn io(path: impl AsRef<Path>, source: io::Error) -> Self {
        Self::Io {
            path: path.as_ref().to_path_buf(),
            source,
        }
    }
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Version(version) => {
                write!(f, "package version {version:?} cannot name a snapshot")
            }
            Self::Escape(path) => write!(
                f,
                "refusing to write outside the snapshot store: {}",
                path.display()
            ),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for SnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub fn snapshot_templates(
    registry: &Registry,
    omarchy_path: &Path,
    snapshots: &Path,
    version: &str,
) -> Result<SnapshotReport, SnapshotError> {
    if !version_ok(version) {
        return Err(SnapshotError::Version(version.to_string()));
    }
    if !omarchy_path.is_absolute() {
        return Err(SnapshotError::Escape(omarchy_path.to_path_buf()));
    }
    if !snapshots.is_absolute() || snapshots.parent().is_none() || snapshots == Path::new("/") {
        return Err(SnapshotError::Escape(snapshots.to_path_buf()));
    }
    ensure_store(snapshots)?;
    let staging = snapshots.join(format!(".partial-{version}"));
    let dest = snapshots.join(version);
    if staging.parent() != Some(snapshots) || dest.parent() != Some(snapshots) {
        return Err(SnapshotError::Escape(snapshots.to_path_buf()));
    }
    remove_tree(snapshots, &staging)?;
    let files = match write_staging(registry, omarchy_path, snapshots, &staging) {
        Ok(files) => files,
        Err(error) => {
            let _ = remove_tree(snapshots, &staging);
            return Err(error);
        }
    };
    if let Err(error) = publish(snapshots, &staging, version) {
        let _ = remove_tree(snapshots, &staging);
        return Err(error);
    }
    Ok(SnapshotReport {
        version: version.to_string(),
        files,
    })
}

fn version_ok(version: &str) -> bool {
    let mut components = Path::new(version).components();
    let Some(Component::Normal(_)) = components.next() else {
        return false;
    };
    components.next().is_none() && version.chars().all(|c| !c.is_whitespace() && c != '\0')
}

fn ensure_store(dir: &Path) -> Result<(), SnapshotError> {
    let mut missing = Vec::new();
    let mut cursor = dir.to_path_buf();
    loop {
        match fs::symlink_metadata(&cursor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(SnapshotError::Escape(cursor));
            }
            Ok(meta) if meta.is_dir() => break,
            Ok(_) => {
                return Err(SnapshotError::io(
                    &cursor,
                    io::Error::other("snapshot store is not a directory"),
                ));
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                let Some(parent) = cursor.parent().map(Path::to_path_buf) else {
                    return Err(SnapshotError::Escape(cursor));
                };
                missing.push(cursor);
                cursor = parent;
            }
            Err(source) => return Err(SnapshotError::io(&cursor, source)),
        }
    }
    for path in missing.iter().rev() {
        fs::create_dir(path).map_err(|source| SnapshotError::io(path, source))?;
    }
    Ok(())
}

fn write_staging(
    registry: &Registry,
    omarchy_path: &Path,
    store: &Path,
    staging: &Path,
) -> Result<u64, SnapshotError> {
    fs::create_dir(staging).map_err(|source| SnapshotError::io(staging, source))?;
    let mut files = 0;
    for root in registry.roots() {
        files += copy_root(store, staging, omarchy_path, root.template())?;
    }
    Ok(files)
}

fn copy_root(
    store: &Path,
    staging: &Path,
    omarchy_path: &Path,
    template: &RelPath,
) -> Result<u64, SnapshotError> {
    let source = template.under(omarchy_path);
    if !source.starts_with(omarchy_path) {
        return Err(SnapshotError::Escape(source));
    }
    match fs::symlink_metadata(&source) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(SnapshotError::io(&source, error)),
        Ok(meta) if meta.file_type().is_symlink() => Err(SnapshotError::Escape(source)),
        Ok(meta) if meta.is_dir() => copy_dir(store, staging, &source, template),
        Ok(_) => Err(SnapshotError::io(
            &source,
            io::Error::other("template root is not a directory"),
        )),
    }
}

fn copy_dir(store: &Path, staging: &Path, dir: &Path, rel: &RelPath) -> Result<u64, SnapshotError> {
    let mut names = Vec::new();
    let entries = fs::read_dir(dir).map_err(|source| SnapshotError::io(dir, source))?;
    for entry in entries {
        let entry = entry.map_err(|source| SnapshotError::io(dir, source))?;
        names.push(entry.file_name());
    }
    names.sort();
    let mut files = 0;
    for name in names {
        let Some(text) = name.to_str() else {
            return Err(SnapshotError::io(
                dir,
                io::Error::other("file name is not UTF-8"),
            ));
        };
        if !segment_ok(text) {
            return Err(SnapshotError::Escape(dir.join(&name)));
        }
        let child_rel = RelPath::new(&format!("{}/{text}", rel.as_str()))
            .map_err(|_| SnapshotError::Escape(dir.join(&name)))?;
        let child = dir.join(&name);
        let meta =
            fs::symlink_metadata(&child).map_err(|source| SnapshotError::io(&child, source))?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            files += copy_dir(store, staging, &child, &child_rel)?;
        } else if meta.is_file() {
            copy_file(store, staging, &child, &child_rel)?;
            files += 1;
        }
    }
    Ok(files)
}

fn copy_file(
    store: &Path,
    staging: &Path,
    from: &Path,
    rel: &RelPath,
) -> Result<(), SnapshotError> {
    let dest = rel.under(staging);
    if !dest.starts_with(staging) || !staging.starts_with(store) {
        return Err(SnapshotError::Escape(dest));
    }
    let parent = dest
        .parent()
        .ok_or_else(|| SnapshotError::Escape(dest.clone()))?;
    ensure_real_dir(staging, parent)?;
    if link_at(&dest)? {
        return Err(SnapshotError::Escape(dest));
    }
    let temp = temp_path(parent, &dest);
    if !temp.starts_with(staging) || link_at(&temp)? {
        return Err(SnapshotError::Escape(temp));
    }
    if fs::symlink_metadata(&temp).is_ok() {
        fs::remove_file(&temp).map_err(|source| SnapshotError::io(&temp, source))?;
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|source| SnapshotError::io(&temp, source))?;
    let mut input = File::open(from).map_err(|source| SnapshotError::io(from, source))?;
    io::copy(&mut input, &mut output).map_err(|source| SnapshotError::io(from, source))?;
    output
        .sync_all()
        .map_err(|source| SnapshotError::io(&temp, source))?;
    drop(output);
    drop(input);
    fs::set_permissions(&temp, fs::Permissions::from_mode(0o644))
        .map_err(|source| SnapshotError::io(&temp, source))?;
    fs::rename(&temp, &dest).map_err(|source| SnapshotError::io(&dest, source))?;
    if let Ok(dir) = File::open(parent) {
        let _ = dir.sync_all();
    }
    Ok(())
}

fn temp_path(parent: &Path, dest: &Path) -> PathBuf {
    let primary = parent.join(".drift-snapshot.tmp");
    if primary == dest {
        parent.join(".drift-snapshot.alt.tmp")
    } else {
        primary
    }
}

fn publish(store: &Path, staging: &Path, version: &str) -> Result<(), SnapshotError> {
    let dest = store.join(version);
    let backup = store.join(format!(".previous-{version}"));
    if dest.parent() != Some(store) || backup.parent() != Some(store) {
        return Err(SnapshotError::Escape(dest));
    }
    match fs::symlink_metadata(&dest) {
        Ok(meta) if meta.file_type().is_symlink() => return Err(SnapshotError::Escape(dest)),
        Ok(meta) if !meta.is_dir() => {
            return Err(SnapshotError::io(
                &dest,
                io::Error::other("snapshot version is not a directory"),
            ));
        }
        Ok(_) => {}
        Err(source) if source.kind() == io::ErrorKind::NotFound => {}
        Err(source) => return Err(SnapshotError::io(&dest, source)),
    }
    remove_tree(store, &backup)?;
    let moved = if fs::symlink_metadata(&dest).is_ok() {
        fs::rename(&dest, &backup).map_err(|source| SnapshotError::io(&dest, source))?;
        true
    } else {
        false
    };
    if let Err(source) = fs::rename(staging, &dest) {
        if moved {
            let _ = fs::rename(&backup, &dest);
        }
        return Err(SnapshotError::io(&dest, source));
    }
    remove_tree(store, &backup)?;
    if let Ok(dir) = File::open(store) {
        let _ = dir.sync_all();
    }
    Ok(())
}

fn ensure_real_dir(anchor: &Path, dir: &Path) -> Result<(), SnapshotError> {
    if dir == anchor {
        return require_real_dir(dir);
    }
    if !dir.starts_with(anchor) {
        return Err(SnapshotError::Escape(dir.to_path_buf()));
    }
    let parent = dir
        .parent()
        .ok_or_else(|| SnapshotError::Escape(dir.to_path_buf()))?;
    ensure_real_dir(anchor, parent)?;
    match fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_symlink() => Err(SnapshotError::Escape(dir.to_path_buf())),
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(SnapshotError::io(dir, io::Error::other("not a directory"))),
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(dir).map_err(|source| SnapshotError::io(dir, source))
        }
        Err(source) => Err(SnapshotError::io(dir, source)),
    }
}

fn require_real_dir(dir: &Path) -> Result<(), SnapshotError> {
    let meta = fs::symlink_metadata(dir).map_err(|source| SnapshotError::io(dir, source))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(SnapshotError::Escape(dir.to_path_buf()));
    }
    Ok(())
}

fn remove_tree(store: &Path, path: &Path) -> Result<(), SnapshotError> {
    if path == store || !path.starts_with(store) {
        return Err(SnapshotError::Escape(path.to_path_buf()));
    }
    match fs::symlink_metadata(path) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(SnapshotError::io(path, source)),
        Ok(meta) if meta.file_type().is_symlink() => Err(SnapshotError::Escape(path.to_path_buf())),
        Ok(meta) if meta.is_dir() => {
            fs::remove_dir_all(path).map_err(|source| SnapshotError::io(path, source))
        }
        Ok(_) => fs::remove_file(path).map_err(|source| SnapshotError::io(path, source)),
    }
}

fn link_at(path: &Path) -> Result<bool, SnapshotError> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(meta.file_type().is_symlink()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(SnapshotError::io(path, source)),
    }
}

fn segment_ok(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.contains('\0')
}
