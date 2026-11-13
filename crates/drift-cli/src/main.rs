mod diff;
mod environment;
mod history;
mod notify;
mod resolve;
mod scan;
mod snapshot;
mod watch;

use std::io::IsTerminal;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use drift_apply::IntegrateMode;

const USAGE: &str = "\
Usage: drift [--snapshot [--width N] [--height N]] [--version] [--help]
       drift integrate [--yes | --dry-run]
       drift diff <path> [--json]
       drift resolve <path>... --keep | --upstream | --merge [--yes | --dry-run]
       drift history [--json]
       drift undo [<transaction>] [--force <path>]... [--yes | --dry-run]
       drift scan
       drift snapshot --pre
       drift status [--json | --bar]
       drift notify [on | off]

With no arguments, drift opens the review screen.
--snapshot prints one frame and does not take the terminal.
integrate shows the changes it would make to the Omarchy menu and hook files and
asks before writing. --yes skips the question; --dry-run only shows the diff.
diff prints base, ours, theirs, hunks and the automatic result for one live path.
resolve shows a plan for keep mine, take upstream or a clean merge of the given files and
asks before writing; all files go in one transaction. --yes skips the question.
history lists the transactions drift has written, newest first. undo shows what undoing
a transaction (the newest by default) would restore or remove, and asks before writing;
files changed since are left alone unless named with --force. --yes skips the question.
scan lists every tracked file with its status. status prints a summary, as JSON with
--json, or as a status-bar module with --bar. snapshot --pre copies installed template
roots into /var/lib/drift/snapshots/<version> before a package upgrade and writes only
under that store. notify sends a normal-urgency notification when a scan finds drift.
The text is 1 config drifted, or N configs drifted, and clicking it opens Drift.
Nothing is sent when there is no drift, while another process holds the Omarchy
update lock, while the pacman database lock exists, or after drift notify off.
The post-update hook inherits the Omarchy lock and still sends, and it still
checks the pacman lock. An Omarchy lock file left behind does not block a later
notify. Pacman removes its database lock when the transaction ends.
Notifications start on. drift notify on turns them back on.
notify exits 0 when it sends or skips, and 2 when the scan or the notifier fails.
Exit codes for the other commands: 0 no drift, 1 drift, 2 error.
";

enum Action {
    Text(String),
    Snapshot { width: u16, height: u16 },
    Launch,
    Integrate(IntegrateMode),
    Scan(scan::View),
    Diff(diff::DiffArgs),
    Resolve(resolve::ResolveArgs),
    History(history::HistoryArgs),
    Undo(history::UndoArgs),
    PreSnapshot,
    Notify(notify::Request),
}

fn prefixed<T>(result: Result<T, String>) -> Result<T, String> {
    result.map_err(|error| format!("drift: {error}"))
}

fn subcommand(args: &[String]) -> Option<Result<Action, String>> {
    let rest = args.get(1..).unwrap_or_default();
    match args.first().map(String::as_str)? {
        "diff" => Some(prefixed(diff::parse(rest)).map(Action::Diff)),
        "resolve" => Some(prefixed(resolve::parse(rest)).map(Action::Resolve)),
        "history" => Some(prefixed(history::parse_history(rest)).map(Action::History)),
        "undo" => Some(prefixed(history::parse_undo(rest)).map(Action::Undo)),
        "scan" if rest.is_empty() => Some(Ok(Action::Scan(scan::View::Scan))),
        "scan" => Some(Err(format!("drift: scan takes no arguments\n{USAGE}"))),
        "status" => Some(
            scan::view_from(rest)
                .map(Action::Scan)
                .map_err(|message| format!("drift: {message}\n{USAGE}")),
        ),
        "integrate" => Some(integrate_mode(rest).map(Action::Integrate)),
        "snapshot" => Some(prefixed(snapshot::parse(rest)).map(|()| Action::PreSnapshot)),
        "notify" => Some(prefixed(notify::parse(rest)).map(Action::Notify)),
        _ => None,
    }
}

