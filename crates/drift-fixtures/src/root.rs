use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::error::Error;
use crate::tree::check_rel;

#[derive(Debug, Default)]
pub struct RootBuilder {
    files: Vec<(PathBuf, Vec<u8>)>,
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
        let path = path.into();
        check_rel(&path)?;
        if self.files.iter().any(|(existing, _)| existing == &path) {
            return Err(Error::message(&path, "duplicate fixture path"));
        }
        self.files.push((path, bytes.as_ref().to_vec()));
        Ok(self)
    }

    pub fn materialize(&self) -> Result<MaterializedRoot, Error> {
        let dir = TempDir::new().map_err(|source| Error::io("root", source))?;
        for (path, bytes) in &self.files {
            let dest = dir.path().join(path);
            if !dest.starts_with(dir.path()) {
                return Err(Error::message(&dest, "path is outside the tree"));
            }
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|source| Error::io(parent, source))?;
            }
            fs::write(&dest, bytes).map_err(|source| Error::io(&dest, source))?;
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
    }
}
