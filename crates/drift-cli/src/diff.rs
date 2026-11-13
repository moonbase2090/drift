use drift_core::RelPath;
use drift_source::build_file_diff;
use drift_source::ScanPaths;
use std::{collections::BTreeSet, io::Write, path::Path};

pub struct DiffArgs {
    pub path: String,
    pub json: bool,
}

pub fn parse(args: &[String]) -> Result<DiffArgs, String> {
    let mut path = None;
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "--json" if !json => json = true,
            value if !value.starts_with('-') && path.is_none() => path = Some(value.to_string()),
            other => {
                return Err(format!(
                    "unexpected argument '{other}'; usage: drift diff <path> [--json]"
                ))
            }
        }
    }
    Ok(DiffArgs {
        path: path.ok_or("usage: drift diff <path> [--json]")?,
        json,
    })
}

pub(crate) fn live_path(paths: &ScanPaths, given: &str) -> Result<RelPath, String> {
    let supplied = Path::new(given);
    let relative = if supplied.is_absolute() {
        supplied
            .strip_prefix(&paths.source.home)
            .map_err(|_| "path is outside the home directory".to_string())?
    } else {
        supplied
    };
    RelPath::new(relative.to_str().ok_or("path must be UTF-8")?).map_err(|error| error.to_string())
}

pub fn run(paths: &ScanPaths, args: &DiffArgs, out: &mut dyn Write) -> Result<u8, String> {
    let path = live_path(paths, &args.path)?;
    let registry = crate::scan::registry(paths)?;
    let diff = build_file_diff(&registry, paths, &BTreeSet::new(), &path)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("unknown tracked path '{}'", args.path))?;
    let text = if args.json {
        diff.to_json()
    } else {
        diff.plain_text()
    };
    out.write_all(text.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|error| error.to_string())?;
    Ok(diff.exit_code())
}

pub struct LiveDiffer {
    paths: ScanPaths,
    registry: drift_core::Registry,
}

impl LiveDiffer {
    pub fn new(paths: ScanPaths, registry: drift_core::Registry) -> Self {
        Self { paths, registry }
    }
}

impl drift_tui::Differ for LiveDiffer {
    fn diff(&mut self, given: &str) -> Result<drift_core::FileDiff, String> {
        let path = live_path(&self.paths, given)?;
        build_file_diff(&self.registry, &self.paths, &BTreeSet::new(), &path)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("unknown tracked path '{given}'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn a_diff_takes_one_path_and_an_optional_json_flag() {
        let parsed = parse(&args(&[".config/hypr/a.lua"])).unwrap();
        assert_eq!(parsed.path, ".config/hypr/a.lua");
        assert!(!parsed.json);
        assert!(parse(&args(&["--json", ".config/a"])).unwrap().json);
        assert!(parse(&args(&[".config/a", "--json"])).unwrap().json);
        for invalid in [
            vec![],
            vec!["--json"],
            vec!["a", "b"],
            vec!["a", "--bar"],
            vec!["a", "--json", "--json"],
        ] {
            assert!(parse(&args(&invalid)).is_err(), "{invalid:?}");
        }
    }

    fn key(code: lunatui_term::KeyCode) -> lunatui_term::Event {
        lunatui_term::Event::Key(lunatui_term::KeyEvent::press(code))
    }

    fn frame(screen: &mut drift_tui::Screen) -> String {
        let (grid, clusters) = lunatui::render_once(screen, 200, 40);
        grid.display_text(&clusters)
    }

    #[test]
    fn enter_on_a_real_file_opens_the_diff_of_the_installed_package_and_the_live_file() {
        use lunatui::App as _;
        use lunatui_term::KeyCode;

        let root = crate::resolve::tests::world(&[(
            "hypr/a.lua",
            "new a\n",
            Some("mine a\n"),
            Some("old a\n"),
        )]);
        let paths = crate::resolve::tests::scan_paths(&root);
        let report = crate::scan::build(&paths).unwrap();
        let registry = crate::scan::registry(&paths).unwrap();
        let mut screen = drift_tui::Screen::with_report(&report)
            .with_differ(Box::new(LiveDiffer::new(paths, registry)));
        let tree = frame(&mut screen);
        assert!(tree.contains("a.lua"), "{tree}");
        for code in [KeyCode::Char('j'), KeyCode::Char('j'), KeyCode::Enter] {
            let _ = screen.update(&key(code));
        }
        let shown = frame(&mut screen);
        assert!(
            shown
                .lines()
                .next()
                .unwrap()
                .contains("Diff  .config/hypr/a.lua"),
            "{shown}"
        );
        for side in ["old a", "mine a", "new a"] {
            assert!(shown.contains(side), "{side}\n{shown}");
        }
        let _ = screen.update(&key(KeyCode::Char('v')));
        assert_ne!(frame(&mut screen), shown);
        let _ = screen.update(&key(KeyCode::Esc));
        assert!(screen.diff().is_none());
        assert!(frame(&mut screen).contains("Drift"));
    }

    #[test]
    fn the_live_differ_refuses_an_unknown_or_outside_path() {
        use drift_tui::Differ as _;

        let root = crate::resolve::tests::world(&[("hypr/a.lua", "n\n", Some("m\n"), None)]);
        let paths = crate::resolve::tests::scan_paths(&root);
        let registry = crate::scan::registry(&paths).unwrap();
        let mut differ = LiveDiffer::new(paths, registry);
        assert_eq!(
            differ.diff(".config/hypr/none.lua").unwrap_err(),
            "unknown tracked path '.config/hypr/none.lua'"
        );
        assert_eq!(
            differ.diff("/etc/passwd").unwrap_err(),
            "path is outside the home directory"
        );
        assert_eq!(
            differ.diff(".config/hypr/a.lua").unwrap().path,
            ".config/hypr/a.lua"
        );
    }
}