fn run(args: &[String]) -> Result<Action, String> {
    if let Some(found) = subcommand(args) {
        return found;
    }
    let mut snapshot = false;
    let mut width = None;
    let mut height = None;
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "-h" | "--help" => return Ok(Action::Text(USAGE.to_string())),
            "-V" | "--version" => {
                return Ok(Action::Text(format!("drift {}", env!("CARGO_PKG_VERSION"))));
            }
            "--snapshot" => snapshot = true,
            "--width" => {
                index += 1;
                width = Some(number(args.get(index).map(String::as_str), "--width")?);
            }
            "--height" => {
                index += 1;
                height = Some(number(args.get(index).map(String::as_str), "--height")?);
            }
            other => return Err(format!("drift: unknown argument '{other}'\n{USAGE}")),
        }
        index += 1;
    }
    match (snapshot, width, height) {
        (false, None, None) => Ok(Action::Launch),
        (true, None, None) => Ok(Action::Snapshot {
            width: 80,
            height: 24,
        }),
        (true, Some(width), Some(height)) => Ok(Action::Snapshot { width, height }),
        (false, _, _) => Err(format!(
            "drift: --width and --height belong with --snapshot\n{USAGE}"
        )),
        (true, _, _) => Err(format!(
            "drift: --snapshot needs both --width and --height, or neither\n{USAGE}"
        )),
    }
}

fn integrate_mode(args: &[String]) -> Result<IntegrateMode, String> {
    let mut mode = IntegrateMode::Confirm;
    for arg in args {
        let next = match arg.as_str() {
            "--yes" | "-y" => IntegrateMode::Yes,
            "--dry-run" => IntegrateMode::DryRun,
            other => return Err(format!("drift: unknown argument '{other}'\n{USAGE}")),
        };
        if mode != IntegrateMode::Confirm && mode != next {
            return Err(format!(
                "drift: --yes and --dry-run cannot be combined\n{USAGE}"
            ));
        }
        mode = next;
    }
    Ok(mode)
}

fn integrate_now(mode: IntegrateMode) -> Result<(), String> {
    let roots = environment::roots_from_env()?;
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    let interactive = std::io::stdin().is_terminal();
    environment::run_integrate(
        &roots,
        elapsed,
        mode,
        interactive,
        &mut std::io::stdin().lock(),
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    )
}

fn number(value: Option<&str>, flag: &str) -> Result<u16, String> {
    let Some(value) = value else {
        return Err(format!("drift: {flag} needs a number\n{USAGE}"));
    };
    let parsed = value
        .parse::<u16>()
        .map_err(|_| format!("drift: {flag} needs a number\n{USAGE}"))?;
    if parsed == 0 {
        return Err(format!("drift: {flag} must be at least 1\n{USAGE}"));
    }
    Ok(parsed)
}

fn live_resolver(registry: &drift_core::Registry) -> resolve::LiveResolver {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map_or_else(
            || std::path::PathBuf::from("/run"),
            std::path::PathBuf::from,
        );
    let fresh = scan::paths_from_env();
    let roots = resolve::roots_for(&fresh, runtime);
    resolve::LiveResolver::new(
        fresh,
        roots,
        Box::new(drift_apply::Hyprctl::new()),
        registry.clone(),
    )
}

fn live_historian() -> history::LiveHistorian {
    let (paths, roots) = state_roots();
    history::LiveHistorian::new(paths, roots, history::system_clock)
}

fn open_review() -> std::io::Result<()> {
    if !std::io::stdout().is_terminal() {
        return drift_tui::launch();
    }
    let paths = scan::paths_from_env();
    let registry = scan::load_registry(&paths)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let watched = watch::dirs(
        &registry,
        &paths.source.home,
        &paths.source.package.omarchy_path,
    );
    let resolver = live_resolver(&registry);
    let differ = diff::LiveDiffer::new(scan::paths_from_env(), registry.clone());
    let worker = drift_tui::spawn_scan(move || scan::scan_report(&paths, &registry));
    let watcher = watch::arm(&watched, worker.changes());
    let historian = live_historian();
    let result = drift_tui::launch_worker_with(
        worker,
        Box::new(resolver),
        Box::new(historian),
        Box::new(differ),
    );
    drop(watcher);
    result
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    execute(run(&args))
}

