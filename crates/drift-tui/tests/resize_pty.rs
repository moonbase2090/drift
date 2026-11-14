use std::env;
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const START: (u16, u16) = (100, 30);
const SMALL: (u16, u16) = (60, 20);
const LARGE: (u16, u16) = (120, 40);

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--host") {
        host();
        return;
    }
    for scenario in [
        Scenario {
            name: "tree",
            keys: b"",
            top: "Drift",
            bottom: "[Q] Quit",
        },
        Scenario {
            name: "diff",
            keys: b"\r",
            top: "Diff",
            bottom: "[Q] Quit",
        },
        Scenario {
            name: "history",
            keys: b"H",
            top: "History",
            bottom: "[Q] Back",
        },
        Scenario {
            name: "help",
            keys: b"?",
            top: "Help",
            bottom: "",
        },
        Scenario {
            name: "palette",
            keys: b":",
            top: "",
            bottom: "",
        },
    ] {
        run(&scenario);
    }
}

struct Scenario {
    name: &'static str,
    keys: &'static [u8],
    top: &'static str,
    bottom: &'static str,
}

struct NoResolver;

impl drift_tui::Resolver for NoResolver {
    fn propose(
        &mut self,
        _: drift_core::Action,
        _: &[String],
    ) -> Result<drift_tui::Proposal, String> {
        Err("not used".to_string())
    }

    fn propose_result(&mut self, _: &drift_tui::EditedFile) -> Result<drift_tui::Proposal, String> {
        Err("Writing is unavailable in this example.".to_string())
    }

    fn apply(&mut self, _: &drift_tui::Proposal) -> Result<String, String> {
        Err("not used".to_string())
    }
}

struct OneRow;

impl drift_tui::Historian for OneRow {
    fn rows(&mut self) -> Result<drift_core::HistoryView, String> {
        Ok(drift_core::HistoryView {
            rows: vec![drift_core::HistoryRow {
                id: "resolve-1".to_string(),
                time: "2026-11-12T10:30:00Z".to_string(),
                package_version: "4.5.0-3".to_string(),
                files: 1,
                actions: "1 take upstream".to_string(),
            }],
            unreadable: Vec::new(),
        })
    }

    fn propose(&mut self, _: &str, _: bool) -> Result<drift_tui::UndoProposal, String> {
        Err("not used".to_string())
    }

    fn apply(&mut self, _: &drift_tui::UndoProposal) -> Result<String, String> {
        Err("not used".to_string())
    }
}

struct SameDiff;

impl drift_tui::Differ for SameDiff {
    fn diff(&mut self, path: &str) -> Result<drift_core::FileDiff, String> {
        let text = |text: &str| drift_core::DiffContent::Text {
            sha256: "ab".repeat(32),
            text: text.to_string(),
        };
        Ok(drift_core::FileDiff {
            path: path.to_string(),
            status: drift_core::Status::Conflict,
            badge: None,
            provenance: drift_core::BaseProvenance::Reviewed,
            note: String::new(),
            base: text("same\n"),
            ours: text("left\n"),
            theirs: text("right\n"),
        })
    }
}

fn host() {
    let worker = drift_tui::spawn_scan(|| {
        Ok(drift_core::ScanReport {
            package_version: "4.5.0-3".to_string(),
            reviewed_version: None,
            entries: vec![drift_core::ScanEntry {
                path: "a.conf".to_string(),
                template: Some("config/a.conf".to_string()),
                status: drift_core::Status::Conflict,
                badge: None,
                base: drift_core::BaseProvenance::Reviewed,
                volatile: false,
                note: String::new(),
            }],
            backups: Vec::new(),
            ignored: 0,
        })
    });
    drift_tui::launch_worker_with(
        worker,
        Box::new(NoResolver),
        Box::new(OneRow),
        Box::new(SameDiff),
    )
    .unwrap();
}

