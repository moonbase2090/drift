use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use drift_fixtures::{MaterializedRoot, RootBuilder};
use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

const FILES: [(&str, &str, Option<&str>, Option<&str>); 6] = [
    (
        "hypr/hyprland.conf",
        "a\nB\n",
        Some("a\nb\n"),
        Some("a\nb\n"),
    ),
    (
        "hypr/monitors.lua",
        "scale = 1\n",
        Some("scale = 2\n"),
        Some("scale = 1\n"),
    ),
    (
        "hypr/looknfeel.lua",
        "gaps = 5\nrounding = 4\n",
        Some("gaps = 8\nrounding = 2\n"),
        Some("gaps = 5\nrounding = 2\n"),
    ),
    (
        "hypr/bindings.lua",
        "x = 3\n",
        Some("x = 2\n"),
        Some("x = 1\n"),
    ),
    (
        "waybar/style.css",
        "* { color: red; }\n",
        Some("* { color: blue; }\n"),
        Some("* { color: blue; }\n"),
    ),
    (
        "waybar/config.jsonc",
        "{ \"height\": 30 }\n",
        Some("{ \"height\": 26 }\n"),
        Some("{ \"height\": 26 }\n"),
    ),
];

fn sha(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn world() -> MaterializedRoot {
    let mut builder = RootBuilder::new();
    for keep in [
        "home/.local/share/.keep",
        "home/.local/state/.keep",
        "runtime/.keep",
        "bin/.keep",
    ] {
        builder.file(keep, b"").unwrap();
    }
    builder
        .file(
            "home/.config/omarchy/extensions/omarchy-menu.jsonc",
            "{\n  // Extend the menu.\n  // \"about\": {\"label\": \"About\"},\n}\n",
        )
        .unwrap()
        .file("home/.config/omarchy/hooks/post-update.d/.keep", b"")
        .unwrap();
    for name in ["bin/omarchy-notification-send"] {
        builder
            .file_with_mode(name, b"#!/bin/sh\nexit 0\n", 0o755)
            .unwrap();
    }
    let mut listed = String::from("%FILES%\nusr/share/omarchy/config/\n");
    let mut mtree = String::from("#mtree\n/set type=file uid=0 gid=0 mode=644\n");
    for (rel, theirs, ours, base) in FILES {
        listed.push_str(&format!("usr/share/omarchy/config/{rel}\n"));
        mtree.push_str(&format!(
            "./usr/share/omarchy/config/{rel} time=1.0 size=1 sha256digest={}\n",
            sha(theirs)
        ));
        builder
            .file(format!("usr/share/omarchy/config/{rel}"), theirs)
            .unwrap();
        if let Some(ours) = ours {
            builder.file(format!("home/.config/{rel}"), ours).unwrap();
        }
        if let Some(base) = base {
            builder
                .file(format!("home/.local/share/drift/bases/config/{rel}"), base)
                .unwrap();
        }
    }
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(mtree.as_bytes()).unwrap();
    let dir = "var/lib/pacman/local/omarchy-settings-4.5.0-3";
    builder
        .file(
            format!("{dir}/desc"),
            "%NAME%\nomarchy-settings\n\n%VERSION%\n4.5.0-3\n",
        )
        .unwrap()
        .file(format!("{dir}/files"), listed)
        .unwrap()
        .file(format!("{dir}/mtree"), encoder.finish().unwrap())
        .unwrap();
    builder.materialize().unwrap()
}

fn drift(root: &Path, args: &[String]) -> Output {
    let home = root.join("home");
    Command::new(env!("CARGO_BIN_EXE_drift"))
        .args(args)
        .env_clear()
        .env("PATH", root.join("bin"))
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_STATE_HOME", home.join(".local/state"))
        .env("XDG_RUNTIME_DIR", root.join("runtime"))
        .env("OMARCHY_PATH", root.join("usr/share/omarchy"))
        .env("DRIFT_ROOT", root)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap()
}

fn words(line: &str, home: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            ' ' if !quoted => {
                if !current.is_empty() {
                    found.push(std::mem::take(&mut current));
                }
            }
            other => current.push(other),
        }
    }
    if !current.is_empty() {
        found.push(current);
    }
    found
        .into_iter()
        .map(|word| word.replace("$HOME", &home.to_string_lossy()))
        .collect()
}

fn documented(file: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(file).unwrap();
    let mut commands = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            inside = line.trim() == "```sh";
            continue;
        }
        if inside && !line.trim().is_empty() {
            commands.push(line.trim().to_string());
        }
    }
    commands
}

fn newest_transaction(root: &Path) -> String {
    let output = drift(root, &["history".to_string(), "--json".to_string()]);
    let text = String::from_utf8(output.stdout.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{error}: {text} {:?}", output.stderr));
    value["rows"][0]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("a seeded transaction: {text}"))
        .to_string()
}

fn docs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn every_command_in_the_documentation_runs_against_a_fixture_system() {
    let mut files = vec![docs_dir().join("README.md")];
    for name in ["guide.md", "cli.md", "widget.md"] {
        files.push(docs_dir().join("docs").join(name));
    }
    let mut ran = 0;
    let mut seen = std::collections::BTreeSet::new();
    let mut outputs = std::collections::BTreeMap::new();
    for file in files {
        for line in documented(&file) {
            let verb = line.split_whitespace().next().unwrap_or("");
            assert_eq!(
                verb,
                "drift",
                "{}: only drift commands belong in sh blocks: {line}",
                file.display()
            );
            let root = world();
            let seeded = drift(
                root.path(),
                &[
                    "resolve".to_string(),
                    ".config/waybar/config.jsonc".to_string(),
                    "--upstream".to_string(),
                    "--yes".to_string(),
                ],
            );
            assert!(seeded.status.success(), "{:?}", seeded);
            let id = newest_transaction(root.path());
            let line = line.replace("<transaction>", &id);
            let args = words(&line, &root.path().join("home"));
            let output = drift(root.path(), &args[1..]);
            let code = output.status.code();
            assert!(
                matches!(code, Some(0) | Some(1)),
                "{}: `{line}` exited {code:?}\nstdout: {}\nstderr: {}",
                file.display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            seen.insert(args[1].clone());
            outputs.insert(
                line.clone(),
                String::from_utf8_lossy(&output.stdout).to_string(),
            );
            ran += 1;
        }
    }
    assert!(ran >= 20, "{ran}");
    let said = |command: &str, needle: &str| {
        let found = outputs
            .iter()
            .find(|(line, _)| line.starts_with(command))
            .unwrap_or_else(|| panic!("`{command}` is not documented"));
        assert!(found.1.contains(needle), "`{command}` printed {}", found.1);
    };
    said("drift scan", "monitors.lua");
    said("drift status --json", "upstream");
    said("drift status --bar", "\"text\"");
    said("drift diff .config/hypr/hyprland.conf", "hyprland.conf");
    said("drift history", "resolve-");
    said("drift undo --dry-run", "config.jsonc");
    said("drift integrate --dry-run", "post-update.d/drift");
    said("drift resolve .config/hypr/hyprland.conf", "hyprland.conf");
    for verb in [
        "scan",
        "status",
        "diff",
        "resolve",
        "history",
        "undo",
        "snapshot",
        "notify",
        "integrate",
    ] {
        assert!(seen.contains(verb), "no documented command runs {verb}");
    }
}