fn resolve_now(args: &resolve::ResolveArgs) -> Result<u8, String> {
    let paths = scan::paths_from_env();
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map_or_else(
            || std::path::PathBuf::from("/run"),
            std::path::PathBuf::from,
        );
    let roots = resolve::roots_for(&paths, runtime);
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    let env = resolve::Environment {
        paths: &paths,
        roots: &roots,
        state_display: &roots.state.display().to_string(),
        checker: &drift_apply::Hyprctl::new(),
    };
    resolve::run(
        &env,
        args,
        elapsed,
        std::io::stdin().is_terminal(),
        &mut std::io::stdin().lock(),
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    )
}

fn state_roots() -> (drift_source::ScanPaths, drift_apply::Roots) {
    let paths = scan::paths_from_env();
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map_or_else(
            || std::path::PathBuf::from("/run"),
            std::path::PathBuf::from,
        );
    let roots = resolve::roots_for(&paths, runtime);
    (paths, roots)
}

fn notify_now(request: notify::Request) -> Result<u8, String> {
    let (paths, roots) = state_roots();
    notify::run(
        &paths,
        &roots.state,
        &roots.omarchy_lock,
        &roots.pacman_lock,
        std::path::Path::new("omarchy-notification-send"),
        request,
        &mut std::io::stdout(),
    )
}

fn history_now(args: &history::HistoryArgs) -> Result<u8, String> {
    let (paths, roots) = state_roots();
    history::run_history(&roots, &paths, args, &mut std::io::stdout())
}

fn undo_now(args: &history::UndoArgs) -> Result<u8, String> {
    let (paths, roots) = state_roots();
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    history::run_undo(
        &roots,
        &paths,
        args,
        elapsed,
        std::io::stdin().is_terminal(),
        &mut std::io::stdin().lock(),
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    )
}

fn finish(result: Result<u8, String>) -> ExitCode {
    match result {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("drift: {error}");
            ExitCode::from(2)
        }
    }
}

