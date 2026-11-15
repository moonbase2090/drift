use std::fs::{self, File};
use std::os::fd::AsRawFd;

use crate::system::{System, MISSING, UNCHANGED, UPSTREAM};

const RECORD: &str = "#!/bin/sh\n{ printf 'BEGIN\\n'; printf '%s\\n' \"$@\"; printf 'END\\n'; } >> \"$DRIFT_ROOT/notices.log\"\n";

fn logs(system: &System) -> String {
    fs::read_to_string(system.root().join("notices.log")).unwrap_or_default()
}

fn run(system: &System, args: &[&str]) -> (i32, String) {
    let mut session = system.launch(args, (160, 36));
    let code = session.wait_exit();
    (code, session.screen().text())
}

fn sent(headline: &str) -> String {
    format!("BEGIN\n--app-name\ndrift\n-u\nnormal\n{headline}\n--exec\nomarchy-launch-or-focus-tui\n--app-id=TUI.float\ndrift\nEND\n")
}

#[test]
fn notify_delivers_singular_and_plural_counts_with_the_click_command() {
    for (system, headline) in [
        (
            System::builder().template(&UPSTREAM).build(),
            "1 config drifted",
        ),
        (
            System::builder()
                .template(&UPSTREAM)
                .template(&MISSING)
                .build(),
            "2 configs drifted",
        ),
    ] {
        system.write("bin/omarchy-notification-send", RECORD);
        let (code, text) = run(&system, &["notify"]);
        assert_eq!(code, 0, "{text}");
        assert_eq!(logs(&system), sent(headline));
    }
}

#[test]
fn notify_on_and_off_persist_across_processes_and_control_delivery() {
    let system = System::builder().template(&UPSTREAM).build();
    system.write("bin/omarchy-notification-send", RECORD);
    let setting = system.home().join(".local/state/drift/notifications");
    let (code, text) = run(&system, &["notify", "off"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("notifications off"));
    assert_eq!(fs::read(&setting).unwrap(), b"off\n");
    assert_eq!(run(&system, &["notify"]).0, 0);
    assert_eq!(logs(&system), "");
    let (code, text) = run(&system, &["notify", "on"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("notifications on"));
    assert_eq!(fs::read(&setting).unwrap(), b"on\n");
    assert_eq!(run(&system, &["notify"]).0, 0);
    assert_eq!(logs(&system), sent("1 config drifted"));
}

#[test]
fn notify_skips_zero_drift_and_real_update_locks_but_allows_unlocked_leftovers() {
    let quiet = System::builder().template(&UNCHANGED).build();
    quiet.write("bin/omarchy-notification-send", RECORD);
    assert_eq!(run(&quiet, &["notify"]).0, 0);
    assert_eq!(logs(&quiet), "");
    let system = System::builder().template(&UPSTREAM).build();
    system.write("bin/omarchy-notification-send", RECORD);
    system.put("var/lib/pacman/db.lck", b"");
    assert_eq!(run(&system, &["notify"]).0, 0);
    assert_eq!(logs(&system), "");
    system.discard_file("var/lib/pacman/db.lck");
    let path = system.root().join("runtime/omarchy-update.lock");
    system.put("runtime/omarchy-update.lock", b"persistent bytes");
    let holder = File::open(&path).unwrap();
    assert_eq!(
        unsafe { libc::flock(holder.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    assert_eq!(run(&system, &["notify"]).0, 0);
    assert_eq!(logs(&system), "");
    drop(holder);
    assert_eq!(run(&system, &["notify"]).0, 0);
    assert_eq!(logs(&system), sent("1 config drifted"));
    assert_eq!(fs::read(path).unwrap(), b"persistent bytes");
}

#[test]
fn a_failed_notifier_is_reported_and_exits_two() {
    let system = System::builder().template(&UPSTREAM).build();
    system.write(
        "bin/omarchy-notification-send",
        format!("{RECORD}exit 17\n"),
    );
    let (code, text) = run(&system, &["notify"]);
    assert_eq!(code, 2, "{text}");
    assert!(
        text.contains("notifier exited") && text.contains("17"),
        "{text}"
    );
    assert_eq!(logs(&system), sent("1 config drifted"));
}
