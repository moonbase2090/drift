use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use drift_core::{BaseRecord, Registry, Template, Verification};
use drift_source::{
    base_path, enumerate, read_package, read_reviewed_version, reviewed_version_path, SourcePaths,
    Tracked,
};

pub struct StoreRoots {
    pub share: PathBuf,
    pub state: PathBuf,
}

pub struct Scan {
    pub files: Vec<Tracked>,
    pub package_version: String,
    pub reviewed_version: Option<String>,
    pub seeded: Vec<String>,
}

pub fn scan(
    registry: &Registry,
    paths: &SourcePaths,
    roots: &StoreRoots,
) -> Result<Scan, drift_source::Error> {
    let files = enumerate(registry, paths)?;
    let package_version = read_package(&paths.package)?.version;
    let seeded = if read_reviewed_version(&roots.state).is_none() {
        seed(paths, roots, &files, &package_version)
    } else {
        Vec::new()
    };
    Ok(Scan {
        files,
        package_version,
        reviewed_version: read_reviewed_version(&roots.state),
        seeded,
    })
}

fn seed(paths: &SourcePaths, roots: &StoreRoots, files: &[Tracked], version: &str) -> Vec<String> {
    if !can_record(&roots.state) || !recordable_version(version) {
        return Vec::new();
    }
    if ensure_anchor(&roots.share).is_err() {
        return Vec::new();
    }
    let mut seeded = Vec::new();
    let mut complete = true;
    for file in files {
        let Some(template) = file.template.as_ref() else {
            continue;
        };
        if template.verification() != Verification::Verified {
            continue;
        }
        let Some(live) = file.live.as_ref() else {
            continue;
        };
        if live.link_target().is_some()
            || matches!(live.treatment(), drift_core::LiveTreatment::Unreadable)
        {
            continue;
        }
        let Some(stock) = stock_file(paths, template) else {
            complete = false;
            continue;
        };
        let live_path = template.live_path().under(&paths.home);
        if !same_contents(&live_path, &stock) {
            continue;
        }
        let Ok(bytes) = fs::read(&stock) else {
            complete = false;
            continue;
        };
        let root = template.mapping().root.as_str();
        let rel = &template.mapping().rel;
        let Some(dest) = base_path(&roots.share, root, rel) else {
            complete = false;
            continue;
        };
        if write_bytes(&roots.share, &dest, &bytes).is_err() {
            complete = false;
            continue;
        }
        seeded.push(format!("{root}/{}", rel.as_str()));
    }
    seeded.sort();
    if complete && write_version(&roots.state, version).is_ok() {
        seeded
    } else {
        Vec::new()
    }
}

pub struct Recorded {
    pub recorded: Vec<String>,
    pub failed: Vec<String>,
}

pub fn record_bases(share: &Path, records: &[BaseRecord]) -> Recorded {
    let mut recorded = Vec::new();
    let mut failed = Vec::new();
    let anchored = ensure_anchor(share).is_ok();
    for record in records {
        let label = format!("{}/{}", record.root, record.rel.as_str());
        let written = anchored
            && base_path(share, &record.root, &record.rel)
                .is_some_and(|dest| write_bytes(share, &dest, &record.bytes).is_ok());
        if written {
            recorded.push(label);
        } else {
            failed.push(label);
        }
    }
    Recorded { recorded, failed }
}

fn recordable_version(version: &str) -> bool {
    !version.is_empty()
        && version
            .chars()
            .all(|c| !c.is_whitespace() && c != '/' && c != '\0')
}

fn stock_file(paths: &SourcePaths, template: &Template) -> Option<PathBuf> {
    let packaged = paths.package.omarchy_path.join(template.path().as_str());
    if is_regular_file(&packaged) {
        return Some(packaged);
    }
    let rest = template.path().as_str().strip_prefix("config/")?;
    if rest.is_empty() {
        return None;
    }
    let skel = paths.skel.join(".config").join(rest);
    is_regular_file(&skel).then_some(skel)
}

fn same_contents(left: &Path, right: &Path) -> bool {
    let (Ok(mut left_file), Ok(mut right_file)) = (regular_file(left), regular_file(right)) else {
        return false;
    };
    let mut left_buf = [0u8; 8192];
    let mut right_buf = [0u8; 8192];
    loop {
        let Ok(left_n) = read_chunk(&mut left_file, &mut left_buf) else {
            return false;
        };
        let Ok(right_n) = read_chunk(&mut right_file, &mut right_buf) else {
            return false;
        };
        if left_n != right_n || left_buf[..left_n] != right_buf[..right_n] {
            return false;
        }
        if left_n == 0 {
            return true;
        }
    }
}

