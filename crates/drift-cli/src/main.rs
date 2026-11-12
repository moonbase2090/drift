mod environment;

use std::io::IsTerminal;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use drift_apply::IntegrateMode;

const USAGE: &str = "\
Usage: drift [--snapshot [--width N] [--height N]] [--version] [--help]
       drift integrate [--yes | --dry-run]

With no arguments, drift opens the review screen.
--snapshot prints one frame and does not take the terminal.
integrate shows the changes it would make to the Omarchy menu and hook files and
asks before writing. --yes skips the question; --dry-run only shows the diff.
";

enum Action {
    Text(String),
    Snapshot { width: u16, height: u16 },
    Launch,
    Integrate(IntegrateMode),
}

fn run(args: &[String]) -> Result<Action, String> {
    if args.first().map(String::as_str) == Some("integrate") {
        return integrate_mode(&args[1..]).map(Action::Integrate);
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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    execute(run(&args))
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
        Ok(Action::Integrate(mode)) => match integrate_now(mode) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("drift: {error}");
                ExitCode::from(2)
            }
        },
        Ok(Action::Launch) => match drift_tui::launch() {
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
}
