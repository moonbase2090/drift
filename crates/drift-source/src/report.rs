use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use drift_core::{
    classify, merge3, BackupSummary, Base, Classification, MergeOutcome, Registry, RelPath,
    ScanEntry, ScanReport, Template, Versions, MAX_DIFF_BYTES,
};

use crate::bases::read_reviewed_version;
use crate::catalog::{digest_file, scan, SourcePaths, StdDirReader, Tracked};
use crate::error::Error;
use crate::installed::{read_package, sha256_hex};
use crate::resolve::{resolve_bases, BaseQuery, ResolvePaths};

pub struct ScanPaths {
    pub source: SourcePaths,
    pub resolve: ResolvePaths,
}

struct Side {
    hash: String,
    bytes: Option<Vec<u8>>,
}

struct Item {
    tracked: Tracked,
    ours: Option<Side>,
    theirs: Option<Side>,
}

pub fn build_report(
    registry: &Registry,
    paths: &ScanPaths,
    ignored: &BTreeSet<RelPath>,
) -> Result<ScanReport, Error> {
    let installed = read_package(&paths.source.package)?;
    let listing = scan(registry, &paths.source, ignored, &StdDirReader)?;
    let mut items: Vec<Item> = listing
        .tracked
        .into_iter()
        .map(|tracked| {
            let ours = tracked
                .live
                .as_ref()
                .map(|live| read_side(&live.path().under(&paths.source.home)));
            let theirs = tracked
                .template
                .as_ref()
                .map(|template| read_template(template, paths));
            Item {
                tracked,
                ours,
                theirs,
            }
        })
        .collect();

    let wanted: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| needs_base(item))
        .map(|(index, _)| index)
        .collect();
    let queries: Vec<BaseQuery<'_>> = wanted
        .iter()
        .filter_map(|index| {
            let template = items[*index].tracked.template.as_ref()?;
            let mapping = template.mapping();
            Some(BaseQuery {
                root: &mapping.root,
                rel: &mapping.rel,
                template: &mapping.template,
            })
        })
        .collect();
    let resolved = resolve_bases(&paths.resolve, &installed.version, &queries)?;

    let mut bases: Vec<Option<(Base<String>, Vec<u8>)>> = items.iter().map(|_| None).collect();
    for (index, found) in wanted.iter().zip(resolved) {
        if let Some(bytes) = found.bytes {
            if let Some(base) = Base::new(sha256_hex(&bytes), found.provenance) {
                bases[*index] = Some((base, bytes));
            }
        }
    }

    let mut entries = Vec::new();
    for (item, base) in items.drain(..).zip(bases) {
        let versions = Versions {
            base: base.as_ref().map(|(base, _)| base.clone()),
            ours: item.ours.as_ref().map(|side| side.hash.clone()),
            theirs: item.theirs.as_ref().map(|side| side.hash.clone()),
        };
        let base_bytes = base.as_ref().map(|(_, bytes)| bytes.as_slice());
        let Some(result) = classify(&versions, || {
            merge_outcome(
                base_bytes,
                item.ours.as_ref().and_then(|side| side.bytes.as_deref()),
                item.theirs.as_ref().and_then(|side| side.bytes.as_deref()),
            )
        }) else {
            continue;
        };
        entries.push(entry(&item.tracked, result));
    }

    let backups = listing
        .backups
        .iter()
        .map(|group| BackupSummary {
            dir: group.dir.clone(),
            count: group.files.len(),
        })
        .collect();
    Ok(ScanReport {
        package_version: installed.version,
        reviewed_version: read_reviewed_version(&paths.resolve.state),
        entries,
        backups,
        ignored: listing.ignored.len(),
    })
}

fn needs_base(item: &Item) -> bool {
    match (&item.ours, &item.theirs) {
        (Some(ours), Some(theirs)) => ours.hash != theirs.hash,
        (None, Some(_)) => true,
        _ => false,
    }
}

fn entry(tracked: &Tracked, result: Classification) -> ScanEntry {
    let (path, template) = match (&tracked.template, &tracked.live) {
        (Some(template), _) => (
            template.live_path().as_str().to_string(),
            Some(template.path().as_str().to_string()),
        ),
        (None, Some(live)) => (live.path().as_str().to_string(), None),
        (None, None) => (String::new(), None),
    };
    let mut notes = Vec::new();
    if let Some(live) = &tracked.live {
        let note = live.note();
        if !note.is_empty() {
            notes.push(note);
        }
    } else if let Some(template) = &tracked.template {
        if !template.note().is_empty() {
            notes.push(template.note().to_string());
        }
    }
    if tracked.template.as_ref().is_some_and(Template::has_warning) {
        notes.push("package file does not match its recorded digest".to_string());
    }
    let volatile = tracked.template.as_ref().is_some_and(Template::is_volatile)
        || tracked.live.as_ref().is_some_and(|live| live.is_volatile());
    ScanEntry {
        path,
        template,
        status: result.status,
        badge: result.badge,
        base: result.provenance,
        volatile,
        note: notes.join("; "),
    }
}

fn merge_outcome(base: Option<&[u8]>, ours: Option<&[u8]>, theirs: Option<&[u8]>) -> MergeOutcome {
    let (Some(base), Some(ours), Some(theirs)) = (base, ours, theirs) else {
        return MergeOutcome::Unavailable;
    };
    let (Ok(base), Ok(ours), Ok(theirs)) = (
        std::str::from_utf8(base),
        std::str::from_utf8(ours),
        std::str::from_utf8(theirs),
    ) else {
        return MergeOutcome::Unavailable;
    };
    if merge3(base, ours, theirs).is_clean() {
        MergeOutcome::Clean
    } else {
        MergeOutcome::Overlap
    }
}

fn read_template(template: &Template, paths: &ScanPaths) -> Side {
    let primary = template
        .mapping()
        .template_path(&paths.source.package.omarchy_path);
    if let Some(side) = try_side(&primary) {
        return side;
    }
    if let Some(side) = skel_path(template, &paths.source.skel).and_then(|path| try_side(&path)) {
        return side;
    }
    unreadable(&primary)
}

fn skel_path(template: &Template, skel: &Path) -> Option<PathBuf> {
    let rest = template.path().as_str().strip_prefix("config/")?;
    (!rest.is_empty()).then(|| skel.join(".config").join(rest))
}

fn read_side(path: &Path) -> Side {
    try_side(path).unwrap_or_else(|| unreadable(path))
}

fn try_side(path: &Path) -> Option<Side> {
    let digested = digest_file(path).ok()?;
    let bytes = if digested.utf8 && digested.len <= MAX_DIFF_BYTES {
        fs::read(path)
            .ok()
            .filter(|bytes| sha256_hex(bytes) == digested.sha256)
    } else {
        None
    };
    Some(Side {
        hash: digested.sha256,
        bytes,
    })
}

fn unreadable(path: &Path) -> Side {
    Side {
        hash: format!("unreadable:{}", path.display()),
        bytes: None,
    }
}
