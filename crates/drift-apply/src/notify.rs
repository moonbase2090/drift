use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::Command;

pub const APP_NAME: &str = "drift";
pub const URGENCY: &str = "normal";
pub const CLICK: &[&str] = &["omarchy-launch-or-focus-tui", "--app-id=TUI.float", "drift"];
const SETTING: &str = "notifications";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub headline: String,
    pub urgency: &'static str,
    pub click: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyError {
    message: String,
}

impl std::fmt::Display for NotifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for NotifyError {}

impl NotifyError {
    fn message(text: impl Into<String>) -> Self {
        Self {
            message: text.into(),
        }
    }

    fn io(path: &Path, source: std::io::Error) -> Self {
        Self::message(format!("{}: {source}", path.display()))
    }
}

pub fn notice_for(count: usize) -> Option<Notice> {
    let headline = match count {
        0 => return None,
        1 => "1 config drifted".to_string(),
        n => format!("{n} configs drifted"),
    };
    Some(Notice {
        headline,
        urgency: URGENCY,
        click: CLICK,
    })
}

pub fn should_send(enabled: bool, updating: bool, count: usize) -> Option<Notice> {
    if updating || !enabled {
        return None;
    }
    notice_for(count)
}

pub fn notifications_enabled(state: &Path) -> Result<bool, NotifyError> {
    if symlink_at(state)? {
        return Err(NotifyError::message("state path contains a symlink"));
    }
    let path = state.join(SETTING);
    match fs::symlink_metadata(&path) {
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(source) => Err(NotifyError::io(&path, source)),
        Ok(meta) if meta.file_type().is_symlink() => {
            Err(NotifyError::message("notifications setting is a symlink"))
        }
        Ok(meta) if !meta.is_file() => {
            Err(NotifyError::message("notifications setting is not a file"))
        }
        Ok(_) => {
            match fs::read_to_string(&path).map_err(|source| NotifyError::io(&path, source))? {
                text if text.trim() == "on" => Ok(true),
                text if text.trim() == "off" => Ok(false),
                _ => Err(NotifyError::message(
                    "notifications setting is not on or off",
                )),
            }
        }
    }
}

pub fn set_notifications(state: &Path, enabled: bool) -> Result<(), NotifyError> {
    ensure_state_dir(state)?;
    let dest = state.join(SETTING);
    if symlink_at(&dest)? {
        return Err(NotifyError::message("notifications setting is a symlink"));
    }
    let temp = state.join(".drift-notify.tmp");
    if symlink_at(&temp)? {
        return Err(NotifyError::message("notifications temp path is a symlink"));
    }
    let _ = fs::remove_file(&temp);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|source| NotifyError::io(&temp, source))?;
    let text = if enabled { "on\n" } else { "off\n" };
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|source| NotifyError::io(&temp, source))?;
    fs::rename(&temp, &dest).map_err(|source| NotifyError::io(&dest, source))?;
    Ok(())
}

pub fn update_running(omarchy_lock: &Path, pacman_lock: &Path) -> bool {
    let inherited = std::env::var("OMARCHY_UPDATE_LOCK_FD")
        .ok()
        .and_then(|text| text.parse().ok());
    update_running_from(omarchy_lock, pacman_lock, inherited)
}

fn update_running_from(omarchy_lock: &Path, pacman_lock: &Path, inherited: Option<i32>) -> bool {
    if inherited_lock(omarchy_lock, inherited) {
        return false;
    }
    foreign_lock(omarchy_lock) || foreign_lock(pacman_lock)
}

fn inherited_lock(lock: &Path, inherited: Option<i32>) -> bool {
    let Some(fd) = inherited else {
        return false;
    };
    if fd < 0 {
        return false;
    }
    let opened = match fs::canonicalize(format!("/proc/self/fd/{fd}")) {
        Ok(path) => path,
        Err(_) => return false,
    };
    let expected = match fs::canonicalize(lock) {
        Ok(path) => path,
        Err(_) => return false,
    };
    if opened != expected {
        return false;
    }
    unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) == 0 }
}

fn foreign_lock(path: &Path) -> bool {
    let file = match OpenOptions::new().read(true).open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let held = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if held == 0 {
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
        return false;
    }
    std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock
}