fn read_chunk(file: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(source) if source.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(source) => return Err(source),
        }
    }
    Ok(filled)
}

fn regular_file(path: &Path) -> std::io::Result<File> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    File::open(path)
}

fn can_record(state: &Path) -> bool {
    if ensure_anchor(state).is_err() {
        return false;
    }
    let probe = state.join(".drift-seed-probe");
    if is_symlink(&probe).unwrap_or(true) {
        return false;
    }
    let _ = fs::remove_file(&probe);
    match OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(file) => {
            drop(file);
            fs::remove_file(&probe).is_ok()
        }
        Err(_) => false,
    }
}

fn write_version(state: &Path, version: &str) -> Result<(), ()> {
    let dest = reviewed_version_path(state);
    write_bytes(state, &dest, format!("{version}\n").as_bytes())
}

fn write_bytes(anchor: &Path, dest: &Path, bytes: &[u8]) -> Result<(), ()> {
    if !dest.starts_with(anchor) {
        return Err(());
    }
    let parent = dest.parent().ok_or(())?;
    ensure_real_dir(anchor, parent)?;
    if is_symlink(dest).unwrap_or(true) {
        return Err(());
    }
    let temp_path = parent.join(".drift-seed.tmp");
    if is_symlink(&temp_path).unwrap_or(true) {
        return Err(());
    }
    let _ = fs::remove_file(&temp_path);
    let mut temp = TempFile::create(&temp_path)?;
    temp.write_all(bytes)?;
    set_mode(temp.path(), 0o644)?;
    temp.sync()?;
    fs::rename(temp.path(), dest).map_err(|_| ())?;
    temp.forget();
    if let Ok(dir) = File::open(parent) {
        let _ = dir.sync_all();
    }
    Ok(())
}

fn ensure_anchor(dir: &Path) -> Result<(), ()> {
    if path_exists(dir) {
        return require_real_dir(dir);
    }
    let parent = dir.parent().ok_or(())?;
    create_missing_parents(parent)?;
    require_real_dir(parent)?;
    fs::create_dir(dir).map_err(|_| ())
}

fn create_missing_parents(dir: &Path) -> Result<(), ()> {
    match fs::symlink_metadata(dir) {
        Ok(_) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            let parent = dir.parent().ok_or(())?;
            create_missing_parents(parent)?;
            require_real_dir(parent)?;
            std::os::unix::fs::DirBuilderExt::mode(&mut fs::DirBuilder::new(), 0o700)
                .create(dir)
                .map_err(|_| ())
        }
        Err(_) => Err(()),
    }
}

fn ensure_real_dir(anchor: &Path, dir: &Path) -> Result<(), ()> {
    if dir == anchor {
        return require_real_dir(dir);
    }
    if !dir.starts_with(anchor) {
        return Err(());
    }
    let parent = dir.parent().ok_or(())?;
    ensure_real_dir(anchor, parent)?;
    match fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_symlink() => Err(()),
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(dir).map_err(|_| ())
        }
        Err(_) => Err(()),
    }
}

fn require_real_dir(dir: &Path) -> Result<(), ()> {
    let meta = fs::symlink_metadata(dir).map_err(|_| ())?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(());
    }
    Ok(())
}

fn is_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .ok()
        .is_some_and(|meta| meta.is_file())
}

fn is_symlink(path: &Path) -> Result<bool, ()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(meta.file_type().is_symlink()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(()),
    }
}

fn path_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn set_mode(path: &Path, mode: u32) -> Result<(), ()> {
    let permissions = fs::Permissions::from_mode(mode);
    fs::set_permissions(path, permissions).map_err(|_| ())
}

struct TempFile {
    path: PathBuf,
    file: Option<File>,
}

impl TempFile {
    fn create(path: &Path) -> Result<Self, ()> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|_| ())?;
        Ok(Self {
            path: path.to_path_buf(),
            file: Some(file),
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), ()> {
        self.file
            .as_mut()
            .ok_or(())?
            .write_all(bytes)
            .map_err(|_| ())
    }

    fn sync(&self) -> Result<(), ()> {
        self.file.as_ref().ok_or(())?.sync_all().map_err(|_| ())
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
