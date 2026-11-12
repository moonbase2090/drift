use super::*;

use drift_core::{add_menu_row, hook_content, unified_diff, HOOK_MODE, HOOK_PATH, MENU_PATH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrateMode {
    Confirm,
    Yes,
    DryRun,
}

#[derive(Debug, PartialEq, Eq)]
pub enum IntegrateOutcome {
    AlreadyDone,
    Declined,
    DryRun,
    Applied(ApplyReport),
}

struct Change {
    path: &'static str,
    before: Option<String>,
    after: String,
    mode: Option<u32>,
    from_mode: Option<u32>,
}

pub fn integrate(
    roots: &Roots,
    id: &str,
    time: &str,
    mode: IntegrateMode,
    out: &mut dyn Write,
    ask: &mut dyn FnMut() -> bool,
) -> Result<IntegrateOutcome, String> {
    let changes = gather(roots)?;
    if changes.is_empty() {
        say(out, "Drift is already integrated. Nothing to change.\n")?;
        return Ok(IntegrateOutcome::AlreadyDone);
    }
    show(out, &changes)?;
    if let Some(outcome) = confirm(mode, out, ask)? {
        return Ok(outcome);
    }
    let report = commit(roots, id, time, &changes)?;
    say(
        out,
        &format!(
            "Integrated {} file(s) as transaction {id}.\n",
            report.applied.len()
        ),
    )?;
    Ok(IntegrateOutcome::Applied(report))
}

fn gather(roots: &Roots) -> Result<Vec<Change>, String> {
    let menu = read_text(&roots.live.join(MENU_PATH))?;
    let hook = read_text(&roots.live.join(HOOK_PATH))?;
    let mut changes = Vec::new();
    if let Some(after) = add_menu_row(menu.as_deref()).map_err(|error| error.to_string())? {
        changes.push(Change {
            path: MENU_PATH,
            before: menu,
            after,
            mode: None,
            from_mode: None,
        });
    }
    let hook_mode = existing_mode(&roots.live.join(HOOK_PATH));
    let wanted = hook_content(hook.as_deref()).map_err(|error| error.to_string())?;
    let not_executable = hook_mode.is_some_and(|mode| mode & 0o100 == 0);
    if wanted.is_some() || not_executable {
        changes.push(Change {
            path: HOOK_PATH,
            after: wanted.unwrap_or_else(|| hook.clone().unwrap_or_default()),
            before: hook,
            mode: Some(HOOK_MODE),
            from_mode: hook_mode,
        });
    }
    Ok(changes)
}

fn show(out: &mut dyn Write, changes: &[Change]) -> Result<(), String> {
    for change in changes {
        match (&change.before, change.from_mode) {
            (Some(before), Some(from)) if *before == change.after => {
                let to = change.mode.unwrap_or(from);
                say(
                    out,
                    &format!("{}: mode {from:04o} -> {to:04o}\n", change.path),
                )?;
            }
            _ => {
                say(
                    out,
                    &unified_diff(change.path, change.before.as_deref(), &change.after),
                )?;
                if change.mode.is_some() && change.before.is_none() {
                    say(out, &format!("(new file, mode {HOOK_MODE:04o})\n"))?;
                }
            }
        }
    }
    Ok(())
}

fn confirm(
    mode: IntegrateMode,
    out: &mut dyn Write,
    ask: &mut dyn FnMut() -> bool,
) -> Result<Option<IntegrateOutcome>, String> {
    match mode {
        IntegrateMode::DryRun => {
            say(out, "Dry run: nothing was written.\n")?;
            Ok(Some(IntegrateOutcome::DryRun))
        }
        IntegrateMode::Confirm => {
            say(out, "Apply these changes? [y/N] ")?;
            if ask() {
                return Ok(None);
            }
            say(out, "Nothing was written.\n")?;
            Ok(Some(IntegrateOutcome::Declined))
        }
        IntegrateMode::Yes => Ok(None),
    }
}

fn commit(roots: &Roots, id: &str, time: &str, changes: &[Change]) -> Result<ApplyReport, String> {
    let mut files = Vec::with_capacity(changes.len());
    for change in changes {
        let before = change
            .before
            .as_deref()
            .map(|text| sha256_hex(text.as_bytes()));
        files.push(
            PlannedFile::new(
                change.path,
                Action::TakeUpstream,
                before.as_deref(),
                Some(&sha256_hex(change.after.as_bytes())),
                change.mode,
                None,
            )
            .map_err(|error| error.to_string())?,
        );
    }
    let prune = transactions_to_prune(&roots.state).map_err(|error| error.to_string())?;
    let prune_refs: Vec<&str> = prune.iter().map(String::as_str).collect();
    let plan = Plan::new(id, time, env!("CARGO_PKG_VERSION"), files, &prune_refs)
        .map_err(|error| error.to_string())?;
    let payloads: Vec<Payload<'_>> = changes
        .iter()
        .map(|change| Payload {
            path: change.path,
            bytes: change.after.as_bytes(),
            volatile: false,
            allow_volatile: false,
            allow_symlink: false,
        })
        .collect();
    apply(
        roots,
        &Request {
            plan: &plan,
            files: &payloads,
            fail_at: None,
        },
    )
    .map_err(|error| error.to_string())
}

fn say(out: &mut dyn Write, text: &str) -> Result<(), String> {
    out.write_all(text.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|error| error.to_string())
}

fn read_text(path: &Path) -> Result<Option<String>, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(format!(
            "{} is a symlink; drift will not write through it",
            path.display()
        )),
        Ok(meta) if !meta.is_file() => Err(format!("{} is not a regular file", path.display())),
        Ok(_) => fs::read_to_string(path)
            .map(Some)
            .map_err(|error| format!("{}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}
