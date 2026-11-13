use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use drift_apply::edit_text;

static UMASK: Mutex<()> = Mutex::new(());
static NEXT: AtomicU64 = AtomicU64::new(0);

struct UmaskGuard {
    old: libc::mode_t,
    _lock: MutexGuard<'static, ()>,
}

impl UmaskGuard {
    fn new(mask: libc::mode_t) -> Self {
        let lock = UMASK.lock().unwrap_or_else(|error| error.into_inner());
        let old = unsafe { libc::umask(mask) };
        Self { old, _lock: lock }
    }
}

impl Drop for UmaskGuard {
    fn drop(&mut self) {
        unsafe {
            libc::umask(self.old);
        }
    }
}

fn scratch(label: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("drift-edit-sh-{}-{label}-{n}", std::process::id()))
}

fn script(body: &str) -> PathBuf {
    let path = scratch("bin");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o700)
        .open(&path)
        .unwrap();
    file.write_all(body.as_bytes()).unwrap();
    path
}

fn editor_files() -> Vec<PathBuf> {
    let prefix = format!("drift-edit-{}-", std::process::id());
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(std::env::temp_dir()) else {
        return found;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            found.push(entry.path());
        }
    }
    found.sort();
    found
}

#[test]
fn the_editor_file_is_private_under_a_permissive_umask() {
    let _umask = UmaskGuard::new(0o022);
    let note = scratch("mode");
    let program = script(&format!(
        "#!/bin/sh\nstat -c '%a %n' \"$1\" > '{}'\nprintf 'edited\\n' > \"$1\"\n",
        note.display()
    ));
    let text = edit_text("secret\n", program.as_os_str()).unwrap();
    let recorded = fs::read_to_string(&note).unwrap();
    let mut parts = recorded.split_whitespace();
    assert_eq!(parts.next(), Some("600"));
    let path = Path::new(parts.next().unwrap());
    assert!(!path.exists(), "{}", path.display());
    assert_eq!(text, "edited\n");
    let _ = fs::remove_file(&note);
    let _ = fs::remove_file(&program);
}

#[test]
fn a_failed_editor_removes_the_file() {
    let _lock = UMASK.lock().unwrap_or_else(|error| error.into_inner());
    let note = scratch("fail");
    let program = script(&format!(
        "#!/bin/sh\nprintf '%s\\n' \"$1\" > '{}'\nexit 1\n",
        note.display()
    ));
    let error = edit_text("secret\n", program.as_os_str()).unwrap_err();
    assert_eq!(error, "editor failed");
    let path = PathBuf::from(fs::read_to_string(&note).unwrap().trim());
    assert!(!path.exists(), "{}", path.display());
    let _ = fs::remove_file(&note);
    let _ = fs::remove_file(&program);
}

#[test]
fn a_missing_editor_removes_the_file() {
    let _lock = UMASK.lock().unwrap_or_else(|error| error.into_inner());
    let before = editor_files();
    let error = edit_text("secret\n", OsStr::new("/no/such/drift-editor")).unwrap_err();
    assert!(!error.is_empty());
    assert_eq!(editor_files(), before);
}
