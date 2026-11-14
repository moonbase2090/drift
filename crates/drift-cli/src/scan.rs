use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use drift_core::{registry_from_user_config, Registry, ScanReport, USER_CONFIG_PATH};
use drift_source::{build_report, drift_share, drift_state, ResolvePaths, ScanPaths, SourcePaths};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Scan,
    Status,
    Json,
    Bar,
}

pub fn view_from(args: &[String]) -> Result<View, String> {
    let mut view = View::Status;
    let mut chosen = false;
    for arg in args {
        let next = match arg.as_str() {
            "--json" => View::Json,
            "--bar" => View::Bar,
            other => return Err(format!("unknown argument '{other}'")),
        };
        if chosen && view != next {
            return Err("--json and --bar cannot be combined".to_string());
        }
        view = next;
        chosen = true;
    }
    Ok(view)
}

pub fn paths_from_env() -> ScanPaths {
    paths_from(SourcePaths::system(), &|name| std::env::var_os(name))
}

pub fn paths_from(source: SourcePaths, lookup: &dyn Fn(&str) -> Option<OsString>) -> ScanPaths {
    let var = |name: &str| lookup(name).filter(|value| !value.is_empty());
    let data_home = var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| source.home.join(".local/share"));
    let state_home = var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| source.home.join(".local/state"));
    ScanPaths {
        resolve: ResolvePaths {
            share: drift_share(&data_home),
            state: drift_state(&state_home),
            snapshots: drift_source::system_path("/var/lib/drift/snapshots"),
            cache: drift_source::system_path("/var/cache/pacman/pkg"),
            log: drift_source::system_path("/var/log/pacman.log"),
        },
        source,
    }
}

pub(crate) fn load_registry(paths: &ScanPaths) -> Result<Registry, String> {
    registry(paths)
}

pub(crate) fn scan_report(paths: &ScanPaths, registry: &Registry) -> Result<ScanReport, String> {
    build_report(registry, paths, &BTreeSet::new()).map_err(|error| error.to_string())
}

