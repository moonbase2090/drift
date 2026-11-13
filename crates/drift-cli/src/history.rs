use std::io::{BufRead, Write};
use std::time::Duration;

use drift_apply::{
    history, inspect_undo, undo_with_bases, Restored, Roots, UndoOutcome, UndoRequest, UndoStatus,
};
use drift_core::{HistoryView, UndoKind, UndoLine, UndoPreview};
use drift_source::{read_package, ScanPaths};

use crate::environment::utc_timestamp;
use crate::resolve::Mode;

const HISTORY_USAGE: &str = "usage: drift history [--json]";
const UNDO_USAGE: &str =
    "usage: drift undo [<transaction>] [--force <path>]... [--yes | --dry-run]";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryArgs {
    pub json: bool,
}

pub fn parse_history(args: &[String]) -> Result<HistoryArgs, String> {
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            _ => return Err(format!("unexpected argument '{arg}'; {HISTORY_USAGE}")),
        }
    }
    Ok(HistoryArgs { json })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoArgs {
    pub target: Option<String>,
    pub force: Vec<String>,
    pub mode: Mode,
}

pub fn parse_undo(args: &[String]) -> Result<UndoArgs, String> {
    let mut target = None;
    let mut force = Vec::new();
    let mut mode = Mode::Confirm;
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "--force" => {
                index += 1;
                let Some(path) = args.get(index) else {
                    return Err(format!("--force needs a path; {UNDO_USAGE}"));
                };
                if !force.contains(path) {
                    force.push(path.clone());
                }
            }
            "--yes" | "-y" | "--dry-run" => {
                let next = if arg == "--dry-run" {
                    Mode::DryRun
                } else {
                    Mode::Yes
                };
                if mode != Mode::Confirm && mode != next {
                    return Err(format!(
                        "--yes and --dry-run cannot be combined; {UNDO_USAGE}"
                    ));
                }
                mode = next;
            }
            other if other.starts_with('-') => {
                return Err(format!("unexpected argument '{other}'; {UNDO_USAGE}"));
            }
            other => {
                if target.is_some() {
                    return Err(format!("name one transaction at most; {UNDO_USAGE}"));
                }
                target = Some(other.to_string());
            }
        }
        index += 1;
    }
    Ok(UndoArgs {
        target,
        force,
        mode,
    })
}

pub struct Undoer<'a> {
    pub roots: &'a Roots,
    pub paths: &'a ScanPaths,
}

fn line_of(item: drift_apply::UndoItem) -> UndoLine {
    UndoLine {
        path: item.path,
        kind: match item.status {
            UndoStatus::Restore => UndoKind::Restore,
            UndoStatus::Remove => UndoKind::Remove,
            UndoStatus::AlreadyOriginal => UndoKind::AlreadyOriginal,
            UndoStatus::Kept => UndoKind::Kept,
            UndoStatus::Diverged(found) => UndoKind::Diverged {
                expected: found.expected,
                found: found.found,
            },
        },
    }
}

pub struct Undone {
    pub message: String,
}

impl Undoer<'_> {
    pub fn view(&self) -> Result<HistoryView, String> {
        let found = history(&self.roots.state).map_err(|error| error.to_string())?;
        Ok(HistoryView::build(&found.entries, &found.unreadable))
    }

    pub fn newest(&self) -> Result<String, String> {
        self.view()?
            .rows
            .first()
            .map(|row| row.id.clone())
            .ok_or_else(|| "there are no transactions to undo".to_string())
    }

    pub fn preview(&self, target: &str, elapsed: Duration) -> Result<UndoPreview, String> {
        let found = history(&self.roots.state).map_err(|error| error.to_string())?;
        let plan = found
            .entries
            .iter()
            .find(|plan| plan.id() == target)
            .ok_or_else(|| format!("no readable transaction '{target}'"))?;
        let items = inspect_undo(self.roots, target).map_err(|error| error.to_string())?;
        let package_version = read_package(&self.paths.source.package)
            .map(|package| package.version)
            .unwrap_or_else(|_| plan.package_version().to_string());
        Ok(UndoPreview {
            target: target.to_string(),
            id: format!("undo-{}", elapsed.as_millis()),
            time: utc_timestamp(elapsed.as_secs()),
            package_version,
            lines: items.into_iter().map(line_of).collect(),
        })
    }

    pub fn forced(preview: &UndoPreview, asked: &[String]) -> Result<Vec<String>, String> {
        for path in asked {
            if !preview.lines.iter().any(|line| &line.path == path) {
                return Err(format!(
                    "{path} is not part of transaction {}",
                    preview.target
                ));
            }
        }
        Ok(asked.to_vec())
    }

    pub fn perform(&self, preview: &UndoPreview, forced: &[String]) -> Result<Undone, String> {
        let request = UndoRequest {
            target: &preview.target,
            id: &preview.id,
            time: &preview.time,
            package_version: &preview.package_version,
            restore_anyway: forced,
            fail_at: None,
        };
        let done = undo_with_bases(self.roots, &self.paths.resolve.share, &request)
            .map_err(|error| error.to_string())?;
        match done.outcome {
            UndoOutcome::Restored(report) => Ok(Undone {
                message: restored_message(preview, report.applied.len(), &done.bases),
            }),
            UndoOutcome::Diverged(items) => {
                let names: Vec<String> = items.into_iter().map(|item| item.path).collect();
                Err(format!(
                    "{} changed since {}; nothing was written (--force <path> restores one anyway)",
                    names.join(", "),
                    preview.target
                ))
            }
        }
    }
}

