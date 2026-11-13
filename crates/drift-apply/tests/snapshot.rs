use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};

use drift_apply::{snapshot_templates, SnapshotError};
use drift_core::{BaseProvenance, Origin, Registry, RelPath, Root};
use drift_fixtures::RootBuilder;
use drift_source::{resolve_bases, reviewed_version_path, BaseQuery, ResolvePaths};

const ALPHA: &[u8] = b"alpha\n";
const SHELL: &[u8] = b"{\"theme\":\"dark\"}\n";
const NOTES: &[u8] = b"notes\n";
const SECRET: &[u8] = b"secret\n";
const EMPTY: &[u8] = b"";
const DEEP: &[u8] = b"deep\n";

fn rel(path: &str) -> RelPath {
    RelPath::new(path).unwrap()
}

fn registry() -> Registry {
    let notes = Root::new(
        "notes",
        rel("notes"),
        rel(".notes"),
        Vec::new(),
        Origin::User,
    )
    .unwrap();
    Registry::v1().with_roots(vec![notes]).unwrap()
}

fn list_dir(path: &Path) -> Vec<String> {
    let mut names = fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn files_under(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    let mut children = fs::read_dir(dir)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let path = child.path();
        let meta = fs::symlink_metadata(&path).unwrap();
        assert!(
            !meta.file_type().is_symlink(),
            "snapshot contains a link at {}",
            path.display()
        );
        if meta.is_dir() {
            walk(root, &path, out);
        } else if meta.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            let mode = meta.permissions().mode() & 0o777;
            assert_eq!(mode, 0o644, "{rel}");
            out.push((rel, fs::read(&path).unwrap()));
        }
    }
}

#[test]
fn a_fixture_snapshot_matches_bytes_and_stays_inside_the_store() {
    let mut builder = RootBuilder::new();
    builder
        .file("omarchy/config/hypr/a.lua", ALPHA)
        .unwrap()
        .file("omarchy/config/hypr/deep/b.lua", DEEP)
        .unwrap()
        .file("omarchy/config/omarchy/shell.json", SHELL)
        .unwrap()
        .file("omarchy/config/empty", EMPTY)
        .unwrap()
        .file("omarchy/notes/a.txt", NOTES)
        .unwrap()
        .file("omarchy/bin/tool", b"tool\n")
        .unwrap()
        .link("omarchy/config/hypr/out", "../../../secret")
        .unwrap()
        .file("secret", SECRET)
        .unwrap()
        .file("sibling.txt", b"leave me\n")
        .unwrap();
    let tree = builder.materialize().unwrap();
    let omarchy = tree.path().join("omarchy");
    let store = tree.path().join("snapshots");
    let report = snapshot_templates(&registry(), &omarchy, &store, "4.4.0-1").unwrap();
    assert_eq!(report.version, "4.4.0-1");
    assert_eq!(report.files, 5);
    assert_eq!(list_dir(&store), vec!["4.4.0-1".to_string()]);
    assert_eq!(
        files_under(&store.join("4.4.0-1")),
        vec![
            ("config/empty".to_string(), EMPTY.to_vec()),
            ("config/hypr/a.lua".to_string(), ALPHA.to_vec()),
            ("config/hypr/deep/b.lua".to_string(), DEEP.to_vec()),
            ("config/omarchy/shell.json".to_string(), SHELL.to_vec()),
            ("notes/a.txt".to_string(), NOTES.to_vec()),
        ]
    );
    assert_eq!(
        fs::read(tree.path().join("sibling.txt")).unwrap(),
        b"leave me\n"
    );
    assert_eq!(fs::read(tree.path().join("secret")).unwrap(), SECRET);
    assert_eq!(fs::read(omarchy.join("bin/tool")).unwrap(), b"tool\n");
    assert_eq!(fs::read(omarchy.join("config/hypr/a.lua")).unwrap(), ALPHA);

    fs::write(omarchy.join("config/hypr/a.lua"), b"two\n").unwrap();
    let again = snapshot_templates(&registry(), &omarchy, &store, "4.4.0-1").unwrap();
    assert_eq!(again.files, 5);
    assert_eq!(list_dir(&store), vec!["4.4.0-1".to_string()]);
    assert_eq!(
        fs::read(store.join("4.4.0-1/config/hypr/a.lua")).unwrap(),
        b"two\n"
    );

    let state = tree.path().join("state");
    let share = tree.path().join("share");
    fs::create_dir(&state).unwrap();
    fs::create_dir(&share).unwrap();
    fs::write(reviewed_version_path(&state), "4.4.0-1\n").unwrap();
    let paths = ResolvePaths {
        share,
        state,
        snapshots: store,
        cache: tree.path().join("cache"),
        log: tree.path().join("missing.log"),
    };
    let file_rel = rel("hypr/a.lua");
    let template = rel("config/hypr/a.lua");
    let resolved = resolve_bases(
        &paths,
        "9.0.0-1",
        &[BaseQuery {
            root: "config",
            rel: &file_rel,
            template: &template,
        }],
    )
    .unwrap();
    assert_eq!(resolved[0].provenance, BaseProvenance::Snapshot);
    assert_eq!(resolved[0].bytes.as_deref(), Some(b"two\n".as_slice()));
}

