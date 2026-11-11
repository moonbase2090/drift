use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path};
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};

use crate::error::Error;

const KIND_FILE: u8 = 1;
const KIND_SYMLINK: u8 = 2;
const KIND_DIR: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TreeDigest([u8; 32]);

impl TreeDigest {
    pub fn as_bytes(self) -> [u8; 32] {
        self.0
    }

    pub fn to_hex(self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0xf) as usize] as char);
        }
        out
    }
}

impl std::fmt::Display for TreeDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

struct Item {
    key: Vec<u8>,
    kind: u8,
    mode: u32,
    mtime_secs: u64,
    mtime_nanos: u32,
    payload: Vec<u8>,
}

pub fn hash_tree(root: &Path) -> Result<TreeDigest, Error> {
    let mut items = Vec::new();
    walk(root, root, &mut items)?;
    items.sort_by(|left, right| left.key.cmp(&right.key));
    let mut hasher = Sha256::new();
    for item in items {
        update_u32(&mut hasher, item.key.len() as u32);
        hasher.update(&item.key);
        hasher.update([item.kind]);
        update_u32(&mut hasher, item.mode);
        update_u64(&mut hasher, item.mtime_secs);
        update_u32(&mut hasher, item.mtime_nanos);
        update_u64(&mut hasher, item.payload.len() as u64);
        hasher.update(&item.payload);
    }
    let digest = hasher.finalize();
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&digest);
    Ok(TreeDigest(bytes))
}

fn walk(root: &Path, dir: &Path, items: &mut Vec<Item>) -> Result<(), Error> {
    let meta = fs::symlink_metadata(dir).map_err(|source| Error::io(dir, source))?;
    items.push(item(root, dir, &meta)?);
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Ok(());
    }
    let mut children = fs::read_dir(dir)
        .map_err(|source| Error::io(dir, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| Error::io(dir, source))?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        walk(root, &child.path(), items)?;
    }
    Ok(())
}

fn item(root: &Path, path: &Path, meta: &fs::Metadata) -> Result<Item, Error> {
    let key = if path == root {
        Vec::new()
    } else {
        let rel = path.strip_prefix(root).map_err(|source| {
            Error::message(
                path,
                format!("path is outside {root}: {source}", root = root.display()),
            )
        })?;
        rel_key(rel)?
    };
    let modified = meta.modified().map_err(|source| Error::io(path, source))?;
    let since_epoch = modified
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::message(path, "mtime is before the unix epoch"))?;
    let mode = meta.permissions().mode() & 0o7777;
    let (kind, payload) = if meta.file_type().is_symlink() {
        let target = fs::read_link(path).map_err(|source| Error::io(path, source))?;
        (KIND_SYMLINK, target.as_os_str().as_bytes().to_vec())
    } else if meta.is_dir() {
        (KIND_DIR, Vec::new())
    } else if meta.is_file() {
        let bytes = fs::read(path).map_err(|source| Error::io(path, source))?;
        (KIND_FILE, bytes)
    } else {
        return Err(Error::message(path, "unsupported file type"));
    };
    Ok(Item {
        key,
        kind,
        mode,
        mtime_secs: since_epoch.as_secs(),
        mtime_nanos: since_epoch.subsec_nanos(),
        payload,
    })
}

fn rel_key(path: &Path) -> Result<Vec<u8>, Error> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.as_bytes()),
            _ => return Err(Error::message(path, "path is outside the tree")),
        }
    }
    let mut key = Vec::new();
    key.extend_from_slice(&(parts.len() as u32).to_be_bytes());
    for part in parts {
        key.extend_from_slice(&(part.len() as u32).to_be_bytes());
        key.extend_from_slice(part);
    }
    Ok(key)
}

fn update_u32(hasher: &mut Sha256, value: u32) {
    hasher.update(value.to_be_bytes());
}

fn update_u64(hasher: &mut Sha256, value: u64) {
    hasher.update(value.to_be_bytes());
}
