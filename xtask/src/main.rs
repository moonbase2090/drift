mod checks;
mod imports;
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
        if name
            .as_deref()
            .is_some_and(|n| checks::WRITER_CRATES.contains(&n))
        {
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
                    checks::WRITER_CRATES.join(" and ")
                );
                failures += 1;
            }
        }
    }
    println!("write-check: {checked} files checked, {failures} violations");
    failures
}

fn manifests(root: &Path) -> Vec<PathBuf> {
    let mut manifests = vec![root.join("xtask").join("Cargo.toml")];
    if let Ok(entries) = fs::read_dir(root.join("crates")) {
        let mut dirs: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        dirs.sort();
        manifests.extend(dirs.into_iter().map(|d| d.join("Cargo.toml")));
    }
    manifests.into_iter().filter(|m| m.is_file()).collect()
}

fn manifest_check(root: &Path, name: &str, inspect: impl Fn(&str) -> Vec<String>) -> usize {
    let mut failures = 0;
    let all = manifests(root);
    for manifest in &all {
        let Ok(text) = fs::read_to_string(manifest) else {
            eprintln!("{name}: cannot read {}", relative(root, manifest));
            failures += 1;
            continue;
        };
        for message in inspect(&text) {
            eprintln!("{}: {message}", relative(root, manifest));
            failures += 1;
        }
    }
    println!(
        "{name}: {} manifests checked, {failures} violations",
        all.len()
    );
    failures
}

fn fixtures_dependency(root: &Path) -> usize {
    manifest_check(
        root,
        "fixtures-dependency",
        checks::fixtures_dependency_violations,
    )
}

fn feature_policy(root: &Path) -> usize {
    manifest_check(root, "feature-policy", checks::feature_violations)
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

fn text_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if path.is_dir() {
            if name != "target" && name != ".git" {
                text_files(&path, out);
            }
        } else if path.is_file() {
            out.push(path);
        }
    }
}

fn identifiers(root: &Path) -> usize {
    let mut files = Vec::new();
    text_files(root, &mut files);
    let mut failures = 0;
    let mut checked = 0;
    for file in files {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        checked += 1;
        for item in checks::identifier_violations(&text) {
            eprintln!(
                "{}:{}: tracker identifier or personal address `{}`",
                relative(root, &file),
                item.line,
                item.what
            );
            failures += 1;
        }
    }
    println!("identifiers: {checked} files checked, {failures} violations");
    failures
}

fn main() -> ExitCode {
    let root = workspace_root();
    let command = std::env::args().nth(1);
    let failures = match command.as_deref() {
        Some("write-check") => write_check(&root),
        Some("no-comment") => no_comment(&root),
        Some("fixtures-dependency") => fixtures_dependency(&root),
        Some("feature-policy") => feature_policy(&root),
        Some("identifiers") => identifiers(&root),
        Some("check") => {
            write_check(&root)
                + no_comment(&root)
                + fixtures_dependency(&root)
                + feature_policy(&root)
                + identifiers(&root)
        }
        _ => {
            eprintln!("Usage: cargo xtask <write-check|no-comment|fixtures-dependency|feature-policy|identifiers|check>");
            return ExitCode::from(2);
        }
    };
    if failures == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xtask-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_identifier_walk_reads_text_files_and_skips_target_git_and_binaries() {
        let root = scratch("walk");
        let ticket = format!("see D{}", "R-5");
        for (path, bytes) in [
            ("docs/a.md", ticket.clone().into_bytes()),
            ("target/b.md", ticket.clone().into_bytes()),
            (".git/c", ticket.clone().into_bytes()),
            ("bin.dat", vec![0xff, 0xfe, 0x00]),
            ("clean.md", b"nothing here".to_vec()),
        ] {
            let file = root.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, bytes).unwrap();
        }
        let mut files = Vec::new();
        text_files(&root, &mut files);
        let names: Vec<String> = files.iter().map(|file| relative(&root, file)).collect();
        assert_eq!(names, ["bin.dat", "clean.md", "docs/a.md"]);
        assert_eq!(identifiers(&root), 1);
        let _ = fs::remove_dir_all(&root);
    }
}