fn set_size(fd: i32, size: (u16, u16)) {
    let winsize = libc::winsize {
        ws_row: size.1,
        ws_col: size.0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    unsafe {
        libc::ioctl(fd, libc::TIOCSWINSZ, &winsize);
    }
}

fn rows(parser: &Mutex<vt100::Parser>) -> Vec<String> {
    let guard = parser.lock().unwrap_or_else(|err| err.into_inner());
    let screen = guard.screen();
    let (_, columns) = screen.size();
    screen.rows(0, columns).collect()
}

fn wait_for(parser: &Mutex<vt100::Parser>, ready: impl Fn(&[String]) -> bool) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        let found = rows(parser);
        if ready(&found) || Instant::now() >= deadline {
            return found;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}

fn run(scenario: &Scenario) {
    let mut master = -1;
    let mut slave = -1;
    let opened = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(opened, 0);
    set_size(master, START);
    let master = unsafe { File::from_raw_fd(master) };
    let slave = unsafe { File::from_raw_fd(slave) };
    let stdin = unsafe { libc::dup(slave.as_raw_fd()) };
    let stdout = unsafe { libc::dup(slave.as_raw_fd()) };
    drop(slave);
    let mut child = Command::new(env::current_exe().unwrap())
        .arg("--host")
        .stdin(unsafe { Stdio::from_raw_fd(stdin) })
        .stdout(unsafe { Stdio::from_raw_fd(stdout) })
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let parser = Arc::new(Mutex::new(vt100::Parser::new(START.1, START.0, 0)));
    let feed = Arc::clone(&parser);
    let mut reader = master.try_clone().unwrap();
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        while let Ok(size) = reader.read(&mut buffer) {
            if size == 0 {
                break;
            }
            feed.lock()
                .unwrap_or_else(|err| err.into_inner())
                .process(&buffer[..size]);
        }
    });
    let name = scenario.name;
    wait_for(&parser, |found| {
        found.iter().any(|row| row.contains("a.conf"))
    });
    if !scenario.keys.is_empty() {
        use std::io::Write;
        let mut writer = master.try_clone().unwrap();
        writer.write_all(scenario.keys).unwrap();
        writer.flush().unwrap();
        wait_for(&parser, |found| {
            found.first().is_some_and(|row| row.contains(scenario.top)) || scenario.top.is_empty()
        });
    }
    let mut failures = Vec::new();
    for size in [SMALL, LARGE, START] {
        set_size(master.as_raw_fd(), size);
        parser
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .screen_mut()
            .set_size(size.1, size.0);
        let found = wait_for(&parser, |found| {
            let last = usize::from(size.1) - 1;
            (scenario.bottom.is_empty() || found[last].contains(scenario.bottom))
                && (scenario.top.is_empty() || found[0].contains(scenario.top))
                && found
                    .iter()
                    .any(|row| row.chars().count() > usize::from(size.0) * 2 / 3)
        });
        let last = usize::from(size.1) - 1;
        if !scenario.bottom.is_empty() && !found[last].contains(scenario.bottom) {
            failures.push(format!(
                "{name} {size:?}: bottom row {:?}\n{}",
                found[last],
                found.join("\n")
            ));
        }
        if !scenario.top.is_empty() && !found[0].contains(scenario.top) {
            failures.push(format!("{name} {size:?}: top row {:?}", found[0]));
        }
        if scenario.bottom.is_empty() && scenario.top.is_empty() {
            let widest = found
                .iter()
                .map(|row| row.chars().count())
                .max()
                .unwrap_or(0);
            if widest <= usize::from(size.0) * 2 / 3 {
                failures.push(format!("{name} {size:?}: nothing filled the new width"));
            }
        }
    }
    unsafe { libc::kill(child.id() as i32, libc::SIGKILL) };
    let _ = child.wait();
    assert!(failures.is_empty(), "{}", failures.join("\n---\n"));
}