pub(crate) fn registry(paths: &ScanPaths) -> Result<Registry, String> {
    let path = paths.source.home.join(USER_CONFIG_PATH);
    match fs::read_to_string(&path) {
        Ok(text) => {
            registry_from_user_config(&text).map_err(|error| format!("{}: {error}", path.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Registry::v1()),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

pub fn build(paths: &ScanPaths) -> Result<drift_core::ScanReport, String> {
    let registry = registry(paths)?;
    build_report(&registry, paths, &BTreeSet::new()).map_err(|error| error.to_string())
}

pub fn run(paths: &ScanPaths, view: View, out: &mut dyn Write) -> Result<u8, String> {
    let report = build(paths)?;
    let text = match view {
        View::Scan => report.scan_text(),
        View::Status => report.status_text(),
        View::Json => report.to_json(),
        View::Bar => report.bar_json(),
    };
    out.write_all(text.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|error| error.to_string())?;
    Ok(report.exit_code())
}

#[cfg(test)]
mod tests {
    use drift_fixtures::{hash_tree, MaterializedRoot, RootBuilder};
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use sha2::{Digest, Sha256};

    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|item| item.to_string()).collect()
    }

    fn digest(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn world(files: &[(&str, &[u8], &[u8])], extra: &[(&str, &str)]) -> MaterializedRoot {
        let mut listed = String::from("%FILES%\nusr/share/omarchy/config/\n");
        let mut mtree = String::from("#mtree\n/set type=file uid=0 gid=0 mode=644\n");
        let mut builder = RootBuilder::new();
        for (rel, theirs, ours) in files {
            listed.push_str(&format!("usr/share/omarchy/config/{rel}\n"));
            mtree.push_str(&format!(
                "./usr/share/omarchy/config/{rel} time=1.0 size=1 sha256digest={}\n",
                digest(theirs)
            ));
            builder
                .file(format!("usr/share/omarchy/config/{rel}"), theirs)
                .unwrap();
            if !ours.is_empty() {
                builder.file(format!("home/.config/{rel}"), ours).unwrap();
            }
        }
        for (path, text) in extra {
            builder.file(*path, text).unwrap();
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

    fn paths(root: &MaterializedRoot) -> ScanPaths {
        let base = root.path().to_path_buf();
        paths_from(
            SourcePaths::new(
                drift_source::ReadPaths::new(&base, base.join("usr/share/omarchy")),
                base.join("home"),
                base.join("skel"),
            ),
            &|_| None,
        )
    }

    fn output(paths: &ScanPaths, view: View) -> (Result<u8, String>, String) {
        let mut out = Vec::new();
        let code = run(paths, view, &mut out);
        (code, String::from_utf8(out).unwrap())
    }

    #[test]
    fn views_are_chosen_from_the_flags() {
        assert_eq!(view_from(&[]).unwrap(), View::Status);
        assert_eq!(view_from(&args(&["--json"])).unwrap(), View::Json);
        assert_eq!(view_from(&args(&["--bar"])).unwrap(), View::Bar);
        assert_eq!(view_from(&args(&["--json", "--json"])).unwrap(), View::Json);
        assert!(view_from(&args(&["--json", "--bar"])).is_err());
        assert!(view_from(&args(&["--bar", "--json"])).is_err());
        assert!(view_from(&args(&["--yaml"])).is_err());
    }

    #[test]
    fn xdg_directories_override_the_home_defaults() {
        let source = || {
            SourcePaths::new(
                drift_source::ReadPaths::new("/", "/usr/share/omarchy"),
                "/h",
                "/skel",
            )
        };
        let plain = paths_from(source(), &|_| None);
        assert_eq!(plain.resolve.share, PathBuf::from("/h/.local/share/drift"));
        assert_eq!(plain.resolve.state, PathBuf::from("/h/.local/state/drift"));
        assert_eq!(
            plain.resolve.snapshots,
            PathBuf::from("/var/lib/drift/snapshots")
        );
        let custom = paths_from(source(), &|name| match name {
            "XDG_DATA_HOME" => Some("/d".into()),
            "XDG_STATE_HOME" => Some("/s".into()),
            _ => None,
        });
        assert_eq!(custom.resolve.share, PathBuf::from("/d/drift"));
        assert_eq!(custom.resolve.state, PathBuf::from("/s/drift"));
        let empty = paths_from(source(), &|_| Some(OsString::new()));
        assert_eq!(empty.resolve.share, PathBuf::from("/h/.local/share/drift"));
    }

    #[test]
    fn no_drift_exits_zero_and_drift_exits_one() {
        let clean = world(&[("hypr/a.lua", b"x\n", b"x\n")], &[]);
        let (code, text) = output(&paths(&clean), View::Status);
        assert_eq!(code.unwrap(), 0);
        assert!(text.contains("No drift (1 unchanged)"), "{text}");

        let missing = world(&[("hypr/a.lua", b"x\n", b"")], &[]);
        let (code, text) = output(&paths(&missing), View::Status);
        assert_eq!(code.unwrap(), 1);
        assert!(text.contains("1 drifted (1 missing)"), "{text}");

        let overridden = world(&[("hypr/a.lua", b"x\n", b"mine\n")], &[]);
        assert_eq!(output(&paths(&overridden), View::Status).0.unwrap(), 0);
    }

    #[test]
    fn every_view_uses_the_same_exit_code_and_its_own_format() {
        let root = world(
            &[("hypr/a.lua", b"x\n", b""), ("hypr/b.lua", b"y\n", b"y\n")],
            &[],
        );
        let paths = paths(&root);
        let (code, scan) = output(&paths, View::Scan);
        assert_eq!(code.unwrap(), 1);
        assert!(scan.contains("missing"), "{scan}");
        assert!(scan.contains(".config/hypr/b.lua"), "{scan}");
        let (code, json) = output(&paths, View::Json);
        assert_eq!(code.unwrap(), 1);
        assert!(json.starts_with("{\n"), "{json}");
        assert!(json.contains("\"drifted\": 1"), "{json}");
        let (code, bar) = output(&paths, View::Bar);
        assert_eq!(code.unwrap(), 1);
        assert_eq!(bar.lines().count(), 1);
        assert!(bar.contains("\"text\":\"1\""), "{bar}");
    }

    #[test]
    fn the_bar_text_is_empty_at_zero_drift() {
        let clean = world(&[("hypr/a.lua", b"x\n", b"x\n")], &[]);
        let (code, bar) = output(&paths(&clean), View::Bar);
        assert_eq!(code.unwrap(), 0);
        assert!(bar.contains("\"text\":\"\""), "{bar}");
    }

    #[test]
    fn no_read_command_writes_anything() {
        let root = world(
            &[
                ("hypr/a.lua", b"x\n", b""),
                ("hypr/b.lua", b"y\n", b"changed\n"),
                ("hypr/c.lua", b"z\n", b"z\n"),
            ],
            &[("home/.config/hypr/mine.lua", "m\n")],
        );
        let before = hash_tree(root.path()).unwrap();
        for view in [View::Scan, View::Status, View::Json, View::Bar] {
            output(&paths(&root), view).0.unwrap();
            assert_eq!(hash_tree(root.path()).unwrap(), before, "{view:?}");
        }
    }

    #[test]
    fn a_missing_package_is_an_error_with_no_output_and_no_writes() {
        let mut builder = RootBuilder::new();
        builder.file("home/.config/hypr/a.lua", b"x\n").unwrap();
        let root = builder.materialize().unwrap();
        let before = hash_tree(root.path()).unwrap();
        let (result, text) = output(&paths(&root), View::Status);
        assert!(result.is_err());
        assert!(text.is_empty());
        assert_eq!(hash_tree(root.path()).unwrap(), before);
    }

    #[test]
    fn a_user_config_adds_roots_and_a_broken_one_is_an_error() {
        let config = "[[root]]\nid = \"apps\"\ntemplate = \"applications\"\nlive = \".local/share/applications\"\n";
        let good = world(
            &[("hypr/a.lua", b"x\n", b"x\n")],
            &[("home/.config/drift/config.toml", config)],
        );
        assert_eq!(output(&paths(&good), View::Status).0.unwrap(), 0);
        let bad = world(
            &[("hypr/a.lua", b"x\n", b"x\n")],
            &[("home/.config/drift/config.toml", "[[root]]\nid = 3\n")],
        );
        let (result, _) = output(&paths(&bad), View::Status);
        let error = result.unwrap_err();
        assert!(error.contains("config.toml"), "{error}");
    }
    #[test]
    fn diff_views_are_read_only_and_use_the_selected_files_exit_code() {
        let root = world(
            &[("hypr/a.lua", b"x\n", b""), ("hypr/b.lua", b"y\n", b"y\n")],
            &[],
        );
        let paths = paths(&root);
        let before = hash_tree(root.path()).unwrap();
        for json in [false, true] {
            for (path, code) in [(".config/hypr/a.lua", 1), (".config/hypr/b.lua", 0)] {
                let mut out = Vec::new();
                let args = crate::diff::DiffArgs {
                    path: path.to_string(),
                    json,
                };
                assert_eq!(crate::diff::run(&paths, &args, &mut out).unwrap(), code);
                let text = String::from_utf8(out).unwrap();
                assert!(text.contains(path));
                assert_eq!(text.starts_with("{\n"), json);
                let args = crate::diff::DiffArgs {
                    path: paths.source.home.join(path).to_string_lossy().into_owned(),
                    json,
                };
                assert_eq!(
                    crate::diff::run(&paths, &args, &mut Vec::new()).unwrap(),
                    code
                );
            }
        }
        for path in [".config/unknown", "../outside", "/outside/home"] {
            let mut out = Vec::new();
            let args = crate::diff::DiffArgs {
                path: path.to_string(),
                json: true,
            };
            assert!(crate::diff::run(&paths, &args, &mut out).is_err());
            assert!(out.is_empty());
        }
        assert_eq!(hash_tree(root.path()).unwrap(), before);
    }
}
