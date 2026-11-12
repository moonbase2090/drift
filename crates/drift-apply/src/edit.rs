use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use std::os::unix::fs::OpenOptionsExt;

pub fn edit_text(text: &str, program: &OsStr) -> Result<String, String> {
    let (mut file, path) = create_private()?;
    let _cleanup = EditorFile(path.clone());
    file.write_all(text.as_bytes())
        .map_err(|error| error.to_string())?;
    file.flush().map_err(|error| error.to_string())?;
    drop(file);
    let status = std::process::Command::new(program)
        .arg(&path)
        .status()
        .map_err(|error| error.to_string());
    let edited = fs::read_to_string(&path).map_err(|error| error.to_string());
    if !status?.success() {
        return Err(String::from("editor failed"));
    }
    edited
}

struct EditorFile(PathBuf);

impl Drop for EditorFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn create_private() -> Result<(std::fs::File, PathBuf), String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    for _ in 0..32 {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let mut path = std::env::temp_dir();
        path.push(format!("drift-edit-{}-{}-{n}", std::process::id(), nanos));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => return Ok((file, path)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err(String::from("could not create the editor file"))
}
