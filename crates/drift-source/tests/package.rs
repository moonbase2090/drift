use std::io::Write;

use drift_fixtures::RootBuilder;
use drift_source::{read_package, Check, ReadPaths, Upgrade};
use flate2::write::GzEncoder;
use flate2::Compression;

const ALPHA: &[u8] = b"alpha\n";
const BETA: &[u8] = b"beta\n";
const SHELL: &[u8] = b"{\"theme\":\"dark\"}\n";
const SECRET: &[u8] = b"secret\n";
const ALPHA_SHA: &str = "b6a98d9ce9a2d9149288fa3df42d377c3e42737afdcdaf714e33c0a100b51060";
const BETA_SHA: &str = "f2c82decdd7181cf98945929a62598db7e6b477e11f6e0eb0ae97020eff151ad";
const SHELL_SHA: &str = "057c817c6a65ba4c95e68d754eaf8323b5714e95d5ecb624049f5afe95e47ba4";
const SECRET_SHA: &str = "b37e50cedcd3e3f1ff64f4afc0422084ae694253cf399326868e07a35f4a45fb";

fn gzip(text: &str) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(text.as_bytes()).unwrap();
    encoder.finish().unwrap()
}

fn desc(version: &str) -> String {
    format!("%NAME%\nomarchy-settings\n\n%VERSION%\n{version}\n\n%ARCH%\nx86_64\n")
}

fn files(paths: &[&str]) -> String {
    let mut text = String::from("%FILES%\n");
    for path in paths {
        text.push_str(path);
        text.push('\n');
    }
    text.push_str("\n%BACKUP%\netc/foo\n");
    text
}

fn mtree(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut text = String::from("#mtree\n/set type=file uid=0 gid=0 mode=644\n");
    for (path, digest) in entries {
        text.push_str(&format!("./{path} time=1.0 size=1 sha256digest={digest}\n"));
    }
    text.push_str("./usr/share/omarchy/config type=dir\n");
    gzip(&text)
}

fn read(
    files_to_write: &[(&str, &[u8])],
    version: &str,
    list: &[&str],
    mtree_bytes: Option<Vec<u8>>,
    log: Option<&str>,
) -> drift_source::InstalledPackage {
    let mut builder = RootBuilder::new();
    let dir = format!("var/lib/pacman/local/omarchy-settings-{version}");
    builder
        .file(format!("{dir}/desc"), desc(version))
        .unwrap()
        .file(format!("{dir}/files"), files(list))
        .unwrap();
    if let Some(bytes) = mtree_bytes {
        builder.file(format!("{dir}/mtree"), bytes).unwrap();
    }
    if let Some(log) = log {
        builder.file("var/log/pacman.log", log).unwrap();
    }
    for (path, bytes) in files_to_write {
        builder.file(*path, *bytes).unwrap();
    }
    let tree = builder.materialize().unwrap();
    let paths = ReadPaths::new(tree.path(), tree.path().join("usr/share/omarchy"));
    read_package(&paths).unwrap()
}

#[test]
fn a_matching_install_lists_only_verified_templates() {
    let log = "\
[2026-01-01T00:00:00+0000] [ALPM] installed omarchy-settings (0.1.0-1)
[2026-01-02T00:00:00+0000] [ALPM] upgraded omarchy (1 -> 2)
[2026-01-03T00:00:00+0000] [ALPM] upgraded omarchy-settings (1.0.0-1 -> 1.1.0-1)
[2026-02-01T00:00:00+0000] [ALPM] upgraded omarchy-settings (1.1.0-1 -> 1.2.0-1)
[2026-03-01T00:00:00+0000] [ALPM] upgraded other (1 -> 2)
[2026-04-01T00:00:00+0000] [ALPM] upgraded omarchy-settings (not an upgrade)
[2026-05-01T00:00:00+0000] [ALPM] upgraded omarchy-settings (1.2.0-1 -> 2.0.0-1)
";
    let installed = read(
        &[
            ("usr/share/omarchy/config/hypr/a.lua", ALPHA),
            ("usr/share/omarchy/config/omarchy/shell.json", SHELL),
            ("usr/share/omarchy/bin/tool", ALPHA),
            ("etc/hosts", b"nope\n"),
        ],
        "9.9.9-1",
        &[
            "usr/share/omarchy/config/",
            "usr/share/omarchy/config/hypr/a.lua",
            "usr/share/omarchy/config/omarchy/shell.json",
            "usr/share/omarchy/bin/tool",
            "etc/hosts",
        ],
        Some(mtree(&[
            ("usr/share/omarchy/config/hypr/a.lua", ALPHA_SHA),
            ("usr/share/omarchy/config/omarchy/shell.json", SHELL_SHA),
            ("usr/share/omarchy/bin/tool", ALPHA_SHA),
        ])),
        Some(log),
    );
    assert_eq!(installed.version, "9.9.9-1");
    assert!(installed.mtree_present);
    assert_eq!(installed.templates.len(), 2);
    assert!(installed
        .templates
        .iter()
        .all(|template| template.warning.is_none()));
    assert_eq!(installed.templates[0].path, "config/hypr/a.lua");
    assert_eq!(installed.templates[0].sha256.as_deref(), Some(ALPHA_SHA));
    assert_eq!(
        installed.templates[0].mtree_sha256.as_deref(),
        Some(ALPHA_SHA)
    );
    assert_eq!(installed.templates[1].path, "config/omarchy/shell.json");
    assert_eq!(installed.templates[1].sha256.as_deref(), Some(SHELL_SHA));
    assert_eq!(
        installed.upgrades,
        vec![
            Upgrade {
                from: "1.0.0-1".to_string(),
                to: "1.1.0-1".to_string(),
            },
            Upgrade {
                from: "1.1.0-1".to_string(),
                to: "1.2.0-1".to_string(),
            },
            Upgrade {
                from: "1.2.0-1".to_string(),
                to: "2.0.0-1".to_string(),
            },
        ]
    );
}

