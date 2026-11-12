use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use drift_core::{BaseProvenance, RelPath};

use crate::bases::{read_base, read_reviewed_version};
use crate::error::Error;
use crate::installed::{sha256_hex, Upgrade, PACKAGE_NAME};
use crate::parse;

const PACKAGE_PREFIX: &str = "usr/share/omarchy/";
const CACHE_ARCH: &str = "x86_64";
const MAX_BASE_BYTES: usize = 8 * 1024 * 1024;

pub struct ResolvePaths {
    pub share: PathBuf,
    pub state: PathBuf,
    pub snapshots: PathBuf,
    pub cache: PathBuf,
    pub log: PathBuf,
}

pub struct BaseQuery<'a> {
    pub root: &'a str,
    pub rel: &'a RelPath,
    pub template: &'a RelPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBase {
    pub provenance: BaseProvenance,
    pub bytes: Option<Vec<u8>>,
}

pub fn resolve_bases(
    paths: &ResolvePaths,
    installed: &str,
    files: &[BaseQuery<'_>],
) -> Result<Vec<ResolvedBase>, Error> {
    let mut resolved: Vec<ResolvedBase> = files
        .iter()
        .map(|file| match read_base(&paths.share, file.root, file.rel) {
            Some(bytes) => known(BaseProvenance::Reviewed, bytes),
            None => unknown(),
        })
        .collect();
    let upgrades = read_log(&paths.log)?;
    let versions = candidate_versions(read_reviewed_version(&paths.state), &upgrades, installed);
    for version in &versions {
        let mut wanted = BTreeSet::new();
        for (index, file) in files.iter().enumerate() {
            if resolved[index].bytes.is_some() {
                continue;
            }
            if let Some(bytes) = read_snapshot(&paths.snapshots, version, file.template) {
                resolved[index] = known(BaseProvenance::Snapshot, bytes);
            } else if let Some(path) = package_path(file.template) {
                wanted.insert(path);
            }
        }
        if wanted.is_empty() {
            continue;
        }
        let Some(cache) = load_cache(&paths.cache, version, &wanted) else {
            continue;
        };
        for (index, file) in files.iter().enumerate() {
            if resolved[index].bytes.is_some() {
                continue;
            }
            let Some(path) = package_path(file.template) else {
                continue;
            };
            if let Some(bytes) = cache.get(&path) {
                resolved[index] = known(BaseProvenance::Cache, bytes.clone());
            }
        }
    }
    Ok(resolved)
}

fn known(provenance: BaseProvenance, bytes: Vec<u8>) -> ResolvedBase {
    ResolvedBase {
        provenance,
        bytes: Some(bytes),
    }
}

fn unknown() -> ResolvedBase {
    ResolvedBase {
        provenance: BaseProvenance::None,
        bytes: None,
    }
}

fn candidate_versions(
    reviewed: Option<String>,
    upgrades: &[Upgrade],
    installed: &str,
) -> Vec<String> {
    let mut versions = Vec::new();
    if let Some(version) = reviewed {
        if version != installed && version_ok(&version) {
            versions.push(version);
        }
    }
    if let Some(previous) = upgrades.iter().rev().find(|item| item.to == installed) {
        if version_ok(&previous.from) && !versions.iter().any(|version| version == &previous.from) {
            versions.push(previous.from.clone());
        }
    }
    versions
}

fn version_ok(version: &str) -> bool {
    !version.is_empty()
        && version
            .chars()
            .all(|c| !c.is_whitespace() && c != '/' && c != '\0')
}

fn read_log(path: &Path) -> Result<Vec<Upgrade>, Error> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(parse::upgrades(&text)),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(source) => Err(Error::io(path, source)),
    }
}

fn read_snapshot(snapshots: &Path, version: &str, template: &RelPath) -> Option<Vec<u8>> {
    if !version_ok(version) {
        return None;
    }
    let path = template.under(&snapshots.join(version));
    if !path.starts_with(snapshots) {
        return None;
    }
    read_regular(&path)
}

fn package_path(template: &RelPath) -> Option<String> {
    let relative = template.as_str();
    if relative.is_empty() {
        return None;
    }
    Some(format!("{PACKAGE_PREFIX}{relative}"))
}

fn cache_archive(cache: &Path, version: &str) -> Option<PathBuf> {
    if !version_ok(version) {
        return None;
    }
    let name = format!("{PACKAGE_NAME}-{version}-{CACHE_ARCH}.pkg.tar.zst");
    let path = cache.join(name);
    path.starts_with(cache).then_some(path)
}

fn load_cache(
    cache: &Path,
    version: &str,
    wanted: &BTreeSet<String>,
) -> Option<BTreeMap<String, Vec<u8>>> {
    let path = cache_archive(cache, version)?;
    let bytes = read_regular(&path)?;
    let cursor = std::io::Cursor::new(bytes);
    let decoder = zstd::Decoder::new(cursor).ok()?;
    let mut archive = tar::Archive::new(decoder);
    let entries = archive.entries().ok()?;
    let mut mtree = None;
    let mut files = BTreeMap::new();
    for entry in entries {
        let mut entry = entry.ok()?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let name = entry_name(entry.path().ok()?.as_ref())?;
        if name == ".MTREE" {
            if mtree.is_none() {
                mtree = read_limited(&mut entry, MAX_BASE_BYTES);
            }
            continue;
        }
        if wanted.contains(&name) && !files.contains_key(&name) {
            if let Some(bytes) = read_limited(&mut entry, MAX_BASE_BYTES) {
                files.insert(name, bytes);
            }
        }
    }
    let digests = parse::mtree_hashes(mtree.as_deref()?).ok()?;
    let mut verified = BTreeMap::new();
    for (path, bytes) in files {
        let digest = sha256_hex(&bytes);
        if digests.get(&path).map(String::as_str) == Some(digest.as_str()) {
            verified.insert(path, bytes);
        }
    }
    Some(verified)
}

fn entry_name(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    let text = text.strip_prefix("./").unwrap_or(text);
    let safe = !text.is_empty()
        && !text.starts_with('/')
        && text
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
    safe.then(|| text.to_string())
}

fn read_regular(path: &Path) -> Option<Vec<u8>> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_BASE_BYTES as u64 {
        return None;
    }
    let mut file = File::open(path).ok()?;
    read_limited(&mut file, MAX_BASE_BYTES)
}

fn read_limited(reader: &mut impl Read, cap: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let read = reader.read(&mut chunk).ok()?;
        if read == 0 {
            return Some(bytes);
        }
        if bytes.len() + read > cap {
            return None;
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
}
