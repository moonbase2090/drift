use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Component, Path, PathBuf};

use tempfile::TempDir;

use crate::error::Error;
use crate::{LIVE_CONFIG, PACKAGE_TEMPLATES, REVIEWED_BASES};

pub const DEFAULT_FILE_MODE: u32 = 0o644;
pub const DEFAULT_DIR_MODE: u32 = 0o755;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Root {
    Config,
    Templates,
    Bases,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    File { bytes: Vec<u8>, mode: u32 },
    Symlink { target: String },
    Directory { mode: u32 },
    Absent,
}

impl Node {
    pub fn file(bytes: impl AsRef<[u8]>) -> Self {
        Self::File {
            bytes: bytes.as_ref().to_vec(),
            mode: DEFAULT_FILE_MODE,
        }
    }

    pub fn file_mode(bytes: impl AsRef<[u8]>, mode: u32) -> Self {
        Self::File {
            bytes: bytes.as_ref().to_vec(),
            mode,
        }
    }

    pub fn symlink(target: impl Into<String>) -> Self {
        Self::Symlink {
            target: target.into(),
        }
    }

    pub fn directory(mode: u32) -> Self {
        Self::Directory { mode }
    }

    pub fn absent() -> Self {
        Self::Absent
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub root: Root,
    pub path: PathBuf,
    pub node: Node,
}

#[derive(Debug, Default)]
pub struct FixtureBuilder {
    entries: Vec<Entry>,
}

impl FixtureBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn put(
        &mut self,
        root: Root,
        path: impl Into<PathBuf>,
        node: Node,
    ) -> Result<&mut Self, Error> {
        let path = path.into();
        check_rel(&path)?;
        if self
            .entries
            .iter()
            .any(|entry| entry.root == root && entry.path == path)
        {
            return Err(Error::message(&path, "duplicate fixture path"));
        }
        self.entries.push(Entry { root, path, node });
        Ok(self)
    }

    pub fn materialize(&self) -> Result<FixtureTrees, Error> {
        let home = TempDir::new().map_err(|source| Error::io("home", source))?;
        let omarchy = TempDir::new().map_err(|source| Error::io("omarchy", source))?;
        create_dir(home.path().join(LIVE_CONFIG))?;
        create_dir(home.path().join(REVIEWED_BASES))?;
        create_dir(omarchy.path().join(PACKAGE_TEMPLATES))?;

        let mut entries = self.entries.clone();
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        for entry in entries
            .iter()
            .filter(|entry| matches!(entry.node, Node::Directory { .. }))
        {
            write_entry(home.path(), omarchy.path(), entry)?;
        }
        for entry in entries
            .iter()
            .filter(|entry| matches!(entry.node, Node::File { .. } | Node::Symlink { .. }))
        {
            write_entry(home.path(), omarchy.path(), entry)?;
        }
        for entry in entries
            .iter()
            .filter(|entry| matches!(entry.node, Node::Absent))
        {
            let dest = locate(home.path(), omarchy.path(), entry);
            if dest.symlink_metadata().is_ok() {
                return Err(Error::message(&dest, "absent path exists"));
            }
        }
        Ok(FixtureTrees { home, omarchy })
    }
}

pub struct FixtureTrees {
    home: TempDir,
    omarchy: TempDir,
}

impl FixtureTrees {
    pub fn home(&self) -> &Path {
        self.home.path()
    }

    pub fn omarchy_path(&self) -> &Path {
        self.omarchy.path()
    }

    pub fn config_dir(&self) -> PathBuf {
        self.home.path().join(LIVE_CONFIG)
    }

    pub fn base_dir(&self) -> PathBuf {
        self.home.path().join(REVIEWED_BASES)
    }

    pub fn template_dir(&self) -> PathBuf {
        self.omarchy.path().join(PACKAGE_TEMPLATES)
    }

    pub fn path(&self, root: Root, rel: impl AsRef<Path>) -> PathBuf {
        match root {
            Root::Config => self.config_dir().join(rel),
            Root::Templates => self.template_dir().join(rel),
            Root::Bases => self.base_dir().join(rel),
        }
    }
}

impl std::fmt::Debug for FixtureTrees {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FixtureTrees")
            .field("home", &self.home.path())
            .field("omarchy_path", &self.omarchy.path())
            .finish()
    }
}

pub(crate) fn check_rel(path: &Path) -> Result<(), Error> {
    if path.as_os_str().is_empty() {
        return Err(Error::message(path, "path is empty"));
    }
    if path.is_absolute() {
        return Err(Error::message(path, "path is absolute"));
    }
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            _ => return Err(Error::message(path, "path must stay inside the tree")),
        }
    }
    Ok(())
}

fn locate(home: &Path, omarchy: &Path, entry: &Entry) -> PathBuf {
    match entry.root {
        Root::Config => home.join(LIVE_CONFIG).join(&entry.path),
        Root::Templates => omarchy.join(PACKAGE_TEMPLATES).join(&entry.path),
        Root::Bases => home.join(REVIEWED_BASES).join(&entry.path),
    }
}

fn write_entry(home: &Path, omarchy: &Path, entry: &Entry) -> Result<(), Error> {
    let dest = locate(home, omarchy, entry);
    match &entry.node {
        Node::Directory { mode } => {
            create_dir(&dest)?;
            set_mode(&dest, *mode)?;
        }
        Node::File { bytes, mode } => {
            if let Some(parent) = dest.parent() {
                create_dir(parent)?;
            }
            fs::write(&dest, bytes).map_err(|source| Error::io(&dest, source))?;
            set_mode(&dest, *mode)?;
        }
        Node::Symlink { target } => {
            if let Some(parent) = dest.parent() {
                create_dir(parent)?;
            }
            if dest.symlink_metadata().is_ok() {
                fs::remove_file(&dest).map_err(|source| Error::io(&dest, source))?;
            }
            symlink(target, &dest).map_err(|source| Error::io(&dest, source))?;
        }
        Node::Absent => {}
    }
    Ok(())
}

fn create_dir(path: impl AsRef<Path>) -> Result<(), Error> {
    let path = path.as_ref();
    fs::create_dir_all(path).map_err(|source| Error::io(path, source))
}

fn set_mode(path: &Path, mode: u32) -> Result<(), Error> {
    let permissions = fs::Permissions::from_mode(mode & 0o7777);
    fs::set_permissions(path, permissions).map_err(|source| Error::io(path, source))
}
