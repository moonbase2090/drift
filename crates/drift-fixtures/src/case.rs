use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::Error;
use crate::tree::{check_rel, FixtureBuilder, Node, Root, DEFAULT_DIR_MODE, DEFAULT_FILE_MODE};
use crate::{BADGES, STATUSES};

#[derive(Debug)]
pub struct Case {
    pub directory: PathBuf,
    pub expected: Expected,
    pub builder: FixtureBuilder,
}

pub struct MaterializedCase {
    pub case: Case,
    pub trees: crate::tree::FixtureTrees,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Expected {
    pub files: Vec<ExpectedFile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ExpectedFile {
    pub path: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub badge: Option<String>,
    #[serde(default)]
    pub base: Option<Presence>,
    #[serde(default)]
    pub ours: Option<Presence>,
    #[serde(default)]
    pub theirs: Option<Presence>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Presence {
    Absent,
    Same,
    Changed,
}

#[derive(Debug, Default, Deserialize)]
struct Layout {
    #[serde(default)]
    modes: BTreeMap<String, String>,
    #[serde(default)]
    symlinks: BTreeMap<String, String>,
    #[serde(default)]
    absent: Vec<String>,
}

pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

pub fn case_dirs() -> Result<Vec<PathBuf>, Error> {
    let root = fixtures_dir();
    let mut found = Vec::new();
    collect_cases(&root, &mut found)?;
    found.sort();
    Ok(found)
}

pub fn load_case(directory: &Path) -> Result<Case, Error> {
    let expected_path = directory.join("expected.json");
    let expected = read_expected(&expected_path)?;
    let layout = read_layout(&directory.join("layout.json"))?;
    let mut builder = FixtureBuilder::new();
    ingest_tree(directory, "base", Root::Bases, &layout, &mut builder)?;
    ingest_tree(directory, "ours", Root::Config, &layout, &mut builder)?;
    ingest_tree(directory, "theirs", Root::Templates, &layout, &mut builder)?;
    apply_layout_extras(&layout, &mut builder)?;
    check_layout_modes(&layout, &builder)?;
    Ok(Case {
        directory: directory.to_path_buf(),
        expected,
        builder,
    })
}

pub fn materialize_case(directory: &Path) -> Result<MaterializedCase, Error> {
    let case = load_case(directory)?;
    let trees = case.builder.materialize()?;
    Ok(MaterializedCase { case, trees })
}

fn collect_cases(dir: &Path, found: &mut Vec<PathBuf>) -> Result<(), Error> {
    if !dir.exists() {
        return Ok(());
    }
    if dir.join("expected.json").is_file() {
        found.push(dir.to_path_buf());
        return Ok(());
    }
    let mut children = fs::read_dir(dir)
        .map_err(|source| Error::io(dir, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| Error::io(dir, source))?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let path = child.path();
        if path.is_dir() {
            collect_cases(&path, found)?;
        }
    }
    Ok(())
}

fn read_expected(path: &Path) -> Result<Expected, Error> {
    let text = fs::read_to_string(path).map_err(|source| Error::io(path, source))?;
    let expected: Expected =
        serde_json::from_str(&text).map_err(|source| Error::json(path, source))?;
    let mut seen = BTreeMap::<String, ()>::new();
    for file in &expected.files {
        check_rel(Path::new(&file.path))
            .map_err(|_| Error::message(path, format!("invalid fixture path {}", file.path)))?;
        if seen.insert(file.path.clone(), ()).is_some() {
            return Err(Error::message(
                path,
                format!("duplicate path {}", file.path),
            ));
        }
        if let Some(status) = &file.status {
            if !STATUSES.contains(&status.as_str()) {
                return Err(Error::message(path, format!("unknown status {status}")));
            }
        }
        if let Some(badge) = &file.badge {
            if !BADGES.contains(&badge.as_str()) {
                return Err(Error::message(path, format!("unknown badge {badge}")));
            }
        }
    }
    Ok(expected)
}

fn read_layout(path: &Path) -> Result<Layout, Error> {
    if !path.exists() {
        return Ok(Layout::default());
    }
    let text = fs::read_to_string(path).map_err(|source| Error::io(path, source))?;
    serde_json::from_str(&text).map_err(|source| Error::json(path, source))
}

fn ingest_tree(
    case_dir: &Path,
    tree_name: &str,
    root: Root,
    layout: &Layout,
    builder: &mut FixtureBuilder,
) -> Result<(), Error> {
    let tree = case_dir.join(tree_name);
    if !tree.exists() {
        return Ok(());
    }
    let mut found = Vec::new();
    walk_source(&tree, Path::new(""), &mut found)?;
    for (rel, node) in found {
        let case_path = format!("{tree_name}/{}", rel.display());
        if layout.absent.iter().any(|item| item == &case_path) {
            return Err(Error::message(
                tree.join(&rel),
                "path is both present and absent",
            ));
        }
        let node = match node {
            Node::File { bytes, mode } => {
                let mode = layout_mode(layout, &case_path)?.unwrap_or(mode);
                Node::File { bytes, mode }
            }
            Node::Directory { mode } => {
                let mode = layout_mode(layout, &case_path)?.unwrap_or(mode);
                Node::Directory { mode }
            }
            other => other,
        };
        if let Some(target) = layout.symlinks.get(&case_path) {
            match &node {
                Node::Symlink { target: existing } if existing == target => {}
                _ => {
                    return Err(Error::message(
                        tree.join(&rel),
                        "layout symlink disagrees with the file on disk",
                    ));
                }
            }
        }
        builder.put(root, rel, node)?;
    }
    Ok(())
}

fn check_layout_modes(layout: &Layout, builder: &FixtureBuilder) -> Result<(), Error> {
    for case_path in layout.modes.keys() {
        let (root, rel) = split_case_path(case_path)?;
        let found = builder.entries().iter().any(|entry| {
            entry.root == root
                && entry.path == rel
                && matches!(entry.node, Node::File { .. } | Node::Directory { .. })
        });
        if !found {
            return Err(Error::message(
                case_path,
                "mode does not match a file or directory in the case",
            ));
        }
    }
    Ok(())
}

fn apply_layout_extras(layout: &Layout, builder: &mut FixtureBuilder) -> Result<(), Error> {
    for case_path in &layout.absent {
        let (root, rel) = split_case_path(case_path)?;
        builder.put(root, rel, Node::absent())?;
    }
    for (case_path, target) in &layout.symlinks {
        let (root, rel) = split_case_path(case_path)?;
        if builder
            .entries()
            .iter()
            .any(|entry| entry.root == root && entry.path == rel)
        {
            continue;
        }
        builder.put(root, rel, Node::symlink(target))?;
    }
    Ok(())
}

fn walk_source(dir: &Path, prefix: &Path, out: &mut Vec<(PathBuf, Node)>) -> Result<(), Error> {
    let mut children = fs::read_dir(dir)
        .map_err(|source| Error::io(dir, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| Error::io(dir, source))?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let path = child.path();
        let name = child.file_name();
        let rel = prefix.join(&name);
        let file_type = child
            .file_type()
            .map_err(|source| Error::io(&path, source))?;
        if file_type.is_symlink() {
            let target = fs::read_link(&path).map_err(|source| Error::io(&path, source))?;
            let text = target
                .to_str()
                .ok_or_else(|| Error::message(&path, "symlink target is not utf-8"))?;
            out.push((rel, Node::symlink(text)));
            continue;
        }
        if file_type.is_dir() {
            let mode = mode_of(&path)?.unwrap_or(DEFAULT_DIR_MODE);
            out.push((rel.clone(), Node::directory(mode)));
            walk_source(&path, &rel, out)?;
            continue;
        }
        if file_type.is_file() {
            let bytes = fs::read(&path).map_err(|source| Error::io(&path, source))?;
            let mode = mode_of(&path)?.unwrap_or(DEFAULT_FILE_MODE);
            out.push((rel, Node::file_mode(bytes, mode)));
            continue;
        }
        return Err(Error::message(&path, "unsupported file type"));
    }
    Ok(())
}

fn layout_mode(layout: &Layout, case_path: &str) -> Result<Option<u32>, Error> {
    let Some(text) = layout.modes.get(case_path) else {
        return Ok(None);
    };
    let mode = u32::from_str_radix(text, 8)
        .map_err(|_| Error::message(case_path, format!("mode {text} is not octal")))?;
    if mode > 0o7777 {
        return Err(Error::message(
            case_path,
            format!("mode {text} is out of range"),
        ));
    }
    Ok(Some(mode))
}

fn split_case_path(case_path: &str) -> Result<(Root, PathBuf), Error> {
    let Some((head, rest)) = case_path.split_once('/') else {
        return Err(Error::message(case_path, "layout path needs a tree name"));
    };
    let root = match head {
        "ours" => Root::Config,
        "theirs" => Root::Templates,
        "base" => Root::Bases,
        _ => {
            return Err(Error::message(
                case_path,
                "layout path tree must be base, ours, or theirs",
            ))
        }
    };
    if rest.is_empty() {
        return Err(Error::message(
            case_path,
            "layout path is missing a relative path",
        ));
    }
    let rel = PathBuf::from(rest);
    check_rel(&rel)?;
    Ok((root, rel))
}

fn mode_of(path: &Path) -> Result<Option<u32>, Error> {
    let meta = fs::symlink_metadata(path).map_err(|source| Error::io(path, source))?;
    if meta.file_type().is_symlink() {
        return Ok(None);
    }
    Ok(Some(meta.permissions().mode() & 0o777))
}
