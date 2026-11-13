use std::io::{BufRead, Write};
use std::time::Duration;

use drift_apply::{
    history, inspect_undo, pending_prune, recorded_bases, undo_with_bases_expecting, Restored,
    Roots, UndoOutcome, UndoRequest, UndoStatus,
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
            bases: recorded_bases(&self.roots.state, target).map_err(|error| error.to_string())?,
            prune: pending_prune(&self.roots.state).map_err(|error| error.to_string())?,
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
        let done = undo_with_bases_expecting(
            self.roots,
            &self.paths.resolve.share,
            &request,
            &preview.prune,
        )
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
        "Undid {} as {}: {files} file(s) and {} reviewed base(s) restored.",
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

pub struct LiveHistorian {
    paths: ScanPaths,
    roots: Roots,
    clock: fn() -> Duration,
}

pub fn system_clock() -> Duration {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
}

impl LiveHistorian {
    pub fn new(paths: ScanPaths, roots: Roots, clock: fn() -> Duration) -> Self {
        Self {
            paths,
            roots,
            clock,
        }
    }

    fn undoer(&self) -> Undoer<'_> {
        Undoer {
            roots: &self.roots,
            paths: &self.paths,
        }
    }
}

impl drift_tui::Historian for LiveHistorian {
    fn rows(&mut self) -> Result<HistoryView, String> {
        self.undoer().view()
    }

    fn propose(&mut self, target: &str, force: bool) -> Result<drift_tui::UndoProposal, String> {
        let preview = self.undoer().preview(target, (self.clock)())?;
        let forced: Vec<String> = if force {
            preview.diverged().into_iter().map(str::to_string).collect()
        } else {
            Vec::new()
        };
        let state = self.roots.state.display().to_string();
        let view = preview.plan_view(&state, &forced);
        Ok(drift_tui::UndoProposal {
            preview,
            view,
            forced,
        })
    }

    fn apply(&mut self, proposal: &drift_tui::UndoProposal) -> Result<String, String> {
        self.undoer()
            .perform(&proposal.preview, &proposal.forced)
            .map(|done| done.message)
    }
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
                "Undid resolve-1800000000123 as undo-1800000100456: 1 file(s) and 1 reviewed base(s) restored."
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
        root.overwrite("home/.config/hypr/a.lua", "edited later\n")
            .unwrap();
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

    fn at_second() -> Duration {
        Duration::from_millis(SECOND)
    }

    fn manifest(root: &MaterializedRoot, id: &str, file: &str) -> Vec<u8> {
        fs::read(root.path().join(format!("state/backups/{id}/{file}"))).unwrap()
    }

    fn key(code: lunatui_term::KeyCode) -> lunatui_term::Event {
        lunatui_term::Event::Key(lunatui_term::KeyEvent::press(code))
    }

    fn tree_report(root: &MaterializedRoot) -> drift_core::ScanReport {
        crate::scan::build(&scan_paths(root)).unwrap()
    }

    #[test]
    fn the_command_line_and_the_history_screen_write_identical_transactions() {
        use lunatui::App as _;
        use lunatui_term::KeyCode;

        let cli = resolved();
        let ran = undo_at(&cli, &["--yes"], false, "", SECOND);
        assert_eq!(ran.result, Ok(0), "{}", ran.err);

        let tui = resolved();
        let paths = scan_paths(&tui);
        let roots = roots_for(&paths, tui.path().join("runtime"));
        let historian = LiveHistorian::new(paths, roots, at_second);
        let mut screen =
            drift_tui::Screen::with_report(&tree_report(&tui)).with_historian(Box::new(historian));
        let (grid, clusters) = lunatui::render_once(&mut screen, 120, 36);
        let _ = (grid, clusters);
        for code in [KeyCode::Char('H'), KeyCode::Enter, KeyCode::Char('y')] {
            let _ = screen.update(&key(code));
        }
        assert!(
            screen
                .history()
                .and_then(|history| history.notice())
                .is_some_and(|notice| notice
                    .starts_with("Undid resolve-1800000000123 as undo-1800000100456")),
            "{:?}",
            screen.history().and_then(|history| history.notice())
        );

        for file in ["manifest.json", "reviewed-bases.json"] {
            assert_eq!(
                manifest(&cli, "undo-1800000100456", file),
                manifest(&tui, "undo-1800000100456", file),
                "{file}"
            );
        }
        assert_eq!(
            read(&cli, "home/.config/hypr/a.lua").into_bytes(),
            read(&tui, "home/.config/hypr/a.lua").into_bytes()
        );
        assert_eq!(base(&cli, "hypr/a.lua"), base(&tui, "hypr/a.lua"));
        assert_eq!(read(&tui, "home/.config/hypr/a.lua"), "mine a\n");
        assert_eq!(base(&tui, "hypr/a.lua"), "old a\n");
    }

