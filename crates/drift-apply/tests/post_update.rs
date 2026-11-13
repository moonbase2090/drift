use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const FAKE_DRIFT: &str = r#"#!/bin/sh
set -eu
lock=${DRIFT_PACMAN_LOCK:-/var/lib/pacman/db.lck}
if [ -e "$lock" ]; then
  echo lock >&2
  exit 1
fi
printf '%s\n' "$*" >> "$DRIFT_NOTIFY_LOG"
"#;

fn repo(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn scratch(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "drift-post-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Reap(Option<Child>);

impl Drop for Reap {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn executable(path: &Path) {
    let mode = fs::metadata(path).unwrap().permissions().mode();
    assert_ne!(mode & 0o111, 0, "{}", path.display());
}

fn units_verify(paths: &[&str]) {
    let output = Command::new("systemd-analyze")
        .arg("verify")
        .args(paths)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let about_ours = stderr
        .lines()
        .any(|line| line.contains("drift-notify") || paths.iter().any(|path| line.contains(path)));
    assert!(output.status.success() && !about_ours, "{stderr}");
}

fn command_ok(program: &str, args: &[&str]) {
    let output = Command::new(program).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{program} {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{program}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_fake_drift(dir: &Path) {
    let path = dir.join("drift");
    fs::write(&path, FAKE_DRIFT).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
}

fn path_with(bin: &Path) -> std::ffi::OsString {
    let mut paths = vec![bin.to_path_buf()];
    if let Some(current) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&current));
    }
    std::env::join_paths(paths).unwrap()
}

fn stamp_ok(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 21
        && bytes[20] == b'\n'
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] == b'Z'
        && bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7 | 10 | 13 | 16 | 19 | 20) || byte.is_ascii_digit()
        })
}

fn names(dir: &Path) -> Vec<String> {
    let mut found = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    found.sort();
    found
}

fn child_is_waiting(pid: u32) -> bool {
    let Ok(status) = fs::read_to_string(format!("/proc/{pid}/status")) else {
        return false;
    };
    let sleeping = status.lines().any(|line| {
        line.strip_prefix("State:")
            .is_some_and(|rest| rest.trim_start().starts_with('S'))
    });
    if !sleeping {
        return false;
    }
    let Ok(children) = fs::read(format!("/proc/{pid}/task/{pid}/children")) else {
        return false;
    };
    children.iter().any(u8::is_ascii_digit)
}

#[test]
fn the_packaged_hook_matches_integrate_and_the_units_verify() {
    let hook = repo("packaging/hooks/post-update.d/drift");
    let mark = repo("packaging/drift-mark-updated");
    let waiter = repo("packaging/drift-notify-after-upgrade");
    let path_unit = repo("packaging/user/drift-notify.path");
    let service_unit = repo("packaging/user/drift-notify.service");
    assert_eq!(
        fs::read_to_string(&hook).unwrap(),
        "#!/bin/bash\ndrift notify\n"
    );
    executable(&hook);
    executable(&mark);
    executable(&waiter);
    command_ok("bash", &["-n", hook.to_str().unwrap()]);
    command_ok("sh", &["-n", mark.to_str().unwrap()]);
    command_ok("sh", &["-n", waiter.to_str().unwrap()]);

    let mark_text = fs::read_to_string(&mark).unwrap();
    assert!(mark_text.contains("file=${DRIFT_UPDATED_FILE:-/var/lib/drift/updated}\n"));
    assert!(mark_text.contains("tmp=$(mktemp \"$dir/.updated.XXXXXX\")\n"));
    assert!(mark_text.contains("mv -f \"$tmp\" \"$file\"\n"));

    let waiter_text = fs::read_to_string(&waiter).unwrap();
    assert!(waiter_text.contains("lock=${DRIFT_PACMAN_LOCK:-/var/lib/pacman/db.lck}\n"));
    assert!(waiter_text.contains("exec drift notify\n"));
    assert!(!waiter_text.contains("flock"));

    let path_text = fs::read_to_string(&path_unit).unwrap();
    assert!(path_text.contains("PathChanged=/var/lib/drift/updated\n"));
    assert!(path_text.contains("WantedBy=default.target\n"));
    assert!(!path_text.contains("ConditionPathExists"));
    assert!(!path_text.contains("AssertPathExists"));
    assert!(!path_text.contains("\nUnit="));

    let service_text = fs::read_to_string(&service_unit).unwrap();
    assert!(service_text.contains("Type=oneshot\n"));
    assert!(service_text.contains("ExecStart=/usr/lib/drift/drift-notify-after-upgrade\n"));

    let checked = TempDir(scratch("verify"));
    let exe = checked.0.join("drift-notify-after-upgrade");
    fs::copy(&waiter, &exe).unwrap();
    let mut permissions = fs::metadata(&exe).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&exe, permissions).unwrap();
    let rewritten = service_text.replace(
        "ExecStart=/usr/lib/drift/drift-notify-after-upgrade",
        &format!("ExecStart={}", exe.display()),
    );
    assert_eq!(rewritten.matches("ExecStart=").count(), 1);
    let copy_path = checked.0.join("drift-notify.path");
    let copy_service = checked.0.join("drift-notify.service");
    fs::write(&copy_path, &path_text).unwrap();
    fs::write(&copy_service, rewritten).unwrap();
    units_verify(&[copy_path.to_str().unwrap(), copy_service.to_str().unwrap()]);
}