fn execute(action: Result<Action, String>) -> ExitCode {
    match action {
        Ok(Action::Text(text)) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Ok(Action::Snapshot { width, height }) => {
            println!("{}", drift_tui::frame_text(width, height));
            ExitCode::SUCCESS
        }
        Ok(Action::Integrate(mode)) => finish(integrate_now(mode).map(|()| 0)),
        Ok(Action::Resolve(args)) => finish(resolve_now(&args)),
        Ok(Action::History(args)) => finish(history_now(&args)),
        Ok(Action::Undo(args)) => finish(undo_now(&args)),
        Ok(Action::Diff(args)) => finish(diff::run(
            &scan::paths_from_env(),
            &args,
            &mut std::io::stdout(),
        )),
        Ok(Action::Scan(view)) => finish(scan::run(
            &scan::paths_from_env(),
            view,
            &mut std::io::stdout(),
        )),
        Ok(Action::PreSnapshot) => finish(snapshot::run().map(|()| 0)),
        Ok(Action::Notify(request)) => finish(notify_now(request)),
        Ok(Action::Launch) => match open_review() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("drift: {error}");
                ExitCode::from(1)
            }
        },
        Err(text) => {
            eprintln!("{text}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::IsTerminal;

    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn version_flag_prints_version() {
        let Action::Text(out) = run(&args(&["--version"])).unwrap() else {
            panic!("version is text");
        };
        assert_eq!(out, format!("drift {}", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn no_arguments_launches_the_screen() {
        assert!(matches!(run(&[]).unwrap(), Action::Launch));
    }

    #[test]
    fn unknown_argument_is_an_error() {
        assert!(run(&args(&["bogus"])).is_err());
    }

    #[test]
    fn help_and_snapshot_exit_success() {
        assert_eq!(execute(run(&args(&["--help"]))), ExitCode::SUCCESS);
        assert_eq!(execute(run(&args(&["--snapshot"]))), ExitCode::SUCCESS);
    }

    #[test]
    fn unknown_argument_exits_two() {
        assert_eq!(execute(run(&args(&["bogus"]))), ExitCode::from(2));
    }

    #[test]
    fn launch_without_a_terminal_exits_one() {
        if std::io::stdout().is_terminal() {
            return;
        }
        assert_eq!(execute(Ok(Action::Launch)), ExitCode::from(1));
    }

    #[test]
    fn snapshot_defaults_to_eighty_by_twenty_four() {
        let Action::Snapshot { width, height } = run(&args(&["--snapshot"])).unwrap() else {
            panic!("snapshot");
        };
        assert_eq!((width, height), (80, 24));
    }

    #[test]
    fn snapshot_frame_names_drift_and_quit() {
        let text = drift_tui::frame_text(80, 24);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 24);
        assert!(lines[0].contains("Drift"));
        assert!(lines[1].trim().is_empty());
        assert!(lines[23].contains("[Q] Quit"));
    }

    #[test]
    fn integrate_takes_yes_or_dry_run_but_not_both() {
        let mode = |list: &[&str]| match run(&args(list)).unwrap() {
            Action::Integrate(mode) => mode,
            _ => panic!("integrate"),
        };
        assert_eq!(mode(&["integrate"]), IntegrateMode::Confirm);
        assert_eq!(mode(&["integrate", "--yes"]), IntegrateMode::Yes);
        assert_eq!(mode(&["integrate", "-y", "--yes"]), IntegrateMode::Yes);
        assert_eq!(mode(&["integrate", "--dry-run"]), IntegrateMode::DryRun);
        assert!(run(&args(&["integrate", "--yes", "--dry-run"])).is_err());
        assert!(run(&args(&["integrate", "--dry-run", "--yes"])).is_err());
        assert!(run(&args(&["integrate", "--bogus"])).is_err());
        assert_eq!(
            execute(run(&args(&["integrate", "--bogus"]))),
            ExitCode::from(2)
        );
    }

    #[test]
    fn scan_and_status_are_subcommands() {
        let view = |list: &[&str]| match run(&args(list)).unwrap() {
            Action::Scan(view) => view,
            _ => panic!("scan"),
        };
        assert_eq!(view(&["scan"]), scan::View::Scan);
        assert_eq!(view(&["status"]), scan::View::Status);
        assert_eq!(view(&["status", "--json"]), scan::View::Json);
        assert_eq!(view(&["status", "--bar"]), scan::View::Bar);
        assert!(run(&args(&["scan", "--json"])).is_err());
        assert!(run(&args(&["status", "--json", "--bar"])).is_err());
        assert!(run(&args(&["status", "extra"])).is_err());
        assert_eq!(
            execute(run(&args(&["status", "--nope"]))),
            ExitCode::from(2)
        );
    }

    #[test]
    fn notify_defaults_to_sending_and_accepts_the_toggle() {
        let request = |list: &[&str]| match run(&args(list)).unwrap() {
            Action::Notify(request) => request,
            _ => panic!("notify"),
        };
        assert_eq!(request(&["notify"]), notify::Request::Send);
        assert_eq!(request(&["notify", "on"]), notify::Request::Enable);
        assert_eq!(request(&["notify", "off"]), notify::Request::Disable);
        assert!(run(&args(&["notify", "later"])).is_err());
        let Action::Text(help) = run(&args(&["--help"])).unwrap() else {
            panic!("help");
        };
        assert!(help.contains("drift notify [on | off]"));
    }

    #[test]
    fn snapshot_pre_is_a_subcommand_and_the_frame_flag_stays() {
        assert!(matches!(
            run(&args(&["snapshot", "--pre"])).unwrap(),
            Action::PreSnapshot
        ));
        let Action::Text(help) = run(&args(&["--help"])).unwrap() else {
            panic!("help");
        };
        assert!(help.contains("drift snapshot --pre"));
        assert!(matches!(
            run(&args(&["--snapshot"])).unwrap(),
            Action::Snapshot { .. }
        ));
        assert!(run(&args(&["snapshot"])).is_err());
        assert!(run(&args(&["snapshot", "--pre", "--yes"])).is_err());
        assert_eq!(execute(run(&args(&["snapshot"]))), ExitCode::from(2));
    }
    #[test]
    fn diff_usage_errors_start_with_the_command_prefix() {
        for values in [
            vec!["diff"],
            vec!["diff", ".config/a", ".config/b"],
            vec!["diff", ".config/a", "--unknown"],
        ] {
            let error = match run(&args(&values)) {
                Err(error) => error,
                Ok(_) => panic!("expected a usage error for {values:?}"),
            };
            assert!(error.starts_with("drift: "), "{values:?}: {error}");
        }
    }
}
