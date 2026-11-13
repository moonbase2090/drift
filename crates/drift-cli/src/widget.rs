use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use drift_apply::{
    widget_install, widget_remove, widget_sync, OmarchyShell, SystemOmarchy, WidgetMode,
    WidgetOptions, WidgetOutcome, WidgetPaths,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Install,
    Sync,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WidgetArgs {
    pub verb: Verb,
    pub options: WidgetOptions,
    pub source: Option<PathBuf>,
}

pub fn parse(args: &[String]) -> Result<WidgetArgs, String> {
    let (verb, rest) = match args.split_first() {
        Some((word, rest)) if word == "install" => (Verb::Install, rest),
        Some((word, rest)) if word == "sync" => (Verb::Sync, rest),
        Some((word, rest)) if word == "remove" => (Verb::Remove, rest),
        _ => return Err("widget needs install, sync or remove".to_string()),
    };
    let mut mode = WidgetMode::Confirm;
    let mut force = false;
    let mut place = true;
    let mut source = None;
    let mut index = 0;
    while index < rest.len() {
        match rest[index].as_str() {
            "--yes" | "-y" => mode = join(mode, WidgetMode::Yes)?,
            "--dry-run" => mode = join(mode, WidgetMode::DryRun)?,
            "--force" => force = true,
            "--no-place" => place = false,
            "--source" => {
                index += 1;
                let value = rest
                    .get(index)
                    .ok_or_else(|| "--source needs a folder".to_string())?;
                source = Some(PathBuf::from(value));
            }
            other => return Err(format!("unknown argument '{other}'")),
        }
        index += 1;
    }
    if force && mode == WidgetMode::DryRun {
        return Err("--force and --dry-run cannot be combined".to_string());
    }
    if verb == Verb::Remove && (force || !place) {
        return Err("remove takes only --yes, --dry-run and --source".to_string());
    }
    Ok(WidgetArgs {
        verb,
        options: WidgetOptions { mode, force, place },
        source,
    })
}

fn join(current: WidgetMode, next: WidgetMode) -> Result<WidgetMode, String> {
    if current != WidgetMode::Confirm && current != next {
        return Err("--yes and --dry-run cannot be combined".to_string());
    }
    Ok(next)
}

pub fn source_dir(explicit: Option<&Path>, env: Option<&std::ffi::OsStr>) -> PathBuf {
    if let Some(path) = explicit {
        return path.to_path_buf();
    }
    if let Some(path) = env.filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    let packaged = PathBuf::from("/usr/share/omarchy-drift/plugin");
    if packaged.is_dir() {
        return packaged;
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugin")
}

pub fn run(
    args: &WidgetArgs,
    paths: &WidgetPaths,
    omarchy: &mut dyn OmarchyShell,
    stamp: &str,
    interactive: bool,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<u8, String> {
    let mut ask = || {
        if !interactive {
            return false;
        }
        eprint!("Continue? [y/N] ");
        let mut line = String::new();
        input.read_line(&mut line).is_ok() && matches!(line.trim(), "y" | "Y" | "yes")
    };
    let outcome = match args.verb {
        Verb::Install => widget_install(paths, omarchy, args.options, stamp, out, &mut ask),
        Verb::Sync => widget_sync(paths, omarchy, args.options, stamp, out, &mut ask),
        Verb::Remove => widget_remove(paths, omarchy, args.options, stamp, out, &mut ask),
    }?;
    if outcome == WidgetOutcome::Declined && !interactive {
        eprintln!("drift: not a terminal, so nothing was written; pass --yes to apply");
    }
    Ok(match outcome {
        WidgetOutcome::Refused | WidgetOutcome::Declined => 1,
        _ => 0,
    })
}

pub fn run_now(args: &WidgetArgs) -> Result<u8, String> {
    let roots = crate::environment::roots_from_env()?;
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    let stamp: String = crate::environment::utc_timestamp(elapsed.as_secs())
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    let env = std::env::var_os("DRIFT_PLUGIN_DIR");
    let paths = WidgetPaths {
        source: source_dir(args.source.as_deref(), env.as_deref()),
        plugins: roots.live.join("plugins"),
        state: roots.state.clone(),
    };
    let interactive = std::io::IsTerminal::is_terminal(&std::io::stdin());
    run(
        args,
        &paths,
        &mut SystemOmarchy,
        &stamp,
        interactive,
        &mut std::io::stdin().lock(),
        &mut std::io::stdout(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(args: &[&str]) -> Vec<String> {
        args.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn the_verbs_and_flags_parse() {
        let found = parse(&words(&["install"])).unwrap();
        assert_eq!(found.verb, Verb::Install);
        assert_eq!(found.options.mode, WidgetMode::Confirm);
        assert!(found.options.place && !found.options.force);
        let found = parse(&words(&["sync", "--yes", "--force", "--no-place"])).unwrap();
        assert_eq!(found.options.mode, WidgetMode::Yes);
        assert!(found.options.force && !found.options.place);
        let found = parse(&words(&["remove", "--dry-run", "--source", "/x"])).unwrap();
        assert_eq!(found.options.mode, WidgetMode::DryRun);
        assert_eq!(found.source, Some(PathBuf::from("/x")));
    }

    #[test]
    fn bad_combinations_are_rejected() {
        for bad in [
            &["install", "--yes", "--dry-run"][..],
            &["install", "--force", "--dry-run"],
            &["remove", "--force"],
            &["remove", "--no-place"],
            &["install", "--source"],
            &["install", "--bogus"],
            &["frobnicate"],
            &[],
        ] {
            assert!(parse(&words(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_source_prefers_the_flag_then_the_environment() {
        let flag = Path::new("/flag");
        assert_eq!(
            source_dir(Some(flag), Some(std::ffi::OsStr::new("/env"))),
            PathBuf::from("/flag")
        );
        assert_eq!(
            source_dir(None, Some(std::ffi::OsStr::new("/env"))),
            PathBuf::from("/env")
        );
    }
}
