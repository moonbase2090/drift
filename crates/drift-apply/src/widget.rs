use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

pub const RECORD_FILE: &str = "widget-installed.json";

static TEMPS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetMode {
    Confirm,
    Yes,
    DryRun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WidgetOptions {
    pub mode: WidgetMode,
    pub force: bool,
    pub place: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WidgetOutcome {
    Installed,
    Synced,
    UpToDate,
    Removed,
    NotInstalled,
    Refused,
    Declined,
    DryRun,
}

#[derive(Debug, Clone)]
pub struct WidgetPaths {
    pub source: PathBuf,
    pub plugins: PathBuf,
    pub state: PathBuf,
}

pub trait OmarchyShell {
    fn present(&self) -> bool;
    fn run(&mut self, program: &str, args: &[&str]) -> Result<(), String>;
}

pub struct SystemOmarchy;

impl OmarchyShell for SystemOmarchy {
    fn present(&self) -> bool {
        on_path("omarchy") && on_path("omarchy-shell")
    }

    fn run(&mut self, program: &str, args: &[&str]) -> Result<(), String> {
        let output = Command::new(program)
            .args(args)
            .output()
            .map_err(|error| format!("{program}: {error}"))?;
        if output.status.success() {
            return Ok(());
        }
        let text = String::from_utf8_lossy(&output.stderr);
        let text = text.trim();
        Err(if text.is_empty() {
            format!("{program} {} failed", args.join(" "))
        } else {
            text.to_string()
        })
    }
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| {
            let candidate = dir.join(name);
            candidate.is_file()
        })
    })
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Add,
    Update,
    Same,
    Edited,
    Delete,
    Keep,
}

struct Entry {
    rel: String,
    step: Step,
    source: Option<Vec<u8>>,
    dest: Option<Vec<u8>>,
}

fn digest(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn say(out: &mut dyn Write, text: &str) -> Result<(), String> {
    out.write_all(text.as_bytes())
        .map_err(|error| error.to_string())
}

fn read_tree(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut found = BTreeMap::new();
    if root.is_dir() {
        walk(root, root, &mut found)?;
    }
    Ok(found)
}

fn walk(root: &Path, dir: &Path, found: &mut BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        if kind.is_dir() {
            walk(root, &path, found)?;
        } else if kind.is_file() {
            let rel = path
                .strip_prefix(root)
                .map_err(|error| error.to_string())?
                .to_string_lossy()
                .to_string();
            let bytes = fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
            found.insert(rel, bytes);
        }
    }
    Ok(())
}

