use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn edit_text(text: &str, program: &OsStr) -> Result<String, String> {
    let path = temp_path()?;
    fs::write(&path, text).map_err(|error| error.to_string())?;
    let status = Command::new(program)
        .arg(&path)
        .status()
        .map_err(|error| error.to_string());
    let edited = fs::read_to_string(&path).map_err(|error| error.to_string());
    let _ = fs::remove_file(&path);
    if !status?.success() {
        return Err(String::from("editor failed"));
    }
    edited
}

fn temp_path() -> Result<PathBuf, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let mut path = std::env::temp_dir();
    path.push(format!("drift-edit-{}-{nanos}", std::process::id()));
    Ok(path)
}
