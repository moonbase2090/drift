use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use drift_core::{LiveFile, LiveState, Registry, RelPath, Template, Verification};

use crate::error::Error;
use crate::installed::{read_entries, Check, ReadPaths};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePaths {
    pub package: ReadPaths,
    pub home: PathBuf,
    pub skel: PathBuf,
}

impl SourcePaths {
    pub fn new(package: ReadPaths, home: impl Into<PathBuf>, skel: impl Into<PathBuf>) -> Self {
        Self {
            package,
            home: home.into(),
            skel: skel.into(),
        }
    }

    pub fn system() -> Self {
        let home = std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        Self::new(ReadPaths::system(), home, "/etc/skel")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tracked {
    pub template: Option<Template>,
    pub live: Option<LiveFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupGroup {
    pub dir: String,
    pub files: Vec<LiveFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub tracked: Vec<Tracked>,
    pub backups: Vec<BackupGroup>,
    pub ignored: Vec<Tracked>,
}

pub trait DirReader {
    fn children(&self, dir: &Path) -> Option<Vec<String>>;
}

pub struct StdDirReader;

impl DirReader for StdDirReader {
    fn children(&self, dir: &Path) -> Option<Vec<String>> {
        read_children(dir)
    }
}

pub fn is_backup_name(name: &str) -> bool {
    match name.rsplit_once(".bak.") {
        Some((stem, digits)) => {
            !stem.is_empty() && !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
        }
        None => false,
    }
}

pub fn enumerate(registry: &Registry, paths: &SourcePaths) -> Result<Vec<Tracked>, Error> {
    Ok(scan(registry, paths, &BTreeSet::new(), &StdDirReader)?.tracked)
}

pub fn scan(
    registry: &Registry,
    paths: &SourcePaths,
    ignored: &BTreeSet<RelPath>,
    reader: &dyn DirReader,
) -> Result<Listing, Error> {
    let installed = read_entries(&paths.package, false)?;
    let mut tracked = Vec::new();
    let mut seen_live = BTreeSet::new();

    for item in installed.templates {
        let Ok(path) = RelPath::new(&item.path) else {
            continue;
        };
        let verdict = verification(
            item.warning,
            item.mtree_sha256.as_deref(),
            &item.path,
            &paths.skel,
        );
        let Some(template) = registry.template(&path, verdict) else {
            continue;
        };
        let live_key = template.live_path().as_str().to_string();
        let live = live_file(registry, &paths.home, template.live_path());
        seen_live.insert(live_key);
        tracked.push(Tracked {
            template: Some(template),
            live,
        });
    }

    let mut ignored_items: Vec<Tracked> = Vec::new();
    let mut parents = BTreeSet::new();
    for item in &tracked {
        if let Some(template) = &item.template {
            if let Some(parent) = parent_of(template.live_path().as_str()) {
                parents.insert(parent);
            }
        }
    }
    let mut backups: BTreeMap<String, Vec<LiveFile>> = BTreeMap::new();
    for parent in parents {
        let Some(entries) = reader.children(&paths.home.join(&parent)) else {
            continue;
        };
        for name in entries {
            let live_path = format!("{parent}/{name}");
            if seen_live.contains(&live_path) {
                continue;
            }
            let Ok(path) = RelPath::new(&live_path) else {
                continue;
            };
            let Some(state) = live_state(&path.under(&paths.home)) else {
                continue;
            };
            let Some(live) = registry.live_file(&path, state) else {
                continue;
            };
            seen_live.insert(live_path);
            if ignored.contains(&path) {
                ignored_items.push(Tracked {
                    template: None,
                    live: Some(live),
                });
            } else if is_backup_name(&name) {
                backups.entry(parent.clone()).or_default().push(live);
            } else {
                tracked.push(Tracked {
                    template: None,
                    live: Some(live),
                });
            }
        }
    }

    let (mut tracked, mut from_templates): (Vec<Tracked>, Vec<Tracked>) =
        tracked.into_iter().partition(|item| {
            !item
                .template
                .as_ref()
                .is_some_and(|template| ignored.contains(template.live_path()))
        });
    ignored_items.append(&mut from_templates);
    tracked.sort_by_key(track_key);
    ignored_items.sort_by_key(track_key);
    let backups = backups
        .into_iter()
        .map(|(dir, mut files)| {
            files.sort_by(|a, b| a.path().as_str().cmp(b.path().as_str()));
            BackupGroup { dir, files }
        })
        .collect();
    Ok(Listing {
        tracked,
        backups,
        ignored: ignored_items,
    })
}

fn verification(
    warning: Option<Check>,
    mtree_sha256: Option<&str>,
    template_path: &str,
    skel: &Path,
) -> Verification {
    match warning {
        None => Verification::Verified,
        Some(Check::HashMismatch) => Verification::Mismatch,
        Some(Check::MissingOnDisk) => fallback_verification(mtree_sha256, template_path, skel),
        Some(_) => Verification::Unchecked,
    }
}

fn fallback_verification(
    mtree_sha256: Option<&str>,
    template_path: &str,
    skel: &Path,
) -> Verification {
    let (Some(expected), Some(actual)) = (mtree_sha256, skel_digest(skel, template_path)) else {
        return Verification::Unchecked;
    };
    if actual == expected {
        Verification::Verified
    } else {
        Verification::Mismatch
    }
}

fn skel_digest(skel: &Path, template_path: &str) -> Option<String> {
    let rest = template_path.strip_prefix("config/")?;
    if rest.is_empty() {
        return None;
    }
    let bytes = fs::read(skel.join(".config").join(rest)).ok()?;
    Some(crate::installed::sha256_hex(&bytes))
}

fn live_file(registry: &Registry, home: &Path, live: &RelPath) -> Option<LiveFile> {
    let state = live_state(&live.under(home))?;
    registry.live_file(live, state)
}

fn live_state(path: &Path) -> Option<LiveState> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return None,
        Err(_) => return Some(LiveState::Unreadable),
    };
    if meta.file_type().is_symlink() {
        return Some(match fs::read_link(path) {
            Ok(target) => LiveState::Symlink {
                target: target.to_string_lossy().into_owned(),
            },
            Err(_) => LiveState::Unreadable,
        });
    }
    if !meta.is_file() {
        return Some(LiveState::Unreadable);
    }
    match digest_file(path) {
        Ok(digested) => Some(LiveState::Regular {
            len: digested.len,
            utf8: digested.utf8,
            sha256: digested.sha256,
        }),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(LiveState::Unreadable),
    }
}

pub(crate) struct Digested {
    pub(crate) len: u64,
    pub(crate) utf8: bool,
    pub(crate) sha256: String,
}

pub(crate) fn digest_file(path: &Path) -> std::io::Result<Digested> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    let mut len = 0u64;
    let mut pending = [0u8; 3];
    let mut pending_len = 0usize;
    let mut utf8 = true;
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
        if utf8 && !accept_utf8(&mut pending, &mut pending_len, &buf[..read]) {
            utf8 = false;
        }
        len += read as u64;
    }
    if pending_len > 0 {
        utf8 = false;
    }
    Ok(Digested {
        len,
        utf8,
        sha256: hex_digest(&hasher.finalize()),
    })
}