fn read_record(state: &Path) -> BTreeMap<String, String> {
    let Ok(text) = fs::read_to_string(state.join(RECORD_FILE)) else {
        return BTreeMap::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return BTreeMap::new();
    };
    value
        .get("files")
        .and_then(|files| files.as_object())
        .map(|files| {
            files
                .iter()
                .filter_map(|(rel, hash)| Some((rel.clone(), hash.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn write_record(state: &Path, files: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    let hashes: serde_json::Map<String, serde_json::Value> = files
        .iter()
        .map(|(rel, bytes)| (rel.clone(), serde_json::Value::String(digest(bytes))))
        .collect();
    let text = serde_json::json!({ "files": hashes }).to_string();
    fs::create_dir_all(state).map_err(|error| format!("{}: {error}", state.display()))?;
    write_atomic(&state.join(RECORD_FILE), text.as_bytes())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent", path.display()))?;
    fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let serial = TEMPS.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(".{name}.drift-{}-{serial}", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|error| format!("{}: {error}", temp.display()))?;
    let written = file
        .write_all(bytes)
        .and_then(|()| file.flush())
        .map_err(|error| format!("{}: {error}", temp.display()));
    drop(file);
    let moved = written.and_then(|()| {
        fs::rename(&temp, path).map_err(|error| format!("{}: {error}", path.display()))
    });
    if moved.is_err() {
        let _ = fs::remove_file(&temp);
    }
    moved
}

fn is_link(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

fn refuse_link(dest_dir: &Path, id: &str) -> Result<(), String> {
    if is_link(dest_dir) {
        return Err(format!(
            "{} is a symlink; drift widget will not write through it. Remove the link first (omarchy plugin remove {id} unlinks it)",
            dest_dir.display()
        ));
    }
    Ok(())
}

fn ensure_plain(dest_dir: &Path, rel: &str) -> Result<(), String> {
    let mut path = dest_dir.to_path_buf();
    for part in rel.split('/') {
        path.push(part);
        if is_link(&path) {
            return Err(format!(
                "{} is a symlink; drift widget will not write through it. Move it away or remove it, then run again",
                path.display()
            ));
        }
    }
    Ok(())
}

fn plugin_id(source: &Path) -> Result<String, String> {
    let manifest = source.join("manifest.json");
    let text = fs::read_to_string(&manifest)
        .map_err(|error| format!("{}: {error}", manifest.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| format!("{}: {error}", manifest.display()))?;
    let id = value
        .get("id")
        .and_then(|id| id.as_str())
        .ok_or_else(|| format!("{} has no id", manifest.display()))?;
    let valid = id
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && !id.contains("..");
    if valid {
        Ok(id.to_string())
    } else {
        Err(format!("{} has an invalid id '{id}'", manifest.display()))
    }
}

fn entries(
    source: &BTreeMap<String, Vec<u8>>,
    dest: &BTreeMap<String, Vec<u8>>,
    record: &BTreeMap<String, String>,
) -> Vec<Entry> {
    let mut names: Vec<&String> = source.keys().chain(dest.keys()).collect();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .map(|rel| {
            let new = source.get(rel);
            let old = dest.get(rel);
            let step = match (new, old) {
                (Some(_), None) => Step::Add,
                (Some(new), Some(old)) if new == old => Step::Same,
                (Some(_), Some(old)) => {
                    if record.get(rel).is_some_and(|hash| *hash == digest(old)) {
                        Step::Update
                    } else {
                        Step::Edited
                    }
                }
                (None, Some(old)) => {
                    if record.get(rel).is_some_and(|hash| *hash == digest(old)) {
                        Step::Delete
                    } else {
                        Step::Keep
                    }
                }
                (None, None) => Step::Same,
            };
            Entry {
                rel: rel.clone(),
                step,
                source: new.cloned(),
                dest: old.cloned(),
            }
        })
        .collect()
}

fn show_plan(out: &mut dyn Write, id: &str, plan: &[Entry]) -> Result<(), String> {
    for entry in plan {
        let verb = match entry.step {
            Step::Add => "add",
            Step::Update => "update",
            Step::Same => continue,
            Step::Edited => "EDITED by you, would replace",
            Step::Delete => "remove (no longer shipped)",
            Step::Keep => "keep (not shipped, not installed by drift)",
        };
        say(out, &format!("{verb}  {id}/{}\n", entry.rel))?;
        if entry.step == Step::Edited {
            show_diff(out, entry)?;
        }
    }
    Ok(())
}

fn show_diff(out: &mut dyn Write, entry: &Entry) -> Result<(), String> {
    let (Some(old), Some(new)) = (entry.dest.as_deref(), entry.source.as_deref()) else {
        return Ok(());
    };
    match (std::str::from_utf8(old), std::str::from_utf8(new)) {
        (Ok(old), Ok(new)) => {
            let diff = drift_core::unified_diff(&entry.rel, Some(old), new);
            for line in diff.lines().skip(2) {
                say(out, &format!("    {line}\n"))?;
            }
            Ok(())
        }
        _ => say(out, "    (binary, differs)\n"),
    }
}

fn backup_dir(plugins: &Path, id: &str, stamp: &str) -> PathBuf {
    let base = plugins.join(format!(".{id}.bak.{stamp}"));
    let mut candidate = base.clone();
    let mut n = 1;
    while fs::symlink_metadata(&candidate).is_ok() {
        candidate = PathBuf::from(format!("{}-{n}", base.display()));
        n += 1;
    }
    candidate
}

fn keep_backup(
    plugins: &Path,
    id: &str,
    stamp: &str,
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<PathBuf, String> {
    let backup = backup_dir(plugins, id, stamp);
    for (rel, bytes) in files {
        write_atomic(&backup.join(rel), bytes)?;
    }
    Ok(backup)
}

fn commands(
    omarchy: &mut dyn OmarchyShell,
    out: &mut dyn Write,
    steps: &[Vec<String>],
) -> Result<(), String> {
    for step in steps {
        let (program, args) = step.split_first().ok_or("empty command")?;
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        say(out, &format!("run  {}\n", step.join(" ")))?;
        omarchy.run(program, &args)?;
    }
    Ok(())
}

fn cmd(parts: &[&str]) -> Vec<String> {
    parts.iter().map(ToString::to_string).collect()
}

fn require_omarchy(omarchy: &dyn OmarchyShell, mode: WidgetMode) -> Result<(), String> {
    if mode != WidgetMode::DryRun && !omarchy.present() {
        return Err(
            "Omarchy is not installed here: omarchy and omarchy-shell are not on PATH, so the widget cannot be enabled"
                .to_string(),
        );
    }
    Ok(())
}

pub fn widget_install(
    paths: &WidgetPaths,
    omarchy: &mut dyn OmarchyShell,
    options: WidgetOptions,
    stamp: &str,
    out: &mut dyn Write,
    ask: &mut dyn FnMut() -> bool,
) -> Result<WidgetOutcome, String> {
    refresh(paths, omarchy, options, stamp, out, ask, true)
}

pub fn widget_sync(
    paths: &WidgetPaths,
    omarchy: &mut dyn OmarchyShell,
    options: WidgetOptions,
    stamp: &str,
    out: &mut dyn Write,
    ask: &mut dyn FnMut() -> bool,
) -> Result<WidgetOutcome, String> {
    let id = plugin_id(&paths.source)?;
    if !paths.plugins.join(&id).is_dir() {
        return Err(format!(
            "{id} is not installed; run drift widget install first"
        ));
    }
    refresh(paths, omarchy, options, stamp, out, ask, false)
}

fn refresh_steps(id: &str, enable: bool, place: bool) -> Vec<Vec<String>> {
    let mut steps = vec![cmd(&["omarchy-shell", "shell", "rescanPlugins"])];
    if enable {
        steps.push(cmd(&["omarchy", "plugin", "enable", id]));
    }
    if enable && place {
        steps.push(cmd(&["omarchy", "bar", "put", id]));
    }
    steps
}

fn dry_run(
    out: &mut dyn Write,
    steps: &[Vec<String>],
    what: &str,
) -> Result<WidgetOutcome, String> {
    for step in steps {
        say(out, &format!("would run  {}\n", step.join(" ")))?;
    }
    say(out, &format!("Dry run: nothing was {what}.\n"))?;
    Ok(WidgetOutcome::DryRun)
}

fn guard_edits(
    options: WidgetOptions,
    edited: usize,
    out: &mut dyn Write,
    ask: &mut dyn FnMut() -> bool,
) -> Result<Option<WidgetOutcome>, String> {
    if edited == 0 || options.force {
        return Ok(None);
    }
    say(
        out,
        &format!("{edited} file(s) differ from the shipped copy because you edited them.\n"),
    )?;
    if options.mode == WidgetMode::Confirm && ask() {
        return Ok(None);
    }
    if options.mode == WidgetMode::Yes {
        say(
            out,
            "Not overwriting your edits; pass --force to replace them (a backup is kept).\n",
        )?;
        return Ok(Some(WidgetOutcome::Refused));
    }
    say(out, "Not overwriting your edits. Nothing was written.\n")?;
    Ok(Some(WidgetOutcome::Declined))
}

fn apply_plan(plan: &[Entry], dest_dir: &Path) -> Result<(), String> {
    for entry in plan {
        let path = dest_dir.join(&entry.rel);
        match (&entry.step, &entry.source) {
            (Step::Add | Step::Update | Step::Edited, Some(bytes)) => write_atomic(&path, bytes)?,
            (Step::Delete, _) => {
                fs::remove_file(&path).map_err(|error| format!("{}: {error}", path.display()))?
            }
            _ => {}
        }
    }
    Ok(())
}

fn refresh(
    paths: &WidgetPaths,
    omarchy: &mut dyn OmarchyShell,
    options: WidgetOptions,
    stamp: &str,
    out: &mut dyn Write,
    ask: &mut dyn FnMut() -> bool,
    enable: bool,
) -> Result<WidgetOutcome, String> {
    require_omarchy(omarchy, options.mode)?;
    let id = plugin_id(&paths.source)?;
    let dest_dir = paths.plugins.join(&id);
    let source = read_tree(&paths.source)?;
    let installed = read_tree(&dest_dir)?;
    let was_installed = dest_dir.is_dir();
    refuse_link(&dest_dir, &id)?;
    let plan = entries(&source, &installed, &read_record(&paths.state));
    for entry in &plan {
        if matches!(
            entry.step,
            Step::Add | Step::Update | Step::Edited | Step::Delete
        ) {
            ensure_plain(&dest_dir, &entry.rel)?;
        }
    }
    show_plan(out, &id, &plan)?;
    let edited = plan.iter().filter(|e| e.step == Step::Edited).count();
    let changes = plan.iter().any(|e| {
        matches!(
            e.step,
            Step::Add | Step::Update | Step::Edited | Step::Delete
        )
    });
    let steps = refresh_steps(&id, enable, options.place);
    if options.mode == WidgetMode::DryRun {
        return dry_run(out, &steps, "written");
    }
    if let Some(stopped) = guard_edits(options, edited, out, ask)? {
        return Ok(stopped);
    }
    if !changes && was_installed {
        say(out, "The widget is already up to date.\n")?;
        if enable {
            commands(omarchy, out, &steps)?;
        }
        return Ok(WidgetOutcome::UpToDate);
    }
    if edited > 0 {
        let backup = keep_backup(&paths.plugins, &id, stamp, &installed)?;
        say(
            out,
            &format!("Your edited copy is kept at {}\n", backup.display()),
        )?;
    }
    apply_plan(&plan, &dest_dir)?;
    fs::create_dir_all(&dest_dir).map_err(|error| format!("{}: {error}", dest_dir.display()))?;
    write_record(&paths.state, &source)?;
    say(
        out,
        &format!("Copied the widget to {}\n", dest_dir.display()),
    )?;
    commands(omarchy, out, &steps)?;
    Ok(if was_installed {
        WidgetOutcome::Synced
    } else {
        WidgetOutcome::Installed
    })
}

fn unlink_widget(
    dest_dir: &Path,
    id: &str,
    omarchy: &mut dyn OmarchyShell,
    options: WidgetOptions,
    out: &mut dyn Write,
    ask: &mut dyn FnMut() -> bool,
) -> Result<WidgetOutcome, String> {
    say(
        out,
        &format!(
            "unlink  {} (a symlink; what it points to is left alone)\n",
            dest_dir.display()
        ),
    )?;
    let steps = [
        cmd(&["omarchy", "plugin", "disable", id]),
        cmd(&["omarchy-shell", "shell", "rescanPlugins"]),
    ];
    if options.mode == WidgetMode::DryRun {
        return dry_run(out, &steps, "removed");
    }
    if options.mode == WidgetMode::Confirm && !ask() {
        say(out, "Nothing was removed.\n")?;
        return Ok(WidgetOutcome::Declined);
    }
    if let Err(error) = commands(omarchy, out, &steps[..1]) {
        say(out, &format!("note: {error}\n"))?;
    }
    fs::remove_file(dest_dir).map_err(|error| format!("{}: {error}", dest_dir.display()))?;
    say(out, &format!("Unlinked {id}.\n"))?;
    commands(omarchy, out, &steps[1..])?;
    Ok(WidgetOutcome::Removed)
}

pub fn widget_remove(
    paths: &WidgetPaths,
    omarchy: &mut dyn OmarchyShell,
    options: WidgetOptions,
    stamp: &str,
    out: &mut dyn Write,
    ask: &mut dyn FnMut() -> bool,
) -> Result<WidgetOutcome, String> {
    let id = plugin_id(&paths.source)?;
    let dest_dir = paths.plugins.join(&id);
    if fs::symlink_metadata(&dest_dir).is_err() {
        say(out, &format!("{id} is not installed. Nothing to remove.\n"))?;
        return Ok(WidgetOutcome::NotInstalled);
    }
    require_omarchy(omarchy, options.mode)?;
    if is_link(&dest_dir) {
        return unlink_widget(&dest_dir, &id, omarchy, options, out, ask);
    }
    let installed = read_tree(&dest_dir)?;
    let record = read_record(&paths.state);
    let edited: Vec<&String> = installed
        .iter()
        .filter(|(rel, bytes)| record.get(*rel).is_none_or(|hash| *hash != digest(bytes)))
        .map(|(rel, _)| rel)
        .collect();
    say(out, &format!("remove  {}\n", dest_dir.display()))?;
    if !edited.is_empty() {
        say(
            out,
            &format!(
                "{} file(s) there are not the copy drift installed; a backup is kept.\n",
                edited.len()
            ),
        )?;
    }
    let steps = [
        cmd(&["omarchy", "plugin", "disable", &id]),
        cmd(&["omarchy-shell", "shell", "rescanPlugins"]),
    ];
    if options.mode == WidgetMode::DryRun {
        return dry_run(out, &steps, "removed");
    }
    if options.mode == WidgetMode::Confirm && !ask() {
        say(out, "Nothing was removed.\n")?;
        return Ok(WidgetOutcome::Declined);
    }
    if !edited.is_empty() {
        let backup = keep_backup(&paths.plugins, &id, stamp, &installed)?;
        say(out, &format!("Your copy is kept at {}\n", backup.display()))?;
    }
    if let Err(error) = commands(omarchy, out, &steps[..1]) {
        say(out, &format!("note: {error}\n"))?;
    }
    fs::remove_dir_all(&dest_dir).map_err(|error| format!("{}: {error}", dest_dir.display()))?;
    let record_path = paths.state.join(RECORD_FILE);
    if record_path.exists() {
        fs::remove_file(&record_path)
            .map_err(|error| format!("{}: {error}", record_path.display()))?;
    }
    say(out, &format!("Removed {id}.\n"))?;
    commands(omarchy, out, &steps[1..])?;
    Ok(WidgetOutcome::Removed)
}
