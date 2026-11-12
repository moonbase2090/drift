use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use drift_core::{LiveFile, LiveState, Registry, RelPath, Template, Verification};

use crate::error::Error;
use crate::installed::{read_package, Check, ReadPaths};

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

pub fn enumerate(registry: &Registry, paths: &SourcePaths) -> Result<Vec<Tracked>, Error> {
    let installed = read_package(&paths.package)?;
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

    let mut parents = BTreeSet::new();
    for item in &tracked {
        if let Some(template) = &item.template {
            if let Some(parent) = parent_of(template.live_path().as_str()) {
                parents.insert(parent);
            }
        }
    }
    for parent in parents {
        let Some(entries) = read_children(&paths.home.join(&parent)) else {
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
            tracked.push(Tracked {
                template: None,
                live: Some(live),
            });
        }
    }

    tracked.sort_by_key(track_key);
    Ok(tracked)
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
    match fs::read(path) {
        Ok(bytes) => Some(LiveState::Regular {
            len: bytes.len() as u64,
            utf8: std::str::from_utf8(&bytes).is_ok(),
        }),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(LiveState::Unreadable),
    }
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
