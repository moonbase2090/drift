use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use drift_core::RelPath;
use drift_source::{ignored_path, read_ignored};

use crate::notify::{ensure_state_dir, symlink_at, NotifyError};

pub fn add_ignored(state: &Path, paths: &[RelPath]) -> Result<usize, NotifyError> {
    let mut all = read_ignored(state);
    let before = all.len();
    all.extend(paths.iter().cloned());
    let added = all.len() - before;
    if added == 0 {
        return Ok(0);
    }
    write_set(state, &all)?;
    Ok(added)
}

pub fn remove_ignored(state: &Path, paths: &[RelPath]) -> Result<usize, NotifyError> {
    let mut all = read_ignored(state);
    let before = all.len();
    for path in paths {
        all.remove(path);
    }
    let removed = before - all.len();
    if removed == 0 {
        return Ok(0);
    }
    write_set(state, &all)?;
    Ok(removed)
}

fn write_set(state: &Path, all: &BTreeSet<RelPath>) -> Result<(), NotifyError> {
    ensure_state_dir(state)?;
    let dest = ignored_path(state);
    if symlink_at(&dest)? {
        return Err(NotifyError::message("ignored list is a symlink"));
    }
    let temp = state.join(".drift-ignored.tmp");
    if symlink_at(&temp)? {
        return Err(NotifyError::message("ignored list temp path is a symlink"));
    }
    let _ = fs::remove_file(&temp);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|source| NotifyError::io(&temp, source))?;
    let mut text = String::new();
    for path in all {
        text.push_str(path.as_str());
        text.push('\n');
    }
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|source| NotifyError::io(&temp, source))?;
    fs::rename(&temp, &dest).map_err(|source| NotifyError::io(&dest, source))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(text: &str) -> RelPath {
        RelPath::new(text).unwrap()
    }

    #[test]
    fn ignoring_adds_once_persists_sorted_and_removing_undoes_it() {
        let root = drift_fixtures::RootBuilder::new()
            .file("state/keep", b"x\n")
            .unwrap()
            .materialize()
            .unwrap();
        let state = root.path().join("state");
        assert!(read_ignored(&state).is_empty());
        let added = add_ignored(&state, &[rel(".config/b.lua"), rel(".config/a.lua")]).unwrap();
        assert_eq!(added, 2);
        assert_eq!(
            fs::read_to_string(state.join("ignored")).unwrap(),
            ".config/a.lua\n.config/b.lua\n"
        );
        assert_eq!(add_ignored(&state, &[rel(".config/a.lua")]).unwrap(), 0);
        assert_eq!(remove_ignored(&state, &[rel(".config/a.lua")]).unwrap(), 1);
        assert_eq!(remove_ignored(&state, &[rel(".config/zzz")]).unwrap(), 0);
        assert_eq!(
            fs::read_to_string(state.join("ignored")).unwrap(),
            ".config/b.lua\n"
        );
        assert!(!state.join(".drift-ignored.tmp").exists());
    }

    #[test]
    fn a_linked_ignored_list_is_refused_and_its_target_untouched() {
        let root = drift_fixtures::RootBuilder::new()
            .file("secret", b"keep\n")
            .unwrap()
            .link("state/ignored", "../secret")
            .unwrap()
            .materialize()
            .unwrap();
        let state = root.path().join("state");
        let error = add_ignored(&state, &[rel(".config/a.lua")]).unwrap_err();
        assert!(error.to_string().contains("symlink"), "{error}");
        assert_eq!(
            fs::read_to_string(root.path().join("secret")).unwrap(),
            "keep\n"
        );
    }

    #[test]
    fn garbage_lines_in_the_list_are_skipped_when_reading() {
        let root = drift_fixtures::RootBuilder::new()
            .file("state/ignored", b"/abs\n../up\n\n.config/ok.lua\n")
            .unwrap()
            .materialize()
            .unwrap();
        let read = read_ignored(&root.path().join("state"));
        assert_eq!(read.len(), 1);
        assert!(read.contains(&rel(".config/ok.lua")));
    }
}
