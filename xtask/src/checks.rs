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

fn parse_manifest(manifest: &str) -> Result<toml::Table, String> {
    manifest
        .parse::<toml::Table>()
        .map_err(|e| format!("invalid TOML: {}", e.message()))
}

fn runtime_dependency_sections(table: &toml::Table) -> Vec<(String, &toml::Table)> {
    let mut sections = Vec::new();
    for kind in ["dependencies", "build-dependencies"] {
        if let Some(t) = table.get(kind).and_then(toml::Value::as_table) {
            sections.push((kind.to_string(), t));
        }
    }
    if let Some(targets) = table.get("target").and_then(toml::Value::as_table) {
        for (target, config) in targets {
            for kind in ["dependencies", "build-dependencies"] {
                if let Some(t) = config.get(kind).and_then(toml::Value::as_table) {
                    sections.push((format!("target.{target}.{kind}"), t));
                }
            }
        }
    }
    sections
}

pub fn fixtures_dependency_violations(manifest: &str) -> Vec<String> {
    let table = match parse_manifest(manifest) {
        Ok(t) => t,
        Err(message) => return vec![message],
    };
    let mut found = Vec::new();
    for (section, deps) in runtime_dependency_sections(&table) {
        for (name, value) in deps {
            let renamed = value
                .get("package")
                .and_then(toml::Value::as_str)
                .is_some_and(|p| p == TEST_ONLY_CRATE);
            if name == TEST_ONLY_CRATE || renamed {
                found.push(format!(
                    "`{TEST_ONLY_CRATE}` in [{section}] may only be a dev-dependency"
                ));
            }
        }
    }
    found
}

pub fn feature_violations(manifest: &str) -> Vec<String> {
    let table = match parse_manifest(manifest) {
        Ok(t) => t,
        Err(message) => return vec![message],
    };
    let Some(features) = table.get("features").and_then(toml::Value::as_table) else {
        return Vec::new();
    };
    let items = |name: &str| -> Vec<String> {
        features
            .get(name)
            .and_then(toml::Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(toml::Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut enabled: Vec<String> = Vec::new();
    let mut queue = items("default");
    while let Some(name) = queue.pop() {
        if enabled.contains(&name) {
            continue;
        }
        queue.extend(items(&name));
        enabled.push(name);
    }
    features
        .keys()
        .filter(|name| *name != "default" && !enabled.contains(name))
        .map(|name| format!("feature `{name}` is off by default; features ship on by default"))
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
    fn dev_dependency_on_fixtures_passes() {
        let m = "[dependencies]\ndrift-core.workspace = true\n\n[dev-dependencies]\ndrift-fixtures.workspace = true\n";
        assert!(fixtures_dependency_violations(m).is_empty());
    }

    #[test]
    fn runtime_dependency_on_fixtures_fails() {
        let m = "[package]\nname = \"x\"\n\n[dependencies]\ndrift-fixtures.workspace = true\n";
        assert_eq!(fixtures_dependency_violations(m).len(), 1);
    }

    #[test]
    fn build_and_target_dependencies_on_fixtures_fail() {
        let m = "[build-dependencies]\ndrift-fixtures = { path = \"../drift-fixtures\" }\n[target.'cfg(unix)'.dependencies]\ndrift-fixtures.workspace = true\n";
        assert_eq!(fixtures_dependency_violations(m).len(), 2);
    }

    #[test]
    fn table_inline_dotted_and_renamed_dependencies_fail() {
        let table_form = "[dependencies.drift-fixtures]\nworkspace = true\n";
        let inline = "dependencies = { drift-fixtures = { workspace = true } }\n";
        let dotted = "dependencies.drift-fixtures.workspace = true\n";
        let renamed = "[dependencies]\nfx = { package = 'drift-fixtures', path = 'p' }\n";
        for m in [table_form, inline, dotted, renamed] {
            assert_eq!(fixtures_dependency_violations(m).len(), 1, "{m}");
        }
    }

    #[test]
    fn workspace_declaration_is_allowed() {
        let m = "[workspace.dependencies]\ndrift-fixtures = { path = \"crates/drift-fixtures\" }\n";
        assert!(fixtures_dependency_violations(m).is_empty());
    }

    #[test]
    fn invalid_manifest_is_reported() {
        assert_eq!(fixtures_dependency_violations("[dependencies\n").len(), 1);
        assert_eq!(feature_violations("[features\n").len(), 1);
    }

    #[test]
    fn default_off_feature_fails() {
        let m = "[features]\ndefault = []\nextra = []\n";
        assert_eq!(feature_violations(m).len(), 1);
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
    fn single_quoted_literal_strings_are_understood() {
        let m = "[features]\ndefault = ['extra']\nextra = []\n";
        assert!(feature_violations(m).is_empty());
        let m = "[features]\ndefault = ['a']\na = ['b']\nb = []\nc = []\n";
        assert_eq!(feature_violations(m).len(), 1);
    }

    #[test]
    fn dotted_and_inline_feature_tables_are_understood() {
        let dotted = "features.default = ['a']\nfeatures.a = []\nfeatures.b = []\n";
        assert_eq!(feature_violations(dotted).len(), 1);
        let inline = "features = { default = ['a'], a = [] }\n";
        assert!(feature_violations(inline).is_empty());
        let off = "features = { default = [], a = [] }\n";
        assert_eq!(feature_violations(off).len(), 1);
    }

    #[test]
    fn multiline_arrays_comments_and_no_features_table() {
        let m = "[features]\ndefault = [\n    \"a\", # on\n]\na = []\n";
        assert!(feature_violations(m).is_empty());
        assert!(feature_violations("[dependencies]\nx = \"1\"\n").is_empty());
    }

    #[test]
    fn alias_overwritten_in_another_module_is_still_caught() {
        let src = "mod w { use std::fs as disk; pub fn f(p: &Path) { disk::write(p, b\"x\").unwrap(); } }\nmod r { use std::io as disk; }\n";
        assert_eq!(whats(src), vec!["fs::write"]);
        let reversed = "mod r { use std::io as disk; }\nmod w { use std::fs as disk; pub fn f(p: &Path) { disk::write(p, b\"x\"); } }\n";
        assert_eq!(whats(reversed), vec!["fs::write"]);
    }

    #[test]
    fn re_exported_fs_is_caught_across_modules() {
        let src =
            "mod a { pub use std::fs as disk; }\nfn f(p: &Path) { a::disk::write(p, b\"x\"); }\n";
        let got = whats(src);
        assert!(got.contains(&"fs::write".to_string()));
        assert!(got.contains(&"pub use fs".to_string()));
        assert_eq!(
            whats("pub(crate) use std::fs as disk;\n"),
            vec!["pub use fs"]
        );
    }

    #[test]
    fn comment_check_reports_lines() {
        assert_eq!(comment_violations("fn a() {}\n// x\n"), vec![2]);
        assert!(comment_violations("fn a() {}\n").is_empty());
    }
}