fn restored_message(preview: &UndoPreview, files: usize, bases: &Restored) -> String {
    let mut message = format!(
        "Undid {} as transaction {}: {files} file(s) restored; {} reviewed base(s) restored.",
        preview.target,
        preview.id,
        bases.restored.len()
    );
    if !bases.failed.is_empty() {
        message.push_str(&format!(
            " Could not restore the reviewed base for {}.",
            bases.failed.join(", ")
        ));
    }
    message
}

fn say(writer: &mut dyn Write, text: &str) -> Result<(), String> {
    writer
        .write_all(text.as_bytes())
        .and_then(|()| writer.flush())
        .map_err(|error| error.to_string())
}

pub fn run_history(
    roots: &Roots,
    paths: &ScanPaths,
    args: &HistoryArgs,
    out: &mut dyn Write,
) -> Result<u8, String> {
    let view = Undoer { roots, paths }.view()?;
    if args.json {
        say(out, &format!("{}\n", view.to_json()))?;
    } else {
        say(out, &view.render_text())?;
    }
    Ok(0)
}

#[allow(clippy::too_many_arguments)]
pub fn run_undo(
    roots: &Roots,
    paths: &ScanPaths,
    args: &UndoArgs,
    elapsed: Duration,
    interactive: bool,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<u8, String> {
    let undoer = Undoer { roots, paths };
    let target = match &args.target {
        Some(target) => target.clone(),
        None => undoer.newest()?,
    };
    let preview = undoer.preview(&target, elapsed)?;
    let forced = Undoer::forced(&preview, &args.force)?;
    match args.mode {
        Mode::DryRun => {
            say(out, &preview.render_text(&forced))?;
            return Ok(0);
        }
        Mode::Confirm => {
            say(out, &preview.render_body(&forced))?;
            say(out, "\nUndo this transaction? [y/N] ")?;
            let mut line = String::new();
            let yes = interactive
                && input.read_line(&mut line).is_ok()
                && matches!(line.trim(), "y" | "Y" | "yes");
            if !yes {
                say(out, "Nothing was written.\n")?;
                if !interactive {
                    say(
                        err,
                        "drift: not a terminal, so nothing was written; pass --yes to undo\n",
                    )?;
                }
                return Ok(0);
            }
        }
        Mode::Yes => say(out, &preview.render_body(&forced))?,
    }
    let done = undoer.perform(&preview, &forced)?;
    say(out, &format!("\n{}\n", done.message))?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use drift_fixtures::{hash_tree, MaterializedRoot};

    use super::*;
    use crate::resolve::tests::{clean, go_at, read, scan_paths, transactions, world};
    use crate::resolve::{roots_for, Mode};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|item| item.to_string()).collect()
    }

    struct Out {
        result: Result<u8, String>,
        out: String,
        err: String,
    }

    fn undo_at(
        root: &MaterializedRoot,
        list: &[&str],
        interactive: bool,
        typed: &str,
        millis: u64,
    ) -> Out {
        let paths = scan_paths(root);
        let roots = roots_for(&paths, root.path().join("runtime"));
        let parsed = parse_undo(&args(list)).unwrap();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = run_undo(
            &roots,
            &paths,
            &parsed,
            Duration::from_millis(millis),
            interactive,
            &mut typed.as_bytes(),
            &mut out,
            &mut err,
        );
        Out {
            result,
            out: String::from_utf8(out).unwrap(),
            err: String::from_utf8(err).unwrap(),
        }
    }

    fn listed(root: &MaterializedRoot, json: bool) -> String {
        let paths = scan_paths(root);
        let roots = roots_for(&paths, root.path().join("runtime"));
        let mut out = Vec::new();
        run_history(&roots, &paths, &HistoryArgs { json }, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    const FIRST: u64 = 1_800_000_000_123;
    const SECOND: u64 = 1_800_000_100_456;

    fn resolved() -> MaterializedRoot {
        let root = world(&[
            ("hypr/a.lua", "new a\n", Some("mine a\n"), Some("old a\n")),
            ("hypr/b.lua", "new b\n", Some("mine b\n"), Some("old b\n")),
        ]);
        let done = go_at(
            &root,
            &[".config/hypr/a.lua", "--upstream", "--yes"],
            false,
            "",
            &clean(),
            FIRST,
        );
        assert_eq!(done.result, Ok(0), "{}", done.err);
        assert_eq!(read(&root, "home/.config/hypr/a.lua"), "new a\n");
        root
    }

    fn base(root: &MaterializedRoot, rel: &str) -> String {
        read(root, &format!("home/.local/share/drift/bases/config/{rel}"))
    }

    #[test]
    fn the_command_lines_parse_with_one_transaction_forced_paths_and_one_mode() {
        assert_eq!(
            parse_undo(&args(&[
                "resolve-1",
                "--force",
                ".config/a",
                "--force",
                ".config/a",
                "-y"
            ]))
            .unwrap(),
            UndoArgs {
                target: Some("resolve-1".to_string()),
                force: vec![".config/a".to_string()],
                mode: Mode::Yes,
            }
        );
        assert_eq!(
            parse_undo(&[]).unwrap(),
            UndoArgs {
                target: None,
                force: Vec::new(),
                mode: Mode::Confirm
            }
        );
        for bad in [
            &["a", "b"][..],
            &["--yes", "--dry-run"],
            &["--force"],
            &["--nope"],
        ] {
            assert!(parse_undo(&args(bad)).is_err(), "{bad:?}");
        }
        assert!(parse_history(&args(&["--json"])).unwrap().json);
        assert!(parse_history(&args(&["x"])).is_err());
    }

    #[test]
    fn history_lists_transactions_newest_first_as_text_and_json() {
        let root = resolved();
        let done = go_at(
            &root,
            &[".config/hypr/b.lua", "--keep", "--yes"],
            false,
            "",
            &clean(),
            SECOND,
        );
        assert_eq!(done.result, Ok(0), "{}", done.err);
        let text = listed(&root, false);
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("ID"), "{text}");
        assert!(lines[1].contains("resolve-1800000100456") && lines[1].contains("1 keep mine"));
        assert!(lines[2].contains("resolve-1800000000123") && lines[2].contains("1 take upstream"));
        let json: serde_json::Value = serde_json::from_str(&listed(&root, true)).unwrap();
        assert_eq!(json["rows"][0]["id"], "resolve-1800000100456");
        assert_eq!(json["rows"][1]["id"], "resolve-1800000000123");
    }

    #[test]
    fn an_empty_history_prints_a_line_and_undo_has_nothing_to_do() {
        let root = world(&[("hypr/a.lua", "new\n", Some("mine\n"), Some("old\n"))]);
        assert_eq!(listed(&root, false), "No transactions yet.\n");
        let run = undo_at(&root, &["--yes"], false, "", SECOND);
        assert_eq!(
            run.result,
            Err("there are no transactions to undo".to_string())
        );
    }

    #[test]
    fn a_dry_run_shows_the_restore_and_writes_nothing() {
        let root = resolved();
        let before = hash_tree(root.path()).unwrap();
        let run = undo_at(&root, &["--dry-run"], false, "", SECOND);
        assert_eq!(run.result, Ok(0), "{}", run.err);
        assert!(
            run.out.contains("undo of resolve-1800000000123"),
            "{}",
            run.out
        );
        assert!(
            run.out.contains("restore          .config/hypr/a.lua"),
            "{}",
            run.out
        );
        assert!(run.out.ends_with("Dry run: nothing was written.\n"));
        assert_eq!(hash_tree(root.path()).unwrap(), before);
        assert_eq!(transactions(&root), 1);
    }

    #[test]
    fn undo_restores_the_exact_bytes_the_reviewed_base_and_records_its_own_transaction() {
        let root = resolved();
        assert_eq!(base(&root, "hypr/a.lua"), "new a\n");
        let run = undo_at(&root, &["--yes"], false, "", SECOND);
        assert_eq!(run.result, Ok(0), "{}", run.err);
        assert!(
            run.out.contains(
                "Undid resolve-1800000000123 as transaction undo-1800000100456: 1 file(s) restored; 1 reviewed base(s) restored."
            ),
            "{}",
            run.out
        );
        assert_eq!(read(&root, "home/.config/hypr/a.lua"), "mine a\n");
        assert_eq!(base(&root, "hypr/a.lua"), "old a\n");
        assert_eq!(transactions(&root), 2);
        assert!(listed(&root, false).contains("undo-1800000100456"));
        let again = undo_at(
            &root,
            &["undo-1800000100456", "--yes"],
            false,
            "",
            SECOND + 1,
        );
        assert_eq!(again.result, Ok(0), "{}", again.err);
        assert_eq!(read(&root, "home/.config/hypr/a.lua"), "new a\n");
        assert_eq!(base(&root, "hypr/a.lua"), "new a\n");
    }

    #[test]
    fn a_file_changed_since_is_left_alone_unless_it_is_forced() {
        let root = resolved();
        let live = root.path().join("home/.config/hypr/a.lua");
        fs::write(&live, "edited later\n").unwrap();
        let shown = undo_at(&root, &["--dry-run"], false, "", SECOND);
        assert!(
            shown.out.contains("changed since    .config/hypr/a.lua"),
            "{}",
            shown.out
        );
        assert!(shown
            .out
            .contains("--force .config/hypr/a.lua restores it anyway"));
        let refused = undo_at(&root, &["--yes"], false, "", SECOND);
        let message = refused.result.unwrap_err();
        assert!(
            message.contains(".config/hypr/a.lua changed since"),
            "{message}"
        );
        assert_eq!(fs::read_to_string(&live).unwrap(), "edited later\n");
        assert_eq!(transactions(&root), 1);
        assert_eq!(
            read(&root, "home/.local/share/drift/bases/config/hypr/a.lua"),
            "new a\n"
        );

        let forced = undo_at(
            &root,
            &["--force", ".config/hypr/a.lua", "--yes"],
            false,
            "",
            SECOND,
        );
        assert_eq!(forced.result, Ok(0), "{}", forced.err);
        assert_eq!(fs::read_to_string(&live).unwrap(), "mine a\n");
    }

    #[test]
    fn a_wrong_transaction_or_forced_path_is_refused_before_anything_is_written() {
        let root = resolved();
        let before = hash_tree(root.path()).unwrap();
        let missing = undo_at(&root, &["nope", "--yes"], false, "", SECOND);
        assert_eq!(
            missing.result,
            Err("no readable transaction 'nope'".to_string())
        );
        let stray = undo_at(
            &root,
            &["--force", ".config/other", "--yes"],
            false,
            "",
            SECOND,
        );
        assert_eq!(
            stray.result,
            Err(".config/other is not part of transaction resolve-1800000000123".to_string())
        );
        assert_eq!(hash_tree(root.path()).unwrap(), before);
    }

    #[test]
    fn without_yes_only_an_interactive_yes_applies() {
        let root = resolved();
        let before = hash_tree(root.path()).unwrap();
        let piped = undo_at(&root, &[], false, "y\n", SECOND);
        assert_eq!(piped.result, Ok(0));
        assert!(piped.out.contains("Nothing was written."), "{}", piped.out);
        assert!(piped.err.contains("pass --yes to undo"), "{}", piped.err);
        let declined = undo_at(&root, &[], true, "n\n", SECOND);
        assert!(declined.out.contains("Nothing was written."));
        assert!(declined.err.is_empty());
        assert_eq!(hash_tree(root.path()).unwrap(), before);
        let accepted = undo_at(&root, &[], true, "y\n", SECOND);
        assert_eq!(accepted.result, Ok(0), "{}", accepted.err);
        assert!(accepted.out.contains("Undo this transaction? [y/N] "));
        assert_eq!(read(&root, "home/.config/hypr/a.lua"), "mine a\n");
    }

    #[test]
    fn undoing_a_keep_mine_removes_its_acknowledgement() {
        let root = world(&[("hypr/a.lua", "new\n", Some("mine\n"), Some("old\n"))]);
        let kept = go_at(
            &root,
            &[".config/hypr/a.lua", "--keep", "--yes"],
            false,
            "",
            &clean(),
            FIRST,
        );
        assert_eq!(kept.result, Ok(0), "{}", kept.err);
        assert_eq!(base(&root, "hypr/a.lua"), "new\n");
        let run = undo_at(&root, &["--yes"], false, "", SECOND);
        assert_eq!(run.result, Ok(0), "{}", run.err);
        assert_eq!(base(&root, "hypr/a.lua"), "old\n");
        assert_eq!(read(&root, "home/.config/hypr/a.lua"), "mine\n");
    }
}
