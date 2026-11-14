use std::collections::BTreeSet;
use std::path::PathBuf;

const MODULES: [&str; 2] = ["tree", "filter"];
const ALLOW_PLANNED: bool = true;

fn docs(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

fn test_functions(module: &str) -> BTreeSet<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/e2e")
        .join(format!("{module}.rs"));
    let text = std::fs::read_to_string(&path).unwrap();
    let mut found = BTreeSet::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim() == "#[test]" {
            let next = lines.peek().copied().unwrap_or("");
            assert!(
                !next.contains("#[ignore"),
                "{module}: an ignored test is not a test"
            );
            if let Some(name) = lines
                .by_ref()
                .find_map(|line| line.trim().strip_prefix("fn "))
            {
                let name = name.split('(').next().unwrap();
                found.insert(format!("{module}::{name}"));
            }
        }
    }
    found
}

struct Entry {
    id: String,
    tests: Vec<String>,
}

fn entries() -> Vec<Entry> {
    docs("feature-map.md")
        .lines()
        .filter(|line| line.starts_with("| `"))
        .map(|line| {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            let id = cells[1].trim_matches('`').to_string();
            let tests = cells[3]
                .split(',')
                .map(|cell| cell.trim().trim_matches('`').to_string())
                .filter(|cell| cell != "planned")
                .collect();
            Entry { id, tests }
        })
        .collect()
}

fn action_ids() -> Vec<String> {
    docs("input-map.md")
        .lines()
        .filter(|line| line.starts_with("| ") && line.matches('`').count() >= 2)
        .filter_map(|line| line.split('|').nth(2))
        .map(|cell| cell.trim().trim_matches('`').to_string())
        .filter(|id| id.contains('.'))
        .collect()
}

#[test]
fn the_feature_map_matches_the_tests() {
    let entries = entries();
    let mut problems = Vec::new();
    let known: BTreeSet<String> = MODULES.iter().flat_map(|m| test_functions(m)).collect();
    let mut seen = BTreeSet::new();
    let mut used = BTreeSet::new();
    for entry in &entries {
        if !seen.insert(entry.id.clone()) {
            problems.push(format!("{} is listed twice", entry.id));
        }
        if entry.tests.is_empty() && !ALLOW_PLANNED {
            problems.push(format!("{} has no test", entry.id));
        }
        for test in &entry.tests {
            if !known.contains(test) {
                problems.push(format!("{} names {test}, which does not exist", entry.id));
            }
            used.insert(test.clone());
        }
    }
    for id in action_ids() {
        if !seen.contains(&id) {
            problems.push(format!("the key map has {id} but the feature map does not"));
        }
    }
    for test in &known {
        if !used.contains(test) {
            problems.push(format!("{test} is not linked from the feature map"));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_guard_reads_every_test_module() {
    for module in MODULES {
        assert!(!test_functions(module).is_empty(), "{module} has no tests");
    }
    let on_disk: BTreeSet<String> =
        std::fs::read_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/e2e"))
            .unwrap()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter_map(|name| name.strip_suffix(".rs").map(str::to_string))
            .filter(|name| !["main", "harness", "system", "view", "map"].contains(&name.as_str()))
            .collect();
    let listed: BTreeSet<String> = MODULES.iter().map(|m| m.to_string()).collect();
    assert_eq!(on_disk, listed);
}
