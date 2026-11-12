use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use drift_core::{LiveState, Registry, RelPath, ResolveInput, Template};

use crate::catalog::{scan, StdDirReader, Tracked};
use crate::error::Error;
use crate::installed::read_package;
use crate::report::ScanPaths;
use crate::resolve::{resolve_bases, BaseQuery};

const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

pub struct LoadedInputs {
    pub package_version: String,
    pub inputs: Vec<ResolveInput>,
    pub unknown: Vec<String>,
}

fn read_regular(path: &Path) -> Option<Vec<u8>> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    fs::read(path).ok()
}

fn skel_path(template: &Template, skel: &Path) -> Option<PathBuf> {
    let rest = template.path().as_str().strip_prefix("config/")?;
    (!rest.is_empty()).then(|| skel.join(".config").join(rest))
}

fn read_template(template: &Template, paths: &ScanPaths) -> Option<Vec<u8>> {
    let primary = template
        .mapping()
        .template_path(&paths.source.package.omarchy_path);
    read_regular(&primary)
        .or_else(|| skel_path(template, &paths.source.skel).and_then(|path| read_regular(&path)))
}

fn live_path(tracked: &Tracked) -> Option<&RelPath> {
    tracked
        .template
        .as_ref()
        .map(Template::live_path)
        .or_else(|| tracked.live.as_ref().map(|live| live.path()))
}

pub fn load_resolve_inputs(
    registry: &Registry,
    paths: &ScanPaths,
    ignored: &BTreeSet<RelPath>,
    targets: &[RelPath],
) -> Result<LoadedInputs, Error> {
    let installed = read_package(&paths.source.package)?;
    let listing = scan(registry, &paths.source, ignored, &StdDirReader)?;
    let mut chosen: Vec<(&RelPath, &Tracked)> = Vec::new();
    let mut unknown = Vec::new();
    for target in targets {
        match listing
            .tracked
            .iter()
            .find(|tracked| live_path(tracked) == Some(target))
        {
            Some(tracked) => {
                if !chosen.iter().any(|(seen, _)| *seen == target) {
                    chosen.push((target, tracked));
                }
            }
            None => unknown.push(target.as_str().to_string()),
        }
    }
    let queries: Vec<BaseQuery<'_>> = chosen
        .iter()
        .filter_map(|(_, tracked)| {
            let mapping = tracked.template.as_ref()?.mapping();
            Some(BaseQuery {
                root: &mapping.root,
                rel: &mapping.rel,
                template: &mapping.template,
            })
        })
        .collect();
    let mut resolved = resolve_bases(&paths.resolve, &installed.version, &queries)?.into_iter();
    let mut inputs = Vec::new();
    for (target, tracked) in chosen {
        let base = tracked
            .template
            .as_ref()
            .and_then(|_| resolved.next())
            .and_then(|found| found.bytes);
        let (ours, link, unreadable) = match tracked.live.as_ref().map(|live| live.state()) {
            None => (None, false, false),
            Some(LiveState::Symlink { .. }) => (None, true, false),
            Some(LiveState::Unreadable) => (None, false, true),
            Some(LiveState::Regular { .. }) => {
                let bytes = read_regular(&target.under(&paths.source.home));
                let missing = bytes.is_none();
                (bytes, false, missing)
            }
        };
        let (root, rel, theirs, volatile) = match tracked.template.as_ref() {
            Some(template) => (
                template.mapping().root.clone(),
                template.mapping().rel.clone(),
                read_template(template, paths),
                template.is_volatile(),
            ),
            None => (String::new(), target.clone(), None, false),
        };
        inputs.push(ResolveInput {
            path: target.as_str().to_string(),
            root,
            rel,
            volatile: volatile || tracked.live.as_ref().is_some_and(|live| live.is_volatile()),
            link,
            unreadable,
            base,
            ours,
            theirs,
        });
    }
    Ok(LoadedInputs {
        package_version: installed.version,
        inputs,
        unknown,
    })
}
