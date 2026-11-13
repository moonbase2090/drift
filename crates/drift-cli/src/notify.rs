use std::io::Write;
use std::path::Path;

use drift_apply::{send, set_notifications, should_send, update_running, Notice};
use drift_source::ScanPaths;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    Send,
    Enable,
    Disable,
}

pub fn parse(args: &[String]) -> Result<Request, String> {
    match args {
        [] => Ok(Request::Send),
        [word] if word == "on" => Ok(Request::Enable),
        [word] if word == "off" => Ok(Request::Disable),
        _ => Err("notify accepts no arguments, or on, or off".to_string()),
    }
}

pub fn run(
    paths: &ScanPaths,
    state: &Path,
    omarchy_lock: &Path,
    pacman_lock: &Path,
    program: &Path,
    request: Request,
    out: &mut dyn Write,
) -> Result<u8, String> {
    run_with(
        paths,
        state,
        omarchy_lock,
        pacman_lock,
        request,
        out,
        |notice| send(program, notice).map_err(|error| error.to_string()),
    )
}

pub fn run_with(
    paths: &ScanPaths,
    state: &Path,
    omarchy_lock: &Path,
    pacman_lock: &Path,
    request: Request,
    out: &mut dyn Write,
    mut deliver: impl FnMut(&Notice) -> Result<(), String>,
) -> Result<u8, String> {
    match request {
        Request::Enable => store(state, true, out),
        Request::Disable => store(state, false, out),
        Request::Send => {
            if update_running(omarchy_lock, pacman_lock) {
                return Ok(0);
            }
            let enabled =
                drift_apply::notifications_enabled(state).map_err(|error| error.to_string())?;
            if !enabled {
                return Ok(0);
            }
            let report = crate::scan::build(paths)?;
            if let Some(notice) = should_send(true, false, report.drifted()) {
                deliver(&notice)?;
            }
            Ok(0)
        }
    }
}

fn store(state: &Path, enabled: bool, out: &mut dyn Write) -> Result<u8, String> {
    set_notifications(state, enabled).map_err(|error| error.to_string())?;
    let word = if enabled { "on" } else { "off" };
    out.write_all(format!("notifications {word}\n").as_bytes())
        .and_then(|()| out.flush())
        .map_err(|error| error.to_string())?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use drift_fixtures::RootBuilder;
    use drift_source::SourcePaths;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::scan::paths_from;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|item| item.to_string()).collect()
    }

    fn digest(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn world(
        files: &[(&str, &[u8], &[u8])],
        extra: &[(&str, &str)],
    ) -> drift_fixtures::MaterializedRoot {
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

    fn paths(root: &drift_fixtures::MaterializedRoot) -> ScanPaths {
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

    fn drive(
        root: &drift_fixtures::MaterializedRoot,
        request: Request,
    ) -> (Result<u8, String>, String, Vec<Notice>) {
        let scanned = paths(root);
        let absent = root.path().join("absent.lock");
        let mut out = Vec::new();
        let mut sent = Vec::new();
        let code = run_with(
            &scanned,
            &scanned.resolve.state,
            &absent,
            &absent,
            request,
            &mut out,
            |notice| {
                sent.push(notice.clone());
                Ok(())
            },
        );
        (code, String::from_utf8(out).unwrap(), sent)
    }

    #[test]
    fn on_and_off_are_the_only_arguments() {
        assert_eq!(parse(&[]).unwrap(), Request::Send);
        assert_eq!(parse(&args(&["on"])).unwrap(), Request::Enable);
        assert_eq!(parse(&args(&["off"])).unwrap(), Request::Disable);
        assert!(parse(&args(&["later"])).is_err());
        assert!(parse(&args(&["on", "off"])).is_err());
    }

    #[test]
    fn a_drifted_scan_sends_the_count_and_zero_sends_nothing() {
        let drifted = world(
            &[
                ("hypr/a.lua", b"new\n", b""),
                ("hypr/b.lua", b"same\n", b"same\n"),
            ],
            &[],
        );
        let (code, out, sent) = drive(&drifted, Request::Send);
        assert_eq!(code.unwrap(), 0);
        assert!(out.is_empty(), "{out}");
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].headline, "1 config drifted");
        assert_eq!(sent[0].urgency, "normal");
        assert_eq!(
            sent[0].click,
            ["omarchy-launch-or-focus-tui", "--app-id=TUI.float", "drift"]
        );

        let clean = world(&[("hypr/a.lua", b"same\n", b"same\n")], &[]);
        let (code, _, sent) = drive(&clean, Request::Send);
        assert_eq!(code.unwrap(), 0);
        assert!(sent.is_empty());
    }

    #[test]
    fn an_update_lock_or_a_saved_off_sends_nothing() {
        let drifted = world(
            &[("hypr/a.lua", b"new\n", b"")],
            &[("run/omarchy-update.lock", "")],
        );
        let scanned = paths(&drifted);
        let mut sent = Vec::new();
        let code = run_with(
            &scanned,
            &scanned.resolve.state,
            &drifted.path().join("run/omarchy-update.lock"),
            &drifted.path().join("absent.lck"),
            Request::Send,
            &mut Vec::new(),
            |notice| {
                sent.push(notice.headline.clone());
                Ok(())
            },
        );
        assert_eq!(code.unwrap(), 0);
        assert!(sent.is_empty());

        let again = world(&[("hypr/a.lua", b"new\n", b"")], &[]);
        let (code, out, _) = drive(&again, Request::Disable);
        assert_eq!(code.unwrap(), 0);
        assert_eq!(out, "notifications off\n");
        let (code, _, sent) = drive(&again, Request::Send);
        assert_eq!(code.unwrap(), 0);
        assert!(sent.is_empty());
        let stored =
            std::fs::read_to_string(paths(&again).resolve.state.join("notifications")).unwrap();
        assert_eq!(stored, "off\n");
        let (code, out, _) = drive(&again, Request::Enable);
        assert_eq!(code.unwrap(), 0);
        assert_eq!(out, "notifications on\n");
        let (code, _, sent) = drive(&again, Request::Send);
        assert_eq!(code.unwrap(), 0);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].headline, "1 config drifted");
    }
}
