use crate::lex::scan;

pub const WRITER_CRATES: &[&str] = &["drift-apply", "drift-fixtures"];
pub const TEST_ONLY_CRATE: &str = "drift-fixtures";

const BANNED_PATHS: &[&str] = &[
    "fs::write",
    "fs::rename",
    "fs::copy",
    "fs::File::create",
    "File::create",
];

const BANNED_WORDS: &[&str] = &[
    "remove_file",
    "remove_dir",
    "remove_dir_all",
    "create_dir",
    "create_dir_all",
    "hard_link",
    "soft_link",
    "symlink",
    "symlink_file",
    "symlink_dir",
    "set_permissions",
    "set_len",
    "set_modified",
    "OpenOptions",
    "create_new",
    "tempfile",
];

const GROUPED_BANNED: &[&str] = &["write", "rename", "copy", "File", "create"];

pub struct Violation {
    pub line: usize,
    pub what: String,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn find_word(haystack: &str, word: &str) -> Vec<usize> {
    let mut hits = Vec::new();
    let mut from = 0;
    while let Some(pos) = haystack[from..].find(word) {
        let start = from + pos;
        let end = start + word.len();
        let before_ok = haystack[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !is_word(c));
        let after_ok = haystack[end..].chars().next().is_none_or(|c| !is_word(c));
        if before_ok && after_ok {
            hits.push(start);
        }
        from = end;
    }
    hits
}

fn line_of(code: &str, offset: usize) -> usize {
    code[..offset].matches('\n').count() + 1
}

pub fn write_violations(source: &str) -> Vec<Violation> {
    let code = scan(source).code;
    let mut found = Vec::new();

    for path in BANNED_PATHS {
        for pos in find_word(&code, path) {
            found.push(Violation {
                line: line_of(&code, pos),
                what: (*path).to_string(),
            });
        }
    }
    for word in BANNED_WORDS {
        for pos in find_word(&code, word) {
            found.push(Violation {
                line: line_of(&code, pos),
                what: (*word).to_string(),
            });
        }
    }
    for pos in find_word(&code, "fs") {
        let rest = &code[pos + 2..];
        if let Some(group) = rest.strip_prefix("::{") {
            let close = group.find('}').unwrap_or(group.len());
            for name in GROUPED_BANNED {
                if !find_word(&group[..close], name).is_empty() {
                    found.push(Violation {
                        line: line_of(&code, pos),
                        what: format!("fs::{{{name}}}"),
                    });
                }
            }
        }
    }

    found.sort_by_key(|v| v.line);
    found.dedup_by(|a, b| a.line == b.line && a.what == b.what);
    found
}

pub fn fixtures_dependency_violations(manifest: &str) -> Vec<usize> {
    let mut found = Vec::new();
    let mut in_runtime_deps = false;
    for (index, raw) in manifest.lines().enumerate() {
        let line = raw.trim();
        if let Some(header) = line.strip_prefix('[') {
            let header = header.trim_end_matches(']').trim_matches('[');
            let kind = header
                .split('.')
                .find(|part| part.ends_with("dependencies"))
                .unwrap_or("");
            in_runtime_deps = matches!(kind, "dependencies" | "build-dependencies")
                && !header.starts_with("workspace");
            if in_runtime_deps && header.ends_with(TEST_ONLY_CRATE) {
                found.push(index + 1);
            }
        } else if in_runtime_deps && line.contains(TEST_ONLY_CRATE) {
            found.push(index + 1);
        }
    }
    found
}

pub fn comment_violations(source: &str) -> Vec<usize> {
    scan(source).comment_lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn whats(source: &str) -> Vec<String> {
        write_violations(source)
            .into_iter()
            .map(|v| v.what)
            .collect()
    }

    #[test]
    fn clean_reader_code_passes() {
        let src = "use std::fs;\nfn r(p: &str) -> String { fs::read_to_string(p).unwrap() }\n";
        assert!(write_violations(src).is_empty());
    }

    #[test]
    fn flags_fs_write() {
        assert_eq!(
            whats("fn a() { std::fs::write(\"x\", b\"y\").unwrap(); }"),
            vec!["fs::write"]
        );
    }

    #[test]
    fn flags_rename_copy_and_create() {
        let src = "fn a() { fs::rename(a, b); fs::copy(a, b); File::create(a); }";
        let got = whats(src);
        assert!(got.contains(&"fs::rename".to_string()));
        assert!(got.contains(&"fs::copy".to_string()));
        assert!(got.contains(&"File::create".to_string()));
    }

    #[test]
    fn flags_directory_and_removal_calls() {
        let src = "fn a() { create_dir_all(p); remove_file(p); remove_dir_all(p); }";
        let got = whats(src);
        assert!(got.contains(&"create_dir_all".to_string()));
        assert!(got.contains(&"remove_file".to_string()));
        assert!(got.contains(&"remove_dir_all".to_string()));
    }

    #[test]
    fn flags_open_options_and_symlinks() {
        let got = whats("fn a() { OpenOptions::new(); std::os::unix::fs::symlink(a, b); }");
        assert!(got.contains(&"OpenOptions".to_string()));
        assert!(got.contains(&"symlink".to_string()));
    }

    #[test]
    fn flags_grouped_imports() {
        let got = whats("use std::fs::{read, write};\n");
        assert_eq!(got, vec!["fs::{write}"]);
    }

    #[test]
    fn grouped_read_only_imports_pass() {
        assert!(write_violations("use std::fs::{read, read_dir};\n").is_empty());
    }

    #[test]
    fn mentions_in_strings_and_comments_are_ignored() {
        let src = "// fs::write\nfn a() -> &'static str { \"fs::write remove_file\" }\n";
        assert!(write_violations(src).is_empty());
    }

    #[test]
    fn longer_identifiers_are_not_matched() {
        assert!(write_violations("fn remove_file_name() {}\n").is_empty());
    }

    #[test]
    fn reports_line_numbers() {
        let v = write_violations("fn a() {}\nfn b() { fs::write(p, d); }\n");
        assert_eq!(v[0].line, 2);
    }

    #[test]
    fn dev_dependency_on_fixtures_passes() {
        let m = "[dependencies]\ndrift-core.workspace = true\n\n[dev-dependencies]\ndrift-fixtures.workspace = true\n";
        assert!(fixtures_dependency_violations(m).is_empty());
    }

    #[test]
    fn runtime_dependency_on_fixtures_fails() {
        let m = "[package]\nname = \"x\"\n\n[dependencies]\ndrift-fixtures.workspace = true\n";
        assert_eq!(fixtures_dependency_violations(m), vec![5]);
    }

    #[test]
    fn build_and_target_dependencies_on_fixtures_fail() {
        let m = "[build-dependencies]\ndrift-fixtures = { path = \"../drift-fixtures\" }\n[target.'cfg(unix)'.dependencies]\ndrift-fixtures.workspace = true\n";
        assert_eq!(fixtures_dependency_violations(m), vec![2, 4]);
    }

    #[test]
    fn table_form_and_renamed_dependencies_fail() {
        let m = "[dependencies.drift-fixtures]\nworkspace = true\n[dependencies]\nfx = { package = \"drift-fixtures\", path = \"p\" }\n";
        assert_eq!(fixtures_dependency_violations(m), vec![1, 4]);
    }

    #[test]
    fn workspace_declaration_is_allowed() {
        let m = "[workspace.dependencies]\ndrift-fixtures = { path = \"crates/drift-fixtures\" }\n";
        assert!(fixtures_dependency_violations(m).is_empty());
    }

    #[test]
    fn comment_check_reports_lines() {
        assert_eq!(comment_violations("fn a() {}\n// x\n"), vec![2]);
        assert!(comment_violations("fn a() {}\n").is_empty());
    }
}