#[test]
fn a_bad_version_writes_nothing() {
    let mut builder = RootBuilder::new();
    builder.file("keep", b"keep\n").unwrap();
    let tree = builder.materialize().unwrap();
    for version in ["", ".", "..", "a/b", "a b", "4.4.0-1/evil", "/abs"] {
        let store = tree.path().join(format!("store-{version}"));
        let error = snapshot_templates(&Registry::v1(), tree.path(), &store, version).unwrap_err();
        assert!(
            matches!(error, SnapshotError::Version(_)),
            "{version}: {error}"
        );
        assert!(!store.exists(), "{version}");
    }
    assert_eq!(fs::read(tree.path().join("keep")).unwrap(), b"keep\n");
}

#[test]
fn a_failed_copy_publishes_nothing_outside_or_inside_the_store() {
    let mut builder = RootBuilder::new();
    builder
        .file_with_mode("omarchy/config/nope.lua", b"hidden\n", 0)
        .unwrap()
        .file("omarchy/config/ok.lua", b"ok\n")
        .unwrap()
        .file("sibling.txt", b"stay\n")
        .unwrap()
        .file("snapshots/.keep", b"keep\n")
        .unwrap();
    let tree = builder.materialize().unwrap();
    let store = tree.path().join("snapshots");
    let error = snapshot_templates(
        &Registry::v1(),
        &tree.path().join("omarchy"),
        &store,
        "1.2.3-1",
    )
    .unwrap_err();
    assert!(error.to_string().contains("nope.lua"), "{error}");
    assert_eq!(list_dir(&store), vec![".keep".to_string()]);
    assert_eq!(fs::read(store.join(".keep")).unwrap(), b"keep\n");
    assert_eq!(
        fs::read(tree.path().join("sibling.txt")).unwrap(),
        b"stay\n"
    );
    assert_eq!(
        fs::read(tree.path().join("omarchy/config/ok.lua")).unwrap(),
        b"ok\n"
    );
}

#[test]
fn a_linked_template_root_is_not_followed() {
    let mut builder = RootBuilder::new();
    builder
        .file("secret", SECRET)
        .unwrap()
        .file("sibling.txt", b"stay\n")
        .unwrap();
    let tree = builder.materialize().unwrap();
    let omarchy = tree.path().join("omarchy");
    fs::create_dir(&omarchy).unwrap();
    symlink(tree.path().join("secret"), omarchy.join("config")).unwrap();
    let store = tree.path().join("snapshots");
    let error = snapshot_templates(&Registry::v1(), &omarchy, &store, "1.0.0-1").unwrap_err();
    assert!(matches!(error, SnapshotError::Escape(_)), "{error}");
    assert!(
        !store.exists() || list_dir(&store).is_empty(),
        "{}",
        store.display()
    );
    assert_eq!(fs::read(tree.path().join("secret")).unwrap(), SECRET);
    assert_eq!(
        fs::read(tree.path().join("sibling.txt")).unwrap(),
        b"stay\n"
    );
}