fn accept_utf8(pending: &mut [u8; 3], pending_len: &mut usize, chunk: &[u8]) -> bool {
    let mut scratch = [0u8; 8195];
    let total = *pending_len + chunk.len();
    if total > scratch.len() {
        return false;
    }
    scratch[..*pending_len].copy_from_slice(&pending[..*pending_len]);
    scratch[*pending_len..total].copy_from_slice(chunk);
    match std::str::from_utf8(&scratch[..total]) {
        Ok(_) => {
            *pending_len = 0;
            true
        }
        Err(error) if error.error_len().is_none() => {
            let tail = total - error.valid_up_to();
            if tail > pending.len() {
                return false;
            }
            pending[..tail].copy_from_slice(&scratch[error.valid_up_to()..total]);
            *pending_len = tail;
            true
        }
        Err(_) => false,
    }
}

fn hex_digest(digest: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

fn read_children(dir: &Path) -> Option<Vec<String>> {
    let entries = fs::read_dir(dir).ok()?;
    let mut names = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if name.is_empty() || name.contains('/') {
            continue;
        }
        let Ok(meta) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        let kind = meta.file_type();
        if kind.is_file() || kind.is_symlink() {
            names.push(name.to_string());
        }
    }
    Some(names)
}

fn parent_of(path: &str) -> Option<String> {
    path.rsplit_once('/').map(|(parent, _)| parent.to_string())
}

fn track_key(item: &Tracked) -> (String, String) {
    (
        item.template
            .as_ref()
            .map(|template| template.path().as_str().to_string())
            .unwrap_or_default(),
        item.live
            .as_ref()
            .map(|live| live.path().as_str().to_string())
            .unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::accept_utf8;

    #[test]
    fn utf8_continues_across_a_read_boundary() {
        let mut pending = [0u8; 3];
        let mut pending_len = 0;
        assert!(accept_utf8(&mut pending, &mut pending_len, &[b'a', 0xc3]));
        assert_eq!(pending_len, 1);
        assert_eq!(pending[0], 0xc3);
        assert!(accept_utf8(&mut pending, &mut pending_len, &[0xa9, b'b']));
        assert_eq!(pending_len, 0);
        assert!(accept_utf8(&mut pending, &mut pending_len, &[0xe2, 0x82]));
        assert_eq!(pending_len, 2);
        assert!(accept_utf8(&mut pending, &mut pending_len, &[0xac]));
        assert_eq!(pending_len, 0);
    }

    #[test]
    fn a_broken_byte_stops_the_utf8_check() {
        let mut pending = [0u8; 3];
        let mut pending_len = 0;
        assert!(!accept_utf8(&mut pending, &mut pending_len, &[0xff, 0xfe]));
        assert!(!accept_utf8(&mut pending, &mut pending_len, &[0xc3, 0x20]));
    }
}