pub fn send(program: &Path, notice: &Notice) -> Result<(), NotifyError> {
    let mut command = Command::new(program);
    command.arg("--app-name").arg(APP_NAME);
    command.arg("-u").arg(notice.urgency);
    command.arg(&notice.headline);
    command.arg("--exec");
    command.args(notice.click);
    match command.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(NotifyError::message(format!("notifier exited {status}"))),
        Err(source) => Err(NotifyError::message(format!(
            "{}: {source}",
            program.display()
        ))),
    }
}

fn ensure_state_dir(state: &Path) -> Result<(), NotifyError> {
    let mut cursor = state.to_path_buf();
    let mut missing = Vec::new();
    loop {
        match fs::symlink_metadata(&cursor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(NotifyError::message("state path contains a symlink"));
            }
            Ok(meta) if meta.is_dir() => break,
            Ok(_) => return Err(NotifyError::message("state path is not a directory")),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                let Some(parent) = cursor.parent().map(Path::to_path_buf) else {
                    return Err(NotifyError::message("state path has no parent"));
                };
                if parent == cursor {
                    return Err(NotifyError::io(&cursor, source));
                }
                missing.push(cursor);
                cursor = parent;
            }
            Err(source) => return Err(NotifyError::io(&cursor, source)),
        }
    }
    for dir in missing.iter().rev() {
        fs::create_dir(dir).map_err(|source| NotifyError::io(dir, source))?;
    }
    Ok(())
}

