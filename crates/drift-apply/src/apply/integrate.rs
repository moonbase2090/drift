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
}

pub fn integrate(
    roots: &Roots,
    id: &str,
    time: &str,
    mode: IntegrateMode,
    out: &mut dyn Write,
    ask: &mut dyn FnMut() -> bool,
) -> Result<IntegrateOutcome, String> {
    let menu = read_text(&roots.live.join(MENU_PATH))?;
    let hook = read_text(&roots.live.join(HOOK_PATH))?;
    let mut changes = Vec::new();
    if let Some(after) = add_menu_row(menu.as_deref()).map_err(|error| error.to_string())? {
        changes.push(Change {
            path: MENU_PATH,
            before: menu,
            after,
            mode: None,
        });
    }
    if let Some(after) = hook_content(hook.as_deref()).map_err(|error| error.to_string())? {
        changes.push(Change {
            path: HOOK_PATH,
            before: hook,
            after,
            mode: Some(HOOK_MODE),
        });
    }
    if changes.is_empty() {
        say(out, "Drift is already integrated. Nothing to change.\n")?;
        return Ok(IntegrateOutcome::AlreadyDone);
    }
    for change in &changes {
        say(
            out,
            &unified_diff(change.path, change.before.as_deref(), &change.after),
        )?;
        if change.mode.is_some() && change.before.is_none() {
            say(out, &format!("(new file, mode {HOOK_MODE:04o})\n"))?;
        }
    }
    match mode {
        IntegrateMode::DryRun => {
            say(out, "Dry run: nothing was written.\n")?;
            return Ok(IntegrateOutcome::DryRun);
        }
        IntegrateMode::Confirm => {
            say(out, "Apply these changes? [y/N] ")?;
            if !ask() {
                say(out, "Nothing was written.\n")?;
                return Ok(IntegrateOutcome::Declined);
            }
        }
        IntegrateMode::Yes => {}
    }
    let mut files = Vec::with_capacity(changes.len());
    for change in &changes {
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
    let report = apply(
        roots,
        &Request {
            plan: &plan,
            files: &payloads,
            fail_at: None,
        },
    )
    .map_err(|error| error.to_string())?;
    say(
        out,
        &format!(
            "Integrated {} file(s) as transaction {id}.\n",
            report.applied.len()
        ),
    )?;
    Ok(IntegrateOutcome::Applied(report))
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
