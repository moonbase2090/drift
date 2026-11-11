use crate::imports;
use crate::lex::scan;

pub const WRITER_CRATES: &[&str] = &["drift-apply", "drift-fixtures"];
pub const TEST_ONLY_CRATE: &str = "drift-fixtures";

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

pub struct Violation {
    pub line: usize,
    pub what: String,
}

pub fn write_violations(source: &str) -> Vec<Violation> {
    let code = scan(source).code;
    let mut found: Vec<Violation> = imports::banned_paths(&code)
        .into_iter()
        .map(|(line, what)| Violation { line, what })
        .collect();
    for (line, name) in imports::identifiers(&code) {
        if BANNED_WORDS.contains(&name.as_str()) {
            found.push(Violation { line, what: name });
        }
    }
    found.sort_by(|a, b| (a.line, &a.what).cmp(&(b.line, &b.what)));
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

pub fn feature_violations(manifest: &str) -> Vec<(usize, String)> {
    let mut features: Vec<(usize, String, Vec<String>)> = Vec::new();
    let mut in_features = false;
    let mut pending: Option<(usize, String, String)> = None;
    for (index, raw) in manifest.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if let Some((start, name, mut value)) = pending.take() {
            value.push_str(line);
            if value.matches('[').count() <= value.matches(']').count() {
                features.push((start, name, quoted(&value)));
            } else {
                pending = Some((start, name, value));
            }
            continue;
        }
        if line.starts_with('[') {
            in_features = line.trim_matches(|c| c == '[' || c == ']').trim() == "features";
        } else if in_features {
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            let (name, value) = (name.trim().trim_matches('"').to_string(), value.to_string());
            if value.matches('[').count() <= value.matches(']').count() {
                features.push((index + 1, name, quoted(&value)));
            } else {
                pending = Some((index + 1, name, value));
            }
        }
    }
    off_by_default(&features)
}

fn quoted(value: &str) -> Vec<String> {
    value
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

fn off_by_default(features: &[(usize, String, Vec<String>)]) -> Vec<(usize, String)> {
    let mut enabled: Vec<String> = Vec::new();
    let mut queue: Vec<String> = features
        .iter()
        .filter(|(_, name, _)| name == "default")
        .flat_map(|(_, _, items)| items.clone())
        .collect();
    while let Some(name) = queue.pop() {
        if enabled.contains(&name) {
            continue;
        }
        if let Some((_, _, items)) = features.iter().find(|(_, n, _)| *n == name) {
            queue.extend(items.iter().cloned());
        }
        enabled.push(name);
    }
    features
        .iter()
        .filter(|(_, name, _)| name != "default" && !enabled.contains(name))
        .map(|(line, name, _)| (*line, name.clone()))
        .collect()
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
        assert_eq!(got, vec!["fs::write"]);
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
    fn flags_aliased_fs_module() {
        let src = "fn a(p: &Path) { use std::fs as disk; disk::write(p, b\"x\").unwrap(); }";
        assert_eq!(whats(src), vec!["fs::write"]);
    }

    #[test]
    fn flags_spaced_and_aliased_group_imports() {
        assert_eq!(whats("use std::fs::{ read , write };\n"), vec!["fs::write"]);
        assert_eq!(
            whats("use std::{fs as d, io};\nfn a() { d::rename(a, b); }\n"),
            vec!["fs::rename"]
        );
        assert_eq!(
            whats("use std::fs::{self as d, read};\nfn a() { d::copy(a, b); }\n"),
            vec!["fs::copy"]
        );
    }

    #[test]
    fn flags_renamed_function_and_type_imports() {
        assert_eq!(whats("use std::fs::write as w;\n"), vec!["fs::write"]);
        let src = "use std::fs::File as F;\nfn a() { F::create(p); }\n";
        assert_eq!(whats(src), vec!["File::create"]);
        assert_eq!(
            whats("use std::fs::OpenOptions as O;\n"),
            vec!["OpenOptions"]
        );
    }

    #[test]
    fn flags_glob_and_nested_aliases() {
        assert_eq!(whats("use std::fs::*;\n"), vec!["fs::*"]);
        assert_eq!(
            whats("use std as s;\nfn a() { s::fs::rename(a, b); }\n"),
            vec!["fs::rename"]
        );
        assert_eq!(
            whats("use std::fs;\nuse fs as disk;\nfn a() { disk::write(p, d); }\n"),
            vec!["fs::write"]
        );
        assert_eq!(
            whats("type D = std::fs::OpenOptions;\n"),
            vec!["OpenOptions"]
        );
    }

    #[test]
    fn unrelated_aliases_and_methods_pass() {
        assert!(write_violations(
            "use std::io::Write as W;\nfn a(o: &mut impl W) { o.write(b\"x\"); }\n"
        )
        .is_empty());
        assert!(write_violations("use my::disk;\nfn a() { disk::write(p); }\n").is_empty());
        assert!(write_violations("use std::fs;\nfn a() { fs::read_to_string(p); }\n").is_empty());
        assert!(write_violations("use std::fs::{self, read_dir};\n").is_empty());
    }

    #[test]
    fn default_off_feature_fails() {
        let m = "[features]\ndefault = []\nextra = []\n";
        assert_eq!(feature_violations(m), vec![(3, "extra".to_string())]);
    }

    #[test]
    fn feature_without_default_fails() {
        assert_eq!(feature_violations("[features]\nextra = []\n").len(), 1);
    }

    #[test]
    fn features_enabled_by_default_pass() {
        let m = "[features]\ndefault = [\"a\", \"b\"]\na = []\nb = [\"c\"]\nc = []\n";
        assert!(feature_violations(m).is_empty());
    }

    #[test]
    fn multiline_default_and_no_features_table() {
        let m = "[features]\ndefault = [\n    \"a\", # on\n]\na = []\n";
        assert!(feature_violations(m).is_empty());
        assert!(feature_violations("[dependencies]\nx = \"1\"\n").is_empty());
    }

    #[test]
    fn feature_reachable_only_through_dependency_chain_passes() {
        let m = "[features]\ndefault = [\"a\"]\na = [\"b\"]\nb = []\n";
        assert!(feature_violations(m).is_empty());
    }

    #[test]
    fn comment_check_reports_lines() {
        assert_eq!(comment_violations("fn a() {}\n// x\n"), vec![2]);
        assert!(comment_violations("fn a() {}\n").is_empty());
    }
}