#[test]
fn one_state_file_change_runs_notify_once() {
    let root = TempDir(scratch("once"));
    let bin = root.0.join("bin");
    fs::create_dir(&bin).unwrap();
    write_fake_drift(&bin);
    let log = root.0.join("log");
    let updated = root.0.join("state/updated");
    let lock = root.0.join("missing.lock");
    let status = Command::new("sh")
        .arg(repo("packaging/drift-mark-updated"))
        .env("DRIFT_UPDATED_FILE", &updated)
        .status()
        .unwrap();
    assert!(status.success());
    let body = fs::read_to_string(&updated).unwrap();
    assert!(stamp_ok(&body), "{body:?}");
    let meta = fs::symlink_metadata(&updated).unwrap();
    assert!(meta.file_type().is_file());
    assert_eq!(
        names(updated.parent().unwrap()),
        vec!["updated".to_string()]
    );
    let status = Command::new("sh")
        .arg(repo("packaging/drift-notify-after-upgrade"))
        .env("PATH", path_with(&bin))
        .env("DRIFT_NOTIFY_LOG", &log)
        .env("DRIFT_PACMAN_LOCK", &lock)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(fs::read_to_string(&log).unwrap(), "notify\n");
    assert!(!lock.exists());
}

#[test]
fn the_waiter_holds_while_the_pacman_lock_exists() {
    let root = TempDir(scratch("lock"));
    let bin = root.0.join("bin");
    fs::create_dir(&bin).unwrap();
    write_fake_drift(&bin);
    let log = root.0.join("log");
    let lock = root.0.join("db.lck");
    fs::write(&lock, b"").unwrap();
    let child = Command::new("sh")
        .arg(repo("packaging/drift-notify-after-upgrade"))
        .env("PATH", path_with(&bin))
        .env("DRIFT_NOTIFY_LOG", &log)
        .env("DRIFT_PACMAN_LOCK", &lock)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reap = Reap(Some(child));
    let pid = reap.0.as_ref().unwrap().id();
    loop {
        if child_is_waiting(pid) {
            break;
        }
        match reap.0.as_mut().unwrap().try_wait() {
            Ok(Some(status)) => panic!("notify waiter exited before it was waiting: {status}"),
            Ok(None) => continue,
            Err(err) => panic!("wait: {err}"),
        }
    }
    assert!(!log.exists());
    fs::remove_file(&lock).unwrap();
    let mut child = reap.0.take().unwrap();
    let status = child.wait().unwrap();
    let mut err = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut err)
        .unwrap();
    assert!(status.success(), "{err}");
    assert_eq!(fs::read_to_string(&log).unwrap(), "notify\n");
    assert!(!lock.exists());
}
