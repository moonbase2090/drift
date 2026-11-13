use std::collections::BTreeSet;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::Duration;

use drift_apply::{execute, pending_prune, HyprlandCheck, Roots};
use drift_core::{prepare, Action, PlanView, Prepared, RelPath};
use drift_source::{load_resolve_inputs, ScanPaths};

use crate::diff::live_path;
use crate::environment::utc_timestamp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Confirm,
    Yes,
    DryRun,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveArgs {
    pub paths: Vec<String>,
    pub action: Action,
    pub mode: Mode,
}

const USAGE: &str =
    "usage: drift resolve <path>... --keep | --upstream | --merge [--yes | --dry-run]";

pub fn parse(args: &[String]) -> Result<ResolveArgs, String> {
    let mut paths = Vec::new();
    let mut action = None;
    let mut mode = Mode::Confirm;
    for arg in args {
        let chosen = match arg.as_str() {
            "--keep" => Some(Action::KeepMine),
            "--upstream" => Some(Action::TakeUpstream),
            "--merge" => Some(Action::Merge),
            _ => None,
        };
        if let Some(next) = chosen {
            if action.is_some_and(|current| current != next) {
                return Err(format!(
                    "choose one of --keep, --upstream and --merge; {USAGE}"
                ));
            }
            action = Some(next);
            continue;
        }
        let next_mode = match arg.as_str() {
            "--yes" | "-y" => Some(Mode::Yes),
            "--dry-run" => Some(Mode::DryRun),
            _ => None,
        };
        if let Some(next) = next_mode {
            if mode != Mode::Confirm && mode != next {
                return Err(format!("--yes and --dry-run cannot be combined; {USAGE}"));
            }
            mode = next;
            continue;
        }
        if arg.starts_with('-') {
            return Err(format!("unexpected argument '{arg}'; {USAGE}"));
        }
        if !paths.contains(arg) {
            paths.push(arg.clone());
        }
    }
    if paths.is_empty() {
        return Err(USAGE.to_string());
    }
    let Some(action) = action else {
        return Err(format!("choose --keep, --upstream or --merge; {USAGE}"));
    };
    Ok(ResolveArgs {
        paths,
        action,
        mode,
    })
}

pub struct Environment<'a> {
    pub paths: &'a ScanPaths,
    pub roots: &'a Roots,
    pub state_display: &'a str,
    pub checker: &'a dyn HyprlandCheck,
}

pub fn roots_for(paths: &ScanPaths, runtime: PathBuf) -> Roots {
    Roots {
        live: paths.source.home.clone(),
        state: paths.resolve.state.clone(),
        omarchy_lock: runtime.join("omarchy-update.lock"),
        pacman_lock: PathBuf::from("/var/lib/pacman/db.lck"),
    }
}

fn say(writer: &mut dyn Write, text: &str) -> Result<(), String> {
    writer
        .write_all(text.as_bytes())
        .and_then(|()| writer.flush())
        .map_err(|error| error.to_string())
}

fn confirmed(input: &mut dyn BufRead, interactive: bool) -> bool {
    let mut line = String::new();
    interactive && input.read_line(&mut line).is_ok() && matches!(line.trim(), "y" | "Y" | "yes")
}

fn prepare_for(
    env: &Environment<'_>,
    args: &ResolveArgs,
    elapsed: Duration,
    err: &mut dyn Write,
) -> Result<(Prepared, String), String> {
    let mut targets: Vec<RelPath> = Vec::new();
    for given in &args.paths {
        targets.push(live_path(env.paths, given)?);
    }
    let registry = crate::scan::registry(env.paths)?;
    let loaded = load_resolve_inputs(&registry, env.paths, &BTreeSet::new(), &targets)
        .map_err(|error| error.to_string())?;
    if !loaded.unknown.is_empty() {
        for path in &loaded.unknown {
            say(err, &format!("drift: unknown tracked path '{path}'\n"))?;
        }
        return Err("nothing was written".to_string());
    }
    let id = format!("resolve-{}", elapsed.as_millis());
    let prune = pending_prune(&env.roots.state).map_err(|error| error.to_string())?;
    let prune_refs: Vec<&str> = prune.iter().map(String::as_str).collect();
    let prepared = prepare(
        args.action,
        &loaded.inputs,
        &id,
        &utc_timestamp(elapsed.as_secs()),
        &loaded.package_version,
        &prune_refs,
    )
    .map_err(|error| error.to_string())?;
    for skip in &prepared.skipped {
        say(
            err,
            &format!("skipped {}: {}\n", skip.path, skip.reason.label()),
        )?;
    }
    Ok((prepared, id))
}