#[test]
fn a_missing_mtree_still_lists_the_template() {
    let installed = read(
        &[("usr/share/omarchy/config/hypr/a.lua", BETA)],
        "1.0.0-1",
        &["usr/share/omarchy/config/hypr/a.lua"],
        None,
        None,
    );
    assert_eq!(installed.version, "1.0.0-1");
    assert!(!installed.mtree_present);
    assert!(installed.upgrades.is_empty());
    assert_eq!(installed.templates.len(), 1);
    assert_eq!(installed.templates[0].path, "config/hypr/a.lua");
    assert_eq!(installed.templates[0].sha256.as_deref(), Some(BETA_SHA));
    assert_eq!(installed.templates[0].mtree_sha256, None);
    assert_eq!(installed.templates[0].warning, Some(Check::MtreeMissing));
}

#[test]
fn a_hash_mismatch_is_flagged_and_kept() {
    let installed = read(
        &[("usr/share/omarchy/config/hypr/a.lua", ALPHA)],
        "2.0.0-1",
        &["usr/share/omarchy/config/hypr/a.lua"],
        Some(mtree(&[("usr/share/omarchy/config/hypr/a.lua", BETA_SHA)])),
        None,
    );
    assert_eq!(installed.templates.len(), 1);
    let template = &installed.templates[0];
    assert_eq!(template.path, "config/hypr/a.lua");
    assert_eq!(template.warning, Some(Check::HashMismatch));
    assert_eq!(template.sha256.as_deref(), Some(ALPHA_SHA));
    assert_eq!(template.mtree_sha256.as_deref(), Some(BETA_SHA));
}

#[test]
fn a_file_missing_from_the_mtree_is_flagged_and_kept() {
    let installed = read(
        &[("usr/share/omarchy/config/hypr/a.lua", ALPHA)],
        "3.0.0-1",
        &["usr/share/omarchy/config/hypr/a.lua"],
        Some(mtree(&[("usr/share/omarchy/config/other.lua", SHELL_SHA)])),
        None,
    );
    assert_eq!(installed.templates.len(), 1);
    assert_eq!(installed.templates[0].warning, Some(Check::NotInMtree));
    assert_eq!(installed.templates[0].sha256.as_deref(), Some(ALPHA_SHA));
    assert_eq!(installed.templates[0].mtree_sha256, None);
}

#[test]
fn a_missing_template_file_is_flagged_and_kept() {
    let installed = read(
        &[],
        "4.0.0-1",
        &["usr/share/omarchy/config/hypr/a.lua"],
        Some(mtree(&[("usr/share/omarchy/config/hypr/a.lua", ALPHA_SHA)])),
        None,
    );
    assert_eq!(installed.templates.len(), 1);
    assert_eq!(installed.templates[0].warning, Some(Check::MissingOnDisk));
    assert_eq!(installed.templates[0].sha256, None);
    assert_eq!(
        installed.templates[0].mtree_sha256.as_deref(),
        Some(ALPHA_SHA)
    );
}

#[test]
fn a_path_that_escapes_the_template_root_is_not_hashed() {
    let installed = read(
        &[("etc/passwd", SECRET)],
        "5.0.0-1",
        &["usr/share/omarchy/config/../../etc/passwd"],
        Some(mtree(&[(
            "usr/share/omarchy/config/../../etc/passwd",
            SECRET_SHA,
        )])),
        None,
    );
    assert_eq!(installed.templates.len(), 1);
    assert_eq!(installed.templates[0].path, "config/../../etc/passwd");
    assert_eq!(installed.templates[0].warning, Some(Check::UnsafePath));
    assert_eq!(installed.templates[0].sha256, None);
    assert_eq!(installed.templates[0].mtree_sha256, None);
}

#[test]
fn a_missing_package_is_an_error() {
    let tree = RootBuilder::new().materialize().unwrap();
    let paths = ReadPaths::new(tree.path(), tree.path().join("usr/share/omarchy"));
    assert!(matches!(
        read_package(&paths),
        Err(drift_source::Error::NotInstalled)
    ));
}