fn symlink_at(path: &Path) -> Result<bool, NotifyError> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(meta.file_type().is_symlink()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(NotifyError::io(path, source)),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::CommandExt;

    use super::*;

    #[test]
    fn the_text_names_one_config_or_many() {
        assert!(notice_for(0).is_none());
        let one = notice_for(1).unwrap();
        assert_eq!(one.headline, "1 config drifted");
        assert_eq!(one.urgency, "normal");
        assert_eq!(
            one.click,
            ["omarchy-launch-or-focus-tui", "--app-id=TUI.float", "drift"]
        );
        assert_eq!(notice_for(3).unwrap().headline, "3 configs drifted");
    }

    #[test]
    fn nothing_is_sent_for_zero_drift_a_disable_or_an_update() {
        assert!(should_send(true, false, 0).is_none());
        assert!(should_send(false, false, 4).is_none());
        assert!(should_send(true, true, 4).is_none());
        assert_eq!(
            should_send(true, false, 4).unwrap().headline,
            "4 configs drifted"
        );
    }

    #[test]
    fn the_toggle_defaults_on_and_persists() {
        let root = drift_fixtures::RootBuilder::new().materialize().unwrap();
        let state = root.path().join("state");
        assert!(notifications_enabled(&state).unwrap());
        set_notifications(&state, false).unwrap();
        assert!(!notifications_enabled(&state).unwrap());
        assert_eq!(fs::read_to_string(state.join(SETTING)).unwrap(), "off\n");
        set_notifications(&state, true).unwrap();
        assert!(notifications_enabled(&state).unwrap());
        assert_eq!(fs::read_to_string(state.join(SETTING)).unwrap(), "on\n");
    }

    #[test]
    fn a_linked_setting_is_refused() {
        let root = drift_fixtures::RootBuilder::new()
            .file("secret", b"keep\n")
            .unwrap()
            .link("state/notifications", "../secret")
            .unwrap()
            .materialize()
            .unwrap();
        let state = root.path().join("state");
        let error = set_notifications(&state, false).unwrap_err();
        assert!(error.to_string().contains("symlink"), "{error}");
        assert_eq!(
            fs::read_to_string(root.path().join("secret")).unwrap(),
            "keep\n"
        );
        assert!(notifications_enabled(&state)
            .unwrap_err()
            .to_string()
            .contains("symlink"));
    }

    #[test]
    fn a_fake_notifier_records_the_text_urgency_and_click_command() {
        let script = "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$(dirname \"$0\")/args\"\n";
        let root = drift_fixtures::RootBuilder::new()
            .file_with_mode("bin/notify", script, 0o755)
            .unwrap()
            .materialize()
            .unwrap();
        let notice = notice_for(2).unwrap();
        send(&root.path().join("bin/notify"), &notice).unwrap();
        let recorded = fs::read_to_string(root.path().join("bin/args")).unwrap();
        let mut expected = format!(
            "--app-name\n{APP_NAME}\n-u\n{}\n{}\n--exec\n",
            notice.urgency, notice.headline
        );
        for word in notice.click {
            expected.push_str(word);
            expected.push('\n');
        }
        assert_eq!(recorded, expected);
        assert_eq!(notice.headline, "2 configs drifted");
    }

    #[test]
    fn a_lock_file_blocks_only_while_another_process_holds_it() {
        let root = drift_fixtures::RootBuilder::new()
            .file("run/omarchy-update.lock", b"")
            .unwrap()
            .file("db.lck", b"")
            .unwrap()
            .materialize()
            .unwrap();
        let omarchy = root.path().join("run/omarchy-update.lock");
        let pacman = root.path().join("db.lck");
        let absent = root.path().join("missing.lock");
        assert!(!update_running_from(&omarchy, &absent, None));
        assert!(!update_running_from(&absent, &pacman, None));
        assert!(!update_running_from(&absent, &absent, None));
        assert!(!update_running_from(&omarchy, &pacman, Some(-1)));
        assert!(!update_running_from(&omarchy, &pacman, Some(1_000_000)));

        {
            let _held = hold(&omarchy);
            assert!(update_running_from(&omarchy, &absent, None));
            assert!(!update_running_from(&absent, &pacman, None));
        }
        assert!(!update_running_from(&omarchy, &absent, None));

        {
            let _held = hold(&pacman);
            assert!(update_running_from(&absent, &pacman, None));
            assert!(update_running_from(&omarchy, &pacman, None));
        }
        assert!(!update_running_from(&absent, &pacman, None));
    }

    #[test]
    fn the_post_update_hook_sends_while_its_lock_is_held() {
        let root = drift_fixtures::RootBuilder::new()
            .file("run/omarchy-update.lock", b"")
            .unwrap()
            .file("db.lck", b"")
            .unwrap()
            .materialize()
            .unwrap();
        let omarchy = root.path().join("run/omarchy-update.lock");
        let pacman = root.path().join("db.lck");
        let hook = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&omarchy)
            .unwrap();
        let fd = hook.as_raw_fd();
        assert_eq!(unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) }, 0);
        assert!(!update_running_from(&omarchy, &pacman, Some(fd)));
        {
            let _held = hold(&pacman);
            assert!(!update_running_from(&omarchy, &pacman, Some(fd)));
            assert!(update_running_from(&omarchy, &pacman, None));
        }
        drop(hook);
        let other = OpenOptions::new().read(true).open(&pacman).unwrap();
        {
            let _held = hold(&omarchy);
            assert!(update_running_from(
                &omarchy,
                &pacman,
                Some(other.as_raw_fd())
            ));
        }
        assert!(!update_running_from(&omarchy, &pacman, None));
        let inherited = OpenOptions::new().read(true).open(&omarchy).unwrap();
        assert!(!update_running_from(
            &omarchy,
            &pacman,
            Some(inherited.as_raw_fd())
        ));
        drop(inherited);
        assert!(!update_running_from(&omarchy, &pacman, None));
    }

    struct Held(std::process::Child);

    impl Held {
        fn at(path: &Path) -> Self {
            let mut child = Command::new("flock")
                .arg("-F")
                .arg(path)
                .arg("sh")
                .arg("-c")
                .arg("printf 'ready\\n'; exec sleep 30")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .process_group(0)
                .spawn()
                .unwrap();
            let mut line = String::new();
            std::io::BufRead::read_line(
                &mut std::io::BufReader::new(child.stdout.as_mut().unwrap()),
                &mut line,
            )
            .unwrap();
            assert_eq!(line, "ready\n");
            Self(child)
        }
    }

    impl Drop for Held {
        fn drop(&mut self) {
            let pid = self.0.id() as libc::pid_t;
            unsafe { libc::kill(-pid, libc::SIGKILL) };
            let _ = self.0.wait();
        }
    }

    fn hold(path: &Path) -> Held {
        Held::at(path)
    }
}
