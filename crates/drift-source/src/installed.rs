use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::Error;
use crate::parse;

pub const PACKAGE_NAME: &str = "omarchy-settings";
pub const DEFAULT_OMARCHY_PATH: &str = "/usr/share/omarchy";
const PACKAGE_PREFIX: &str = "usr/share/omarchy/";
const TEMPLATE_PREFIX: &str = "config/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPaths {
    pub root: PathBuf,
    pub omarchy_path: PathBuf,
}

impl ReadPaths {
    pub fn new(root: impl Into<PathBuf>, omarchy_path: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            omarchy_path: omarchy_path.into(),
        }
    }

    pub fn system() -> Self {
        let omarchy = std::env::var_os("OMARCHY_PATH")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_OMARCHY_PATH));
        Self::new("/", omarchy)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upgrade {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    MtreeMissing,
    NotInMtree,
    HashMismatch,
    MissingOnDisk,
    Unreadable,
    UnsafePath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub path: String,
    pub sha256: Option<String>,
    pub mtree_sha256: Option<String>,
    pub warning: Option<Check>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPackage {
    pub version: String,
    pub templates: Vec<Template>,
    pub upgrades: Vec<Upgrade>,
    pub mtree_present: bool,
}

pub fn read_package(paths: &ReadPaths) -> Result<InstalledPackage, Error> {
    read_entries(paths, true)
}

pub(crate) fn read_entries(
    paths: &ReadPaths,
    config_only: bool,
) -> Result<InstalledPackage, Error> {
    let local = paths.root.join("var/lib/pacman/local");
    let dir = package_dir(&local)?;
    let desc_path = dir.join("desc");
    let desc = fs::read_to_string(&desc_path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            Error::MissingDesc
        } else {
            Error::io(&desc_path, source)
        }
    })?;
    if parse::field(&desc, "NAME").as_deref() != Some(PACKAGE_NAME) {
        return Err(Error::NotInstalled);
    }
    let version = parse::field(&desc, "VERSION").ok_or(Error::MissingVersion)?;
    let files_path = dir.join("files");
    let files_text = fs::read_to_string(&files_path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            Error::MissingFileList
        } else {
            Error::io(&files_path, source)
        }
    })?;
    let listed = parse::file_list(&files_text).ok_or(Error::MissingFileList)?;
    let mtree_path = dir.join("mtree");
    let mtree = match fs::read(&mtree_path) {
        Ok(bytes) => Some(parse::mtree_hashes(&bytes)?),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => return Err(Error::io(&mtree_path, source)),
    };
    let mut templates = Vec::new();
    for package_path in listed {
        if package_path.ends_with('/') || !package_path.starts_with(PACKAGE_PREFIX) {
            continue;
        }
        let relative = &package_path[PACKAGE_PREFIX.len()..];
        if config_only && !relative.starts_with(TEMPLATE_PREFIX) {
            continue;
        }
        templates.push(verify(paths, relative, mtree.as_ref()));
    }
    templates.sort_by(|left, right| left.path.cmp(&right.path));
    let upgrades = read_upgrades(&paths.root.join("var/log/pacman.log"))?;
    Ok(InstalledPackage {
        version,
        templates,
        upgrades,
        mtree_present: mtree.is_some(),
    })
}

fn verify(
    paths: &ReadPaths,
    relative: &str,
    mtree: Option<&std::collections::BTreeMap<String, String>>,
) -> Template {
    let package_path = format!("{PACKAGE_PREFIX}{relative}");
    if !is_safe_relative(relative) {
        return Template {
            path: relative.to_string(),
            sha256: None,
            mtree_sha256: None,
            warning: Some(Check::UnsafePath),
        };
    }
    let mtree_sha256 = mtree.and_then(|entries| entries.get(&package_path).cloned());
    let (sha256, disk) = hash_disk(&paths.omarchy_path.join(relative));
    let warning = match (mtree, &mtree_sha256, &sha256, disk) {
        (None, _, _, Disk::Unreadable) => Some(Check::Unreadable),
        (None, _, _, Disk::Missing) => Some(Check::MissingOnDisk),
        (None, _, _, Disk::Read) => Some(Check::MtreeMissing),
        (Some(_), None, _, _) => Some(Check::NotInMtree),
        (_, _, _, Disk::Missing) => Some(Check::MissingOnDisk),
        (_, _, _, Disk::Unreadable) => Some(Check::Unreadable),
        (_, Some(expected), Some(actual), Disk::Read) if expected != actual => {
            Some(Check::HashMismatch)
        }
        _ => None,
    };
    Template {
        path: relative.to_string(),
        sha256,
        mtree_sha256,
        warning,
    }
}

#[derive(Clone, Copy)]
enum Disk {
    Read,
    Missing,
    Unreadable,
}

fn hash_disk(path: &Path) -> (Option<String>, Disk) {
    match fs::read(path) {
        Ok(bytes) => (Some(sha256_hex(&bytes)), Disk::Read),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => (None, Disk::Missing),
        Err(_) => (None, Disk::Unreadable),
    }
}

fn read_upgrades(path: &Path) -> Result<Vec<Upgrade>, Error> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(parse::upgrades(&text)),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(source) => Err(Error::io(path, source)),
    }
}

fn package_dir(local: &Path) -> Result<PathBuf, Error> {
    let entries = match fs::read_dir(local) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::NotInstalled);
        }
        Err(source) => return Err(Error::io(local, source)),
    };
    let mut matches = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| Error::io(local, source))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("omarchy-settings-") {
            continue;
        }
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|source| Error::io(&path, source))?;
        if meta.is_dir() {
            matches.push(path);
        }
    }
    let mut confirmed = Vec::new();
    for path in matches {
        let desc_path = path.join("desc");
        let text = match fs::read_to_string(&desc_path) {
            Ok(text) => text,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source) => return Err(Error::io(&desc_path, source)),
        };
        if parse::field(&text, "NAME").as_deref() == Some(PACKAGE_NAME) {
            confirmed.push(path);
        }
    }
    match confirmed.len() {
        0 => Err(Error::NotInstalled),
        1 => Ok(confirmed.remove(0)),
        count => Err(Error::Ambiguous(count)),
    }
}

fn is_safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_template_root_is_the_share_directory() {
        assert_eq!(DEFAULT_OMARCHY_PATH, "/usr/share/omarchy");
        assert_eq!(PACKAGE_NAME, "omarchy-settings");
    }

    #[test]
    #[ignore = "reads the machine package database"]
    fn live_read_of_the_installed_package() {
        let installed = read_package(&ReadPaths::system()).unwrap();
        assert!(!installed.version.is_empty());
        assert!(installed.mtree_present);
        assert!(!installed.templates.is_empty());
        let mut warnings = 0;
        for template in &installed.templates {
            assert!(is_safe_relative(&template.path), "{}", template.path);
            match template.warning {
                None => {
                    assert_eq!(template.sha256, template.mtree_sha256);
                    assert!(template.sha256.is_some());
                }
                Some(Check::HashMismatch) => {
                    warnings += 1;
                    assert_ne!(template.sha256, template.mtree_sha256);
                }
                Some(_) => warnings += 1,
            }
        }
        eprintln!(
            "live version {} templates {} warnings {} upgrades {}",
            installed.version,
            installed.templates.len(),
            warnings,
            installed.upgrades.len()
        );
    }
}