pub fn run(
    env: &Environment<'_>,
    args: &ResolveArgs,
    elapsed: Duration,
    interactive: bool,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<u8, String> {
    let (prepared, id) = prepare_for(env, args, elapsed, err)?;
    let Some(plan) = prepared.plan.as_ref() else {
        return Err("nothing to do".to_string());
    };
    let view = PlanView::build(plan, env.state_display, &prepared.contents);
    match args.mode {
        Mode::DryRun => {
            say(out, &view.render_text())?;
            return Ok(0);
        }
        Mode::Confirm => {
            say(out, &view.render_body())?;
            say(out, "\nApply this plan? [y/N] ")?;
            if !confirmed(input, interactive) {
                say(out, "Nothing was written.\n")?;
                if !interactive {
                    say(
                        err,
                        "drift: not a terminal, so nothing was written; pass --yes to apply\n",
                    )?;
                }
                return Ok(0);
            }
        }
        Mode::Yes => say(out, &view.render_body())?,
    }
    let done = execute(env.roots, &env.paths.resolve.share, &prepared, env.checker)
        .map_err(|error| error.to_string())?;
    say(
        out,
        &format!(
            "\nApplied {} file(s) as transaction {id}; {} reviewed base(s) recorded.\n",
            done.checked.report.applied.len(),
            done.recorded.len()
        ),
    )?;
    for failed in &done.unrecorded {
        say(
            err,
            &format!("drift: could not record the reviewed base for {failed}\n"),
        )?;
    }
    if let Some(alert) = done.checked.followup.alert() {
        say(err, &format!("{alert}\n"))?;
    }
    Ok(0)
}

pub struct LiveResolver {
    paths: ScanPaths,
    roots: Roots,
    checker: Box<dyn HyprlandCheck>,
    registry: drift_core::Registry,
}

impl LiveResolver {
    pub fn new(
        paths: ScanPaths,
        roots: Roots,
        checker: Box<dyn HyprlandCheck>,
        registry: drift_core::Registry,
    ) -> Self {
        Self {
            paths,
            roots,
            checker,
            registry,
        }
    }
}

fn now_millis() -> Duration {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
}

impl drift_tui::Resolver for LiveResolver {
    fn propose(&mut self, action: Action, paths: &[String]) -> Result<drift_tui::Proposal, String> {
        let mut targets: Vec<RelPath> = Vec::new();
        for given in paths {
            targets.push(live_path(&self.paths, given)?);
        }
        let loaded = load_resolve_inputs(&self.registry, &self.paths, &BTreeSet::new(), &targets)
            .map_err(|error| error.to_string())?;
        if let Some(path) = loaded.unknown.first() {
            return Err(format!("unknown tracked path '{path}'"));
        }
        let elapsed = now_millis();
        let id = format!("resolve-{}", elapsed.as_millis());
        let prune = pending_prune(&self.roots.state).map_err(|error| error.to_string())?;
        let prune_refs: Vec<&str> = prune.iter().map(String::as_str).collect();
        let prepared = prepare(
            action,
            &loaded.inputs,
            &id,
            &utc_timestamp(elapsed.as_secs()),
            &loaded.package_version,
            &prune_refs,
        )
        .map_err(|error| error.to_string())?;
        let skipped = prepared
            .skipped
            .iter()
            .map(|skip| format!("{} {}", skip.path, skip.reason.label()))
            .collect();
        let state = self.roots.state.display().to_string();
        let view = match prepared.plan.as_ref() {
            Some(plan) => PlanView::build(plan, &state, &prepared.contents),
            None => PlanView {
                id,
                time: String::new(),
                package_version: loaded.package_version,
                files: Vec::new(),
                prune: Vec::new(),
            },
        };
        Ok(drift_tui::Proposal {
            prepared,
            view,
            skipped,
        })
    }

    fn apply(&mut self, proposal: &drift_tui::Proposal) -> Result<String, String> {
        let done = execute(
            &self.roots,
            &self.paths.resolve.share,
            &proposal.prepared,
            self.checker.as_ref(),
        )
        .map_err(|error| error.to_string())?;
        let mut message = format!(
            "Applied {} file(s) as transaction {}; {} reviewed base(s) recorded.",
            done.checked.report.applied.len(),
            proposal.view.id,
            done.recorded.len()
        );
        if !done.unrecorded.is_empty() {
            message.push_str(&format!(
                " Could not record: {}.",
                done.unrecorded.join(", ")
            ));
        }
        if let Some(alert) = done.checked.followup.alert() {
            message.push(' ');
            message.push_str(&alert.replace('\n', " "));
        }
        Ok(message)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;

    use drift_apply::{HyprlandStatus, Roots};
    use drift_fixtures::{hash_tree, MaterializedRoot, RootBuilder};
    use drift_source::{ReadPaths, ResolvePaths, SourcePaths};
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use sha2::{Digest, Sha256};

    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|item| item.to_string()).collect()
    }

    pub(crate) fn sha(text: &str) -> String {
        Sha256::digest(text.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub(crate) fn world(files: &[(&str, &str, Option<&str>, Option<&str>)]) -> MaterializedRoot {
        let mut builder = RootBuilder::new();
        for keep in [
            "state/.keep",
            "runtime/.keep",
            "home/.local/share/.keep",
            "home/.local/state/.keep",
        ] {
            builder.file(keep, b"").unwrap();
        }
        let mut listed = String::from("%FILES%\nusr/share/omarchy/config/\n");
        let mut mtree = String::from("#mtree\n/set type=file uid=0 gid=0 mode=644\n");
        for (rel, theirs, ours, base) in files {
            listed.push_str(&format!("usr/share/omarchy/config/{rel}\n"));
            mtree.push_str(&format!(
                "./usr/share/omarchy/config/{rel} time=1.0 size=1 sha256digest={}\n",
                sha(theirs)
            ));
            builder
                .file(format!("usr/share/omarchy/config/{rel}"), theirs)
                .unwrap();
            if let Some(ours) = ours {
                builder.file(format!("home/.config/{rel}"), ours).unwrap();
            }
            if let Some(base) = base {
                builder
                    .file(format!("home/.local/share/drift/bases/config/{rel}"), base)
                    .unwrap();
            }
        }
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(mtree.as_bytes()).unwrap();
        let dir = "var/lib/pacman/local/omarchy-settings-4.5.0-3";
        builder
            .file(
                format!("{dir}/desc"),
                "%NAME%\nomarchy-settings\n\n%VERSION%\n4.5.0-3\n",
            )
            .unwrap()
            .file(format!("{dir}/files"), listed)
            .unwrap()
            .file(format!("{dir}/mtree"), encoder.finish().unwrap())
            .unwrap();
        builder.materialize().unwrap()
    }

    pub(crate) fn scan_paths(root: &MaterializedRoot) -> ScanPaths {
        let base = root.path().to_path_buf();
        ScanPaths {
            source: SourcePaths::new(
                ReadPaths::new(&base, base.join("usr/share/omarchy")),
                base.join("home"),
                base.join("skel"),
            ),
            resolve: ResolvePaths {
                share: base.join("home/.local/share/drift"),
                state: base.join("state"),
                snapshots: base.join("snap"),
                cache: base.join("cache"),
                log: base.join("log"),
            },
        }
    }

    struct Checker(HyprlandStatus);

    impl HyprlandCheck for Checker {
        fn config_errors(&self) -> HyprlandStatus {
            self.0.clone()
        }
    }

    struct Run {
        result: Result<u8, String>,
        out: String,
        err: String,
    }

    fn go(
        root: &MaterializedRoot,
        list: &[&str],
        interactive: bool,
        typed: &str,
        checker: &Checker,
    ) -> Run {
        go_at(root, list, interactive, typed, checker, 1_800_000_000_123)
    }

    fn go_at(
        root: &MaterializedRoot,
        list: &[&str],
        interactive: bool,
        typed: &str,
        checker: &Checker,
        millis: u64,
    ) -> Run {
        let paths = scan_paths(root);
        let roots: Roots = roots_for(&paths, root.path().join("runtime"));
        let env = Environment {
            paths: &paths,
            roots: &roots,
            state_display: "STATE",
            checker,
        };
        let parsed = parse(&args(list)).unwrap();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = run(
            &env,
            &parsed,
            Duration::from_millis(millis),
            interactive,
            &mut typed.as_bytes(),
            &mut out,
            &mut err,
        );
        Run {
            result,
            out: String::from_utf8(out).unwrap(),
            err: String::from_utf8(err).unwrap(),
        }
    }

    fn clean() -> Checker {
        Checker(HyprlandStatus::Clean)
    }

    fn read(root: &MaterializedRoot, rel: &str) -> String {
        fs::read_to_string(root.path().join(rel)).unwrap()
    }

    fn transactions(root: &MaterializedRoot) -> usize {
        fs::read_dir(root.path().join("state/backups"))
            .map(|dir| dir.count())
            .unwrap_or(0)
    }

    #[test]
    fn the_command_line_names_paths_one_action_and_at_most_one_mode() {
        let parsed = parse(&args(&[".config/a", ".config/b", "--keep", "--yes"])).unwrap();
        assert_eq!(parsed.paths, [".config/a", ".config/b"]);
        assert_eq!(parsed.action, Action::KeepMine);
        assert_eq!(parsed.mode, Mode::Yes);
        assert_eq!(
            parse(&args(&["a", "--upstream", "-y", "--yes"]))
                .unwrap()
                .mode,
            Mode::Yes
        );
        assert_eq!(
            parse(&args(&["a", "--merge", "--dry-run"])).unwrap().mode,
            Mode::DryRun
        );
        assert_eq!(parse(&args(&["a", "a", "--merge"])).unwrap().paths, ["a"]);
        assert_eq!(parse(&args(&["a", "--merge"])).unwrap().mode, Mode::Confirm);
        for invalid in [
            vec![],
            vec!["--keep"],
            vec!["a"],
            vec!["a", "--keep", "--upstream"],
            vec!["a", "--keep", "--yes", "--dry-run"],
            vec!["a", "--keep", "--dry-run", "--yes"],
            vec!["a", "--keep", "--bogus"],
        ] {
            assert!(parse(&args(&invalid)).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn a_dry_run_prints_the_plan_and_writes_nothing() {
        let root = world(&[("hypr/a.lua", "new\n", Some("mine\n"), Some("old\n"))]);
        let before = hash_tree(root.path()).unwrap();
        let ran = go(
            &root,
            &[".config/hypr/a.lua", "--upstream", "--dry-run"],
            false,
            "",
            &clean(),
        );
        assert_eq!(ran.result, Ok(0));
        assert!(
            ran.out
                .contains("take upstream  .config/hypr/a.lua  (+1 -1)"),
            "{}",
            ran.out
        );
        assert!(
            ran.out
                .contains("backup  STATE/backups/resolve-1800000000123/.config/hypr/a.lua"),
            "{}",
            ran.out
        );
        assert!(ran.out.contains("package 4.5.0-3\n"), "{}", ran.out);
        assert!(
            ran.out.ends_with("Dry run: nothing was written.\n"),
            "{}",
            ran.out
        );
        assert_eq!(hash_tree(root.path()).unwrap(), before);
    }

    #[test]
    fn without_a_terminal_and_without_yes_nothing_is_written() {
        let root = world(&[("hypr/a.lua", "new\n", Some("mine\n"), Some("old\n"))]);
        let before = hash_tree(root.path()).unwrap();
        let ran = go(
            &root,
            &[".config/hypr/a.lua", "--upstream"],
            false,
            "y\n",
            &clean(),
        );
        assert_eq!(ran.result, Ok(0));
        assert!(ran.out.contains("Nothing was written."), "{}", ran.out);
        assert!(ran.err.contains("pass --yes"), "{}", ran.err);
        assert_eq!(hash_tree(root.path()).unwrap(), before);
    }

    #[test]
    fn at_a_terminal_only_a_yes_answer_applies() {
        let answers = ["n\n", "\n", "maybe\n", ""];
        for answer in answers {
            let root = world(&[("hypr/a.lua", "new\n", Some("mine\n"), Some("old\n"))]);
            let before = hash_tree(root.path()).unwrap();
            let ran = go(
                &root,
                &[".config/hypr/a.lua", "--upstream"],
                true,
                answer,
                &clean(),
            );
            assert_eq!(ran.result, Ok(0));
            assert!(ran.out.contains("Apply this plan? [y/N]"), "{}", ran.out);
            assert_eq!(hash_tree(root.path()).unwrap(), before, "{answer:?}");
        }
        for answer in ["y\n", "Y\n", "yes\n"] {
            let root = world(&[("hypr/a.lua", "new\n", Some("mine\n"), Some("old\n"))]);
            let ran = go(
                &root,
                &[".config/hypr/a.lua", "--upstream"],
                true,
                answer,
                &clean(),
            );
            assert_eq!(ran.result, Ok(0));
            assert_eq!(
                read(&root, "home/.config/hypr/a.lua"),
                "new\n",
                "{answer:?}"
            );
        }
    }

    #[test]
    fn keep_take_and_merge_with_yes_do_what_they_say_in_one_transaction() {
        let root = world(&[
            ("hypr/a.lua", "new\n", Some("mine\n"), Some("old\n")),
            (
                "hypr/b.lua",
                "one\ntwo\nTHREE\n",
                Some("ONE\ntwo\nthree\n"),
                Some("one\ntwo\nthree\n"),
            ),
            ("hypr/c.lua", "c new\n", Some("c mine\n"), Some("c old\n")),
        ]);
        let ran = go(
            &root,
            &[
                ".config/hypr/a.lua",
                ".config/hypr/c.lua",
                "--upstream",
                "--yes",
            ],
            false,
            "",
            &clean(),
        );
        assert_eq!(ran.result, Ok(0), "{}", ran.err);
        assert!(ran.out.contains("Applied 2 file(s) as transaction resolve-1800000000123; 2 reviewed base(s) recorded."), "{}", ran.out);
        assert_eq!(read(&root, "home/.config/hypr/a.lua"), "new\n");
        assert_eq!(read(&root, "home/.config/hypr/c.lua"), "c new\n");
        assert_eq!(transactions(&root), 1);

        let ran = go_at(
            &root,
            &[".config/hypr/b.lua", "--merge", "--yes"],
            false,
            "",
            &clean(),
            1_800_000_000_999,
        );
        assert_eq!(ran.result, Ok(0), "{}", ran.err);
        assert_eq!(read(&root, "home/.config/hypr/b.lua"), "ONE\ntwo\nTHREE\n");
    }

    #[test]
    fn keep_mine_leaves_the_file_and_records_the_template_as_reviewed() {
        let root = world(&[("hypr/a.lua", "new\n", Some("mine\n"), Some("old\n"))]);
        let ran = go(
            &root,
            &[".config/hypr/a.lua", "--keep", "--yes"],
            false,
            "",
            &clean(),
        );
        assert_eq!(ran.result, Ok(0), "{}", ran.err);
        assert_eq!(read(&root, "home/.config/hypr/a.lua"), "mine\n");
        assert_eq!(
            read(&root, "home/.local/share/drift/bases/config/hypr/a.lua"),
            "new\n"
        );
    }

    #[test]
    fn absolute_paths_under_home_work_and_outside_ones_do_not() {
        let root = world(&[("hypr/a.lua", "new\n", Some("mine\n"), Some("old\n"))]);
        let absolute = root.path().join("home/.config/hypr/a.lua");
        let ran = go(
            &root,
            &[absolute.to_str().unwrap(), "--upstream", "--yes"],
            false,
            "",
            &clean(),
        );
        assert_eq!(ran.result, Ok(0), "{}", ran.err);
        let ran = go(
            &root,
            &["/etc/passwd", "--upstream", "--yes"],
            false,
            "",
            &clean(),
        );
        assert!(ran
            .result
            .unwrap_err()
            .contains("outside the home directory"));
    }

    #[test]
    fn an_unknown_path_stops_everything_and_names_it() {
        let root = world(&[("hypr/a.lua", "new\n", Some("mine\n"), Some("old\n"))]);
        let before = hash_tree(root.path()).unwrap();
        let ran = go(
            &root,
            &[
                ".config/hypr/a.lua",
                ".config/hypr/none.lua",
                "--upstream",
                "--yes",
            ],
            false,
            "",
            &clean(),
        );
        assert!(ran.result.is_err());
        assert!(
            ran.err
                .contains("unknown tracked path '.config/hypr/none.lua'"),
            "{}",
            ran.err
        );
        assert_eq!(hash_tree(root.path()).unwrap(), before);
    }

    #[test]
    fn files_that_cannot_be_acted_on_are_named_and_nothing_to_do_is_an_error() {
        let root = world(&[("hypr/a.lua", "x\n", Some("x\n"), None)]);
        let before = hash_tree(root.path()).unwrap();
        let ran = go(
            &root,
            &[".config/hypr/a.lua", "--upstream", "--yes"],
            false,
            "",
            &clean(),
        );
        assert_eq!(ran.result, Err("nothing to do".to_string()));
        assert!(
            ran.err
                .contains("skipped .config/hypr/a.lua: nothing to do"),
            "{}",
            ran.err
        );
        assert_eq!(hash_tree(root.path()).unwrap(), before);
    }

    #[test]
    fn some_skipped_files_do_not_stop_the_rest() {
        let root = world(&[
            ("hypr/a.lua", "new\n", Some("mine\n"), Some("old\n")),
            ("hypr/b.lua", "same\n", Some("same\n"), None),
        ]);
        let ran = go(
            &root,
            &[
                ".config/hypr/a.lua",
                ".config/hypr/b.lua",
                "--upstream",
                "--yes",
            ],
            false,
            "",
            &clean(),
        );
        assert_eq!(ran.result, Ok(0));
        assert!(
            ran.err
                .contains("skipped .config/hypr/b.lua: nothing to do"),
            "{}",
            ran.err
        );
        assert_eq!(read(&root, "home/.config/hypr/a.lua"), "new\n");
    }

    #[test]
    fn a_hyprland_error_after_a_write_prints_the_undo_alert() {
        let root = world(&[("hypr/a.lua", "new\n", Some("mine\n"), Some("old\n"))]);
        let broken = Checker(HyprlandStatus::Errors("line 3: bad".to_string()));
        let ran = go(
            &root,
            &[".config/hypr/a.lua", "--upstream", "--yes"],
            false,
            "",
            &broken,
        );
        assert_eq!(ran.result, Ok(0));
        assert!(
            ran.err.contains(
                "Hyprland reported config errors after transaction resolve-1800000000123"
            ),
            "{}",
            ran.err
        );
        assert!(ran.err.contains("line 3: bad"), "{}", ran.err);
    }

    #[test]
    fn a_held_update_lock_refuses_the_write() {
        let root = world(&[("hypr/a.lua", "new\n", Some("mine\n"), Some("old\n"))]);
        let paths = scan_paths(&root);
        let mut roots = roots_for(&paths, root.path().join("runtime"));
        roots.omarchy_lock = root.path().join("home/.config/hypr/a.lua");
        let env = Environment {
            paths: &paths,
            roots: &roots,
            state_display: "STATE",
            checker: &clean(),
        };
        let parsed = parse(&args(&[".config/hypr/a.lua", "--upstream", "--yes"])).unwrap();
        let result = run(
            &env,
            &parsed,
            Duration::from_millis(1),
            false,
            &mut "".as_bytes(),
            &mut Vec::new(),
            &mut Vec::new(),
        );
        assert!(result.unwrap_err().contains("omarchy update lock"));
        assert_eq!(read(&root, "home/.config/hypr/a.lua"), "mine\n");
    }

    fn live(root: &MaterializedRoot, status: HyprlandStatus) -> LiveResolver {
        let paths = scan_paths(root);
        let roots = roots_for(&paths, root.path().join("runtime"));
        LiveResolver::new(
            paths,
            roots,
            Box::new(Checker(status)),
            drift_core::Registry::v1(),
        )
    }

    #[test]
    fn the_review_screen_resolver_proposes_applies_and_reports_in_one_transaction() {
        use drift_tui::Resolver as _;
        let root = world(&[
            ("hypr/a.lua", "new\n", Some("mine\n"), Some("old\n")),
            ("hypr/b.lua", "new b\n", Some("mine b\n"), Some("old b\n")),
        ]);
        let mut resolver = live(&root, HyprlandStatus::Clean);
        let before = hash_tree(root.path()).unwrap();
        let proposal = resolver
            .propose(
                Action::TakeUpstream,
                &[
                    ".config/hypr/a.lua".to_string(),
                    ".config/hypr/b.lua".to_string(),
                ],
            )
            .unwrap();
        assert_eq!(proposal.view.files.len(), 2);
        assert!(proposal.skipped.is_empty());
        assert_eq!(hash_tree(root.path()).unwrap(), before);
        let message = resolver.apply(&proposal).unwrap();
        assert!(
            message.starts_with("Applied 2 file(s) as transaction resolve-"),
            "{message}"
        );
        assert!(
            message.contains("2 reviewed base(s) recorded."),
            "{message}"
        );
        assert_eq!(read(&root, "home/.config/hypr/a.lua"), "new\n");
        assert_eq!(read(&root, "home/.config/hypr/b.lua"), "new b\n");
        assert_eq!(transactions(&root), 1);
    }

    #[test]
    fn the_resolver_names_skipped_files_unknown_paths_and_nothing_to_do() {
        use drift_tui::Resolver as _;
        let root = world(&[("hypr/a.lua", "same\n", Some("same\n"), None)]);
        let mut resolver = live(&root, HyprlandStatus::Clean);
        let proposal = resolver
            .propose(Action::TakeUpstream, &[".config/hypr/a.lua".to_string()])
            .unwrap();
        assert!(proposal.prepared.plan.is_none());
        assert_eq!(proposal.skipped, [".config/hypr/a.lua nothing to do"]);
        assert!(proposal.view.files.is_empty());
        let error = resolver
            .propose(Action::KeepMine, &[".config/hypr/none.lua".to_string()])
            .unwrap_err();
        assert!(error.contains("unknown tracked path"), "{error}");
        let error = resolver
            .propose(Action::KeepMine, &["/etc/passwd".to_string()])
            .unwrap_err();
        assert!(error.contains("outside the home directory"), "{error}");
        assert!(resolver.apply(&proposal).is_err());
        assert_eq!(transactions(&root), 0);
    }

    #[test]
    fn the_resolver_message_carries_the_hyprland_undo_offer() {
        use drift_tui::Resolver as _;
        let root = world(&[("hypr/a.lua", "new\n", Some("mine\n"), Some("old\n"))]);
        let mut resolver = live(&root, HyprlandStatus::Errors("line 3: bad".to_string()));
        let proposal = resolver
            .propose(Action::TakeUpstream, &[".config/hypr/a.lua".to_string()])
            .unwrap();
        let message = resolver.apply(&proposal).unwrap();
        assert!(
            message.contains("Hyprland reported config errors after transaction resolve-"),
            "{message}"
        );
        assert!(message.contains("line 3: bad"), "{message}");
        assert!(!message.contains('\n'), "{message}");
    }
}
