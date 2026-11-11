mod checks;
mod lex;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives one level below the workspace root")
        .to_path_buf()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn write_check(root: &Path) -> usize {
    let crates = root.join("crates");
    let Ok(entries) = fs::read_dir(&crates) else {
        eprintln!("write-check: no crates directory at {}", crates.display());
        return 1;
    };
    let mut failures = 0;
    let mut checked = 0;
    let mut dirs: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    dirs.sort();
    for dir in dirs.iter().filter(|d| d.is_dir()) {
        let name = dir.file_name().map(|n| n.to_string_lossy().to_string());
        if name.as_deref() == Some(checks::WRITER_CRATE) {
            continue;
        }
        let mut files = Vec::new();
        rust_files(dir, &mut files);
        for file in files {
            checked += 1;
            let Ok(source) = fs::read_to_string(&file) else {
                eprintln!("write-check: cannot read {}", relative(root, &file));
                failures += 1;
                continue;
            };
            for v in checks::write_violations(&source) {
                eprintln!(
                    "{}:{}: filesystem-mutating API `{}` outside {}",
                    relative(root, &file),
                    v.line,
                    v.what,
                    checks::WRITER_CRATE
                );
                failures += 1;
            }
        }
    }
    println!("write-check: {checked} files checked, {failures} violations");
    failures
}

fn no_comment(root: &Path) -> usize {
    let mut files = Vec::new();
    rust_files(&root.join("crates"), &mut files);
    rust_files(&root.join("xtask"), &mut files);
    let mut failures = 0;
    let checked = files.len();
    for file in files {
        let Ok(source) = fs::read_to_string(&file) else {
            eprintln!("no-comment: cannot read {}", relative(root, &file));
            failures += 1;
            continue;
        };
        for line in checks::comment_violations(&source) {
            eprintln!("{}:{line}: comment", relative(root, &file));
            failures += 1;
        }
    }
    println!("no-comment: {checked} files checked, {failures} violations");
    failures
}

fn main() -> ExitCode {
    let root = workspace_root();
    let command = std::env::args().nth(1);
    let failures = match command.as_deref() {
        Some("write-check") => write_check(&root),
        Some("no-comment") => no_comment(&root),
        Some("check") => write_check(&root) + no_comment(&root),
        _ => {
            eprintln!("Usage: cargo xtask <write-check|no-comment|check>");
            return ExitCode::from(2);
        }
    };
    if failures == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