#[test]
fn scratch_names_are_copied_beside_ordinary_files() {
    let mut builder = RootBuilder::new();
    builder
        .file("omarchy/config/.drift-snapshot.tmp", b"keep me")
        .unwrap()
        .file("omarchy/config/.drift-snapshot.alt.tmp", b"keep alt")
        .unwrap()
        .file("omarchy/config/z.conf", b"normal")
        .unwrap();
    let tree = builder.materialize().unwrap();
    let store = tree.path().join("snapshots");
    let report = snapshot_templates(
        &Registry::v1(),
        &tree.path().join("omarchy"),
        &store,
        "1.0.0-1",
    )
    .unwrap();
    assert_eq!(report.files, 3);
    assert_eq!(list_dir(&store), vec!["1.0.0-1".to_string()]);
    assert_eq!(
        files_under(&store.join("1.0.0-1")),
        vec![
            (
                "config/.drift-snapshot.alt.tmp".to_string(),
                b"keep alt".to_vec()
            ),
            (
                "config/.drift-snapshot.tmp".to_string(),
                b"keep me".to_vec()
            ),
            ("config/z.conf".to_string(), b"normal".to_vec()),
        ]
    );
}

#[test]
fn an_intermediate_source_link_publishes_nothing() {
    let mut builder = RootBuilder::new();
    builder
        .file("outside/nested/secret.conf", SECRET)
        .unwrap()
        .file("real-store/.keep", b"keep\n")
        .unwrap();
    let tree = builder.materialize().unwrap();
    let source = tree.path().join("source");
    fs::create_dir(&source).unwrap();
    symlink(tree.path().join("outside"), source.join("vendor")).unwrap();
    let vendor = Root::new(
        "vendor",
        rel("vendor/nested"),
        rel(".local/share/vendor"),
        Vec::new(),
        Origin::User,
    )
    .unwrap();
    let registry = Registry::v1().with_roots(vec![vendor]).unwrap();
    let store = tree.path().join("real-store");
    let error = snapshot_templates(&registry, &source, &store, "1.0.0-1").unwrap_err();
    assert!(matches!(error, SnapshotError::Escape(_)), "{error}");
    assert!(error.to_string().contains("vendor"), "{error}");
    assert_eq!(list_dir(&store), vec![".keep".to_string()]);
    assert!(!store.join("1.0.0-1").exists());
    assert_eq!(fs::read(store.join(".keep")).unwrap(), b"keep\n");
    assert_eq!(
        fs::read(tree.path().join("outside/nested/secret.conf")).unwrap(),
        SECRET
    );
}

#[test]
fn a_store_ancestor_link_publishes_nothing() {
    let mut builder = RootBuilder::new();
    builder
        .file("omarchy/config/z.conf", b"normal")
        .unwrap()
        .file("real/snapshots/nested/already.txt", b"already\n")
        .unwrap();
    let tree = builder.materialize().unwrap();
    symlink(tree.path().join("real"), tree.path().join("alias")).unwrap();
    let store = tree.path().join("alias").join("snapshots");
    let error = snapshot_templates(
        &Registry::v1(),
        &tree.path().join("omarchy"),
        &store,
        "1.0.0-1",
    )
    .unwrap_err();
    assert!(matches!(error, SnapshotError::Escape(_)), "{error}");
    assert!(error.to_string().contains("alias"), "{error}");
    let real = tree.path().join("real").join("snapshots");
    assert_eq!(list_dir(&real), vec!["nested".to_string()]);
    assert!(!real.join("1.0.0-1").exists());
    assert!(!real.join(".partial-1.0.0-1").exists());
    assert_eq!(
        fs::read(real.join("nested/already.txt")).unwrap(),
        b"already\n"
    );
    assert_eq!(
        fs::read(tree.path().join("omarchy/config/z.conf")).unwrap(),
        b"normal"
    );
}