    #[test]
    fn cancelling_or_declining_in_the_history_screen_writes_nothing() {
        use lunatui::App as _;
        use lunatui_term::KeyCode;

        let tui = resolved();
        let before = hash_tree(tui.path()).unwrap();
        let paths = scan_paths(&tui);
        let roots = roots_for(&paths, tui.path().join("runtime"));
        let mut screen = drift_tui::Screen::with_report(&tree_report(&tui))
            .with_historian(Box::new(LiveHistorian::new(paths, roots, at_second)));
        let _ = lunatui::render_once(&mut screen, 120, 36);
        for code in [
            KeyCode::Char('H'),
            KeyCode::Enter,
            KeyCode::Char('n'),
            KeyCode::Enter,
            KeyCode::Enter,
        ] {
            let _ = screen.update(&key(code));
        }
        assert_eq!(hash_tree(tui.path()).unwrap(), before);
        assert_eq!(transactions(&tui), 1);
    }

    #[test]
    fn a_keep_mine_undo_is_previewed_and_applied_from_both_surfaces() {
        use lunatui::App as _;
        use lunatui_term::KeyCode;

        let make = || {
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
            root
        };
        let cli = make();
        let shown = undo_at(&cli, &["--dry-run"], false, "", SECOND);
        assert!(
            shown
                .out
                .contains("reviewed base    config/hypr/a.lua  (put back as it was)"),
            "{}",
            shown.out
        );
        let ran = undo_at(&cli, &["--yes"], false, "", SECOND);
        assert_eq!(ran.result, Ok(0), "{}", ran.err);

        let tui = make();
        let paths = scan_paths(&tui);
        let roots = roots_for(&paths, tui.path().join("runtime"));
        let mut screen = drift_tui::Screen::with_report(&tree_report(&tui))
            .with_historian(Box::new(LiveHistorian::new(paths, roots, at_second)));
        let _ = lunatui::render_once(&mut screen, 120, 36);
        for code in [KeyCode::Char('H'), KeyCode::Enter] {
            let _ = screen.update(&key(code));
        }
        let (grid, clusters) = lunatui::render_once(&mut screen, 120, 36);
        let text = grid.display_text(&clusters);
        assert!(text.contains("reviewed base config/hypr/a.lua"), "{text}");
        assert!(text.contains("put the reviewed base back"), "{text}");
        let _ = screen.update(&key(KeyCode::Char('y')));
        assert_eq!(base(&tui, "hypr/a.lua"), "old\n");
        assert_eq!(read(&tui, "home/.config/hypr/a.lua"), "mine\n");
        for file in ["manifest.json", "reviewed-bases.json"] {
            assert_eq!(
                manifest(&cli, "undo-1800000100456", file),
                manifest(&tui, "undo-1800000100456", file),
                "{file}"
            );
        }
    }

    #[test]
    fn without_a_name_the_newest_transaction_is_the_one_undone() {
        let root = resolved();
        let second = go_at(
            &root,
            &[".config/hypr/b.lua", "--keep", "--yes"],
            false,
            "",
            &clean(),
            SECOND,
        );
        assert_eq!(second.result, Ok(0), "{}", second.err);
        let shown = undo_at(&root, &["--dry-run"], false, "", SECOND + 10);
        assert!(
            shown.out.starts_with("undo of resolve-1800000100456\n"),
            "{}",
            shown.out
        );
        let named = undo_at(
            &root,
            &["resolve-1800000000123", "--dry-run"],
            false,
            "",
            SECOND + 10,
        );
        assert!(
            named.out.starts_with("undo of resolve-1800000000123\n"),
            "{}",
            named.out
        );
    }

    #[test]
    fn the_history_screen_leaves_a_changed_file_alone_until_u_forces_it() {
        use lunatui::App as _;
        use lunatui_term::KeyCode;

        let tui = resolved();
        tui.overwrite("home/.config/hypr/a.lua", "edited later\n")
            .unwrap();
        let paths = scan_paths(&tui);
        let roots = roots_for(&paths, tui.path().join("runtime"));
        let mut screen = drift_tui::Screen::with_report(&tree_report(&tui))
            .with_historian(Box::new(LiveHistorian::new(paths, roots, at_second)));
        let _ = lunatui::render_once(&mut screen, 120, 36);
        for code in [KeyCode::Char('H'), KeyCode::Enter] {
            let _ = screen.update(&key(code));
        }
        let (grid, clusters) = lunatui::render_once(&mut screen, 120, 36);
        let text = grid.display_text(&clusters);
        assert!(!text.contains(".config/hypr/a.lua"), "{text}");
        assert!(
            screen
                .history()
                .and_then(|history| history.notice())
                .is_some_and(|notice| notice.contains("changed since")),
            "{:?}",
            screen.history().and_then(|history| history.notice())
        );
        let _ = screen.update(&key(KeyCode::Char('n')));
        let _ = screen.update(&key(KeyCode::Char('U')));
        let (grid, clusters) = lunatui::render_once(&mut screen, 120, 36);
        let text = grid.display_text(&clusters);
        assert!(text.contains("restored anyway"), "{text}");
        assert_eq!(read(&tui, "home/.config/hypr/a.lua"), "edited later\n");
        let _ = screen.update(&key(KeyCode::Char('y')));
        assert_eq!(read(&tui, "home/.config/hypr/a.lua"), "mine a\n");
    }

