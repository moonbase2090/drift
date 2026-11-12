use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::error::Error;
use crate::tree::check_rel;

#[derive(Debug)]
enum Node {
    Bytes { bytes: Vec<u8>, mode: Option<u32> },
    Link { target: String },
}

#[derive(Debug, Default)]
pub struct RootBuilder {
    files: Vec<(PathBuf, Node)>,
}

impl RootBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn file(
        &mut self,
        path: impl Into<PathBuf>,
        bytes: impl AsRef<[u8]>,
    ) -> Result<&mut Self, Error> {
        self.insert(
            path,
            Node::Bytes {
                bytes: bytes.as_ref().to_vec(),
                mode: None,
            },
        )
    }

    pub fn file_with_mode(
        &mut self,
        path: impl Into<PathBuf>,
        bytes: impl AsRef<[u8]>,
        mode: u32,
    ) -> Result<&mut Self, Error> {
        self.insert(
            path,
            Node::Bytes {
                bytes: bytes.as_ref().to_vec(),
                mode: Some(mode),
            },
        )
    }

    pub fn link(
        &mut self,
        path: impl Into<PathBuf>,
        target: impl Into<String>,
    ) -> Result<&mut Self, Error> {
        self.insert(
            path,
            Node::Link {
                target: target.into(),
            },
        )
    }

    fn insert(&mut self, path: impl Into<PathBuf>, node: Node) -> Result<&mut Self, Error> {
        let path = path.into();
        check_rel(&path)?;
        if self.files.iter().any(|(existing, _)| existing == &path) {
            return Err(Error::message(&path, "duplicate fixture path"));
        }
        self.files.push((path, node));
        Ok(self)
    }

    pub fn materialize(&self) -> Result<MaterializedRoot, Error> {
        let dir = TempDir::new().map_err(|source| Error::io("root", source))?;
        for (path, node) in &self.files {
            let dest = dir.path().join(path);
            if !dest.starts_with(dir.path()) {
                return Err(Error::message(&dest, "path is outside the tree"));
            }
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|source| Error::io(parent, source))?;
            }
            match node {
                Node::Bytes { bytes, mode } => {
                    fs::write(&dest, bytes).map_err(|source| Error::io(&dest, source))?;
                    if let Some(mode) = mode {
                        fs::set_permissions(&dest, fs::Permissions::from_mode(*mode))
                            .map_err(|source| Error::io(&dest, source))?;
                    }
                }
                Node::Link { target } => {
                    symlink(target, &dest).map_err(|source| Error::io(&dest, source))?;
                }
            }
        }
        Ok(MaterializedRoot { dir })
    }
}

pub struct MaterializedRoot {
    dir: TempDir,
}

impl MaterializedRoot {
    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

impl std::fmt::Debug for MaterializedRoot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MaterializedRoot")
            .field("path", &self.dir.path())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_bytes_under_the_root_and_rejects_escape() {
        let tree = RootBuilder::new()
            .file("var/lib/pacman/local/pkg/desc", b"name\n")
            .unwrap()
            .materialize()
            .unwrap();
        let got = fs::read(tree.path().join("var/lib/pacman/local/pkg/desc")).unwrap();
        assert_eq!(got, b"name\n");
        assert!(RootBuilder::new().file("../x", b"z").is_err());
        assert!(RootBuilder::new().file("/abs", b"z").is_err());
        assert!(RootBuilder::new()
            .file("a", b"1")
            .unwrap()
            .file("a", b"2")
            .is_err());
        let linked = RootBuilder::new()
            .link("home/.config/hypr/a.lua", "../elsewhere/a.lua")
            .unwrap()
            .file_with_mode("home/.config/secret", b"no\n", 0)
            .unwrap()
            .materialize()
            .unwrap();
        assert_eq!(
            fs::read_link(linked.path().join("home/.config/hypr/a.lua")).unwrap(),
            std::path::PathBuf::from("../elsewhere/a.lua")
        );
        assert!(fs::read(linked.path().join("home/.config/secret")).is_err());
    }
}