#[test]
fn an_empty_root_still_records_the_version_directory() {
    let mut builder = RootBuilder::new();
    builder.file("omarchy/.keep", b"o\n").unwrap();
    let tree = builder.materialize().unwrap();
    let store = tree.path().join("snapshots");
    let report = snapshot_templates(
        &Registry::v1(),
        &tree.path().join("omarchy"),
        &store,
        "1.0.0-1",
    )
    .unwrap();
    assert_eq!(report.files, 0);
    assert_eq!(list_dir(&store), vec!["1.0.0-1".to_string()]);
    assert!(list_dir(&store.join("1.0.0-1")).is_empty());
}

#[derive(Debug)]
struct Hook {
    triggers: Vec<Trigger>,
    description: Option<String>,
    when: String,
    exec: String,
    depends: Vec<String>,
    abort_on_fail: bool,
    needs_targets: bool,
}

#[derive(Debug, Default)]
struct Trigger {
    operations: Vec<String>,
    kind: Option<String>,
    targets: Vec<String>,
}

#[derive(Debug, Default)]
struct ActionDraft {
    description: Option<String>,
    when: Option<String>,
    exec: Option<String>,
    depends: Vec<String>,
    abort_on_fail: bool,
    needs_targets: bool,
}

fn parse_hook(text: &str) -> Result<Hook, String> {
    let mut triggers = Vec::new();
    let mut current: Option<Trigger> = None;
    let mut action: Option<ActionDraft> = None;
    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = raw.trim().trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            if let Some(trigger) = current.take() {
                triggers.push(finish_trigger(trigger)?);
            }
            match name {
                "Trigger" => {
                    if action.is_some() {
                        return Err(format!("line {line_no}: trigger after action"));
                    }
                    current = Some(Trigger::default());
                }
                "Action" => {
                    if action.is_some() {
                        return Err(format!("line {line_no}: repeated action"));
                    }
                    action = Some(ActionDraft::default());
                }
                _ => return Err(format!("line {line_no}: unknown section {name}")),
            }
            continue;
        }
        let (key, value) = split_field(line)?;
        if let Some(trigger) = current.as_mut() {
            match (key, value) {
                ("Operation", Some(value)) if matches!(value, "Install" | "Upgrade" | "Remove") => {
                    trigger.operations.push(value.to_string());
                }
                ("Type", Some(value)) if matches!(value, "Package" | "Path") => {
                    if trigger.kind.is_some() {
                        return Err(format!("line {line_no}: repeated Type"));
                    }
                    trigger.kind = Some(value.to_string());
                }
                ("Target", Some(value)) if !value.is_empty() => {
                    trigger.targets.push(value.to_string());
                }
                _ => return Err(format!("line {line_no}: invalid trigger field")),
            }
        } else if let Some(action) = action.as_mut() {
            match (key, value) {
                ("Description", Some(value))
                    if !value.is_empty() && action.description.is_none() =>
                {
                    action.description = Some(value.to_string());
                }
                ("When", Some(value))
                    if matches!(value, "PreTransaction" | "PostTransaction")
                        && action.when.is_none() =>
                {
                    action.when = Some(value.to_string());
                }
                ("Exec", Some(value)) if !value.is_empty() && action.exec.is_none() => {
                    action.exec = Some(value.to_string());
                }
                ("Depends", Some(value)) if !value.is_empty() => {
                    action.depends.push(value.to_string());
                }
                ("AbortOnFail", _) => action.abort_on_fail = true,
                ("NeedsTargets", _) => action.needs_targets = true,
                _ => return Err(format!("line {line_no}: invalid action field")),
            }
        } else {
            return Err(format!("line {line_no}: field outside a section"));
        }
    }
    if let Some(trigger) = current {
        triggers.push(finish_trigger(trigger)?);
    }
    if triggers.is_empty() {
        return Err("hook needs a trigger".to_string());
    }
    let Some(action) = action else {
        return Err("hook needs an action".to_string());
    };
    let (Some(when), Some(exec)) = (action.when, action.exec) else {
        return Err("action needs When and Exec".to_string());
    };
    Ok(Hook {
        triggers,
        description: action.description,
        when,
        exec,
        depends: action.depends,
        abort_on_fail: action.abort_on_fail,
        needs_targets: action.needs_targets,
    })
}