    const FULL: usize = 100;

    fn full() -> MaterializedRoot {
        let files: Vec<(String, String, String, String)> = (0..FULL + 1)
            .map(|index| {
                (
                    format!("hypr/f{index:03}.lua"),
                    format!("new {index}\n"),
                    format!("mine {index}\n"),
                    format!("old {index}\n"),
                )
            })
            .collect();
        let borrowed: Vec<(&str, &str, Option<&str>, Option<&str>)> = files
            .iter()
            .map(|(rel, theirs, ours, base)| {
                (
                    rel.as_str(),
                    theirs.as_str(),
                    Some(ours.as_str()),
                    Some(base.as_str()),
                )
            })
            .collect();
        let root = world(&borrowed);
        for index in 0..FULL {
            let target = format!(".config/hypr/f{index:03}.lua");
            let done = go_at(
                &root,
                &[target.as_str(), "--upstream", "--yes"],
                false,
                "",
                &clean(),
                FIRST + index as u64 * 1000,
            );
            assert_eq!(done.result, Ok(0), "{}", done.err);
        }
        assert_eq!(transactions(&root), FULL);
        root
    }

    fn oldest() -> String {
        format!("resolve-{FIRST}")
    }

    #[test]
    fn a_dry_run_names_the_oldest_transaction_the_undo_would_delete() {
        let root = full();
        let before = hash_tree(root.path()).unwrap();
        let run = undo_at(&root, &["--dry-run"], false, "", FIRST + 1_000_000);
        assert_eq!(run.result, Ok(0), "{}", run.err);
        assert!(
            run.out.contains(&format!("\nprune  {}\n", oldest())),
            "{}",
            run.out
        );
        assert_eq!(hash_tree(root.path()).unwrap(), before);
    }

    #[test]
    fn confirming_deletes_exactly_what_the_preview_named_and_no_more() {
        let root = full();
        let run = undo_at(&root, &["--yes"], false, "", FIRST + 1_000_000);
        assert_eq!(run.result, Ok(0), "{}", run.err);
        assert!(
            run.out.contains(&format!("prune  {}", oldest())),
            "{}",
            run.out
        );
        assert_eq!(transactions(&root), FULL);
        let listed = listed(&root, false);
        assert!(!listed.contains(&oldest()), "{listed}");
        assert!(listed.contains("undo-"), "{listed}");
    }

    #[test]
    fn a_history_that_changes_after_the_preview_refuses_the_undo() {
        let root = full();
        let paths = scan_paths(&root);
        let roots = roots_for(&paths, root.path().join("runtime"));
        let undoer = Undoer {
            roots: &roots,
            paths: &paths,
        };
        let target = undoer.newest().unwrap();
        let preview = undoer
            .preview(&target, Duration::from_millis(FIRST + 1_000_000))
            .unwrap();
        assert_eq!(preview.prune, [oldest()]);
        let extra = go_at(
            &root,
            &[".config/hypr/f100.lua", "--upstream", "--yes"],
            false,
            "",
            &clean(),
            FIRST + 2_000_000,
        );
        assert_eq!(extra.result, Ok(0), "{}", extra.err);
        let live = read(&root, "home/.config/hypr/f099.lua");
        let error = undoer.perform(&preview, &[]).err().unwrap();
        assert!(error.contains("changed since the preview"), "{error}");
        assert_eq!(read(&root, "home/.config/hypr/f099.lua"), live);
        assert!(!listed(&root, false).contains("undo-"));
    }

    #[test]
    fn the_history_screen_plan_shows_the_prune_and_the_confirmed_set_is_the_one_deleted() {
        use lunatui::App as _;
        use lunatui_term::KeyCode;

        let tui = full();
        let paths = scan_paths(&tui);
        let roots = roots_for(&paths, tui.path().join("runtime"));
        let mut screen =
            drift_tui::Screen::with_report(&tree_report(&tui)).with_historian(Box::new(
                LiveHistorian::new(paths, roots, || Duration::from_millis(FIRST + 1_000_000)),
            ));
        let _ = lunatui::render_once(&mut screen, 120, 36);
        for code in [KeyCode::Char('H'), KeyCode::Enter] {
            let _ = screen.update(&key(code));
        }
        let (grid, clusters) = lunatui::render_once(&mut screen, 120, 40);
        let text = grid.display_text(&clusters);
        assert!(
            text.contains(&format!(
                "1 old transaction(s) will be removed: {}",
                oldest()
            )),
            "{text}"
        );
        let _ = screen.update(&key(KeyCode::Char('y')));
        assert!(!listed(&tui, false).contains(&oldest()));
        assert!(listed(&tui, false).contains("undo-1800001000123"));
    }
}