fn finish_trigger(trigger: Trigger) -> Result<Trigger, String> {
    if trigger.operations.is_empty() || trigger.kind.is_none() || trigger.targets.is_empty() {
        return Err("trigger needs Operation, Type and Target".to_string());
    }
    Ok(trigger)
}

fn split_field(line: &str) -> Result<(&str, Option<&str>), String> {
    match line.split_once('=') {
        Some((key, value)) => {
            let key = key.trim();
            let value = value.trim();
            if key.is_empty() || value.is_empty() {
                Err(format!("invalid field {line:?}"))
            } else {
                Ok((key, Some(value)))
            }
        }
        None => {
            let key = line.trim();
            if key.is_empty() {
                Err("empty field".to_string())
            } else {
                Ok((key, None))
            }
        }
    }
}

fn hook_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packaging/10-drift-snapshot.hook")
}

#[test]
fn the_pacman_hook_matches_alpm_syntax_and_does_not_abort() {
    let path = hook_path();
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some("10-drift-snapshot.hook")
    );
    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains("AbortOnFail"), "{text}");
    assert!(!text.contains("NeedsTargets"), "{text}");
    let hook = parse_hook(&text).unwrap();
    assert_eq!(hook.triggers.len(), 1);
    assert_eq!(hook.triggers[0].operations, vec!["Upgrade".to_string()]);
    assert_eq!(hook.triggers[0].kind.as_deref(), Some("Package"));
    assert_eq!(
        hook.triggers[0].targets,
        vec!["omarchy-settings".to_string()]
    );
    assert_eq!(hook.when, "PreTransaction");
    assert_eq!(hook.exec, "/usr/bin/drift snapshot --pre");
    assert_eq!(
        hook.exec.split_whitespace().collect::<Vec<_>>(),
        vec!["/usr/bin/drift", "snapshot", "--pre"]
    );
    assert!(hook.description.is_some());
    assert!(hook.depends.is_empty());
    assert!(!hook.abort_on_fail);
    assert!(!hook.needs_targets);
    let aborting = "\
[Trigger]
Operation = Upgrade
Type = Package
Target = omarchy-settings

[Action]
When = PreTransaction
Exec = /usr/bin/drift snapshot --pre
AbortOnFail
";
    let parsed = parse_hook(aborting).unwrap();
    assert!(parsed.abort_on_fail);
    assert!(parse_hook("[Trigger]\nOperation = Upgrade\n").is_err());
    assert!(parse_hook("Exec = /bin/true\n").is_err());
}

#[test]
fn the_post_transaction_hook_marks_the_upgrade_and_does_not_abort() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packaging/20-drift-updated.hook");
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some("20-drift-updated.hook")
    );
    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains("AbortOnFail"), "{text}");
    assert!(!text.contains("NeedsTargets"), "{text}");
    let hook = parse_hook(&text).unwrap();
    assert_eq!(hook.triggers.len(), 1);
    assert_eq!(hook.triggers[0].operations, vec!["Upgrade".to_string()]);
    assert_eq!(hook.triggers[0].kind.as_deref(), Some("Package"));
    assert_eq!(
        hook.triggers[0].targets,
        vec!["omarchy-settings".to_string()]
    );
    assert_eq!(hook.when, "PostTransaction");
    assert_eq!(hook.exec, "/usr/bin/drift-mark-updated");
    assert!(hook.description.is_some());
    assert!(hook.depends.is_empty());
    assert!(!hook.abort_on_fail);
    assert!(!hook.needs_targets);
}

#[test]
fn messages_name_a_rejected_version() {
    let error = SnapshotError::Version("a/b".to_string());
    assert!(error.to_string().contains("a/b"));
}
