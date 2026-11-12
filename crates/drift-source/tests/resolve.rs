use std::io::Write;

use drift_core::{BaseProvenance, RelPath};
use drift_fixtures::RootBuilder;
use drift_source::{resolve_bases, BaseQuery, ResolvePaths};
use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

const REVIEWED: &[u8] = b"reviewed\n";
const SNAPSHOT: &[u8] = b"snapshot\n";
const CACHED: &[u8] = b"cached\n";
const OTHER: &[u8] = b"other\n";

fn digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let hashed = Sha256::digest(bytes);
    let mut out = String::with_capacity(hashed.len() * 2);
    for byte in hashed {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

fn gzip(text: &str) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(text.as_bytes()).unwrap();
    encoder.finish().unwrap()
}

fn mtree(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut text = String::from("#mtree\n/set type=file uid=0 gid=0 mode=644\n");
    for (path, hashed) in entries {
        text.push_str(&format!("./{path} time=1.0 size=1 sha256digest={hashed}\n"));
    }
    gzip(&text)
}

fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut raw = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut raw);
        for (path, bytes) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, path, *bytes).unwrap();
        }
        builder.finish().unwrap();
    }
    zstd::encode_all(raw.as_slice(), 3).unwrap()
}

fn package(path: &str, bytes: &[u8], hashed: &str) -> Vec<u8> {
    let member = format!("usr/share/omarchy/{path}");
    archive(&[
        (".MTREE", &mtree(&[(member.as_str(), hashed)])),
        (member.as_str(), bytes),
    ])
}

struct Laid {
    root: drift_fixtures::MaterializedRoot,
}

fn lay(fill: impl FnOnce(&mut RootBuilder)) -> Laid {
    let mut builder = RootBuilder::new();
    fill(&mut builder);
    Laid {
        root: builder.materialize().unwrap(),
    }
}

fn paths(tree: &Laid) -> ResolvePaths {
    let root = tree.root.path();
    ResolvePaths {
        share: root.join("share"),
        state: root.join("state"),
        snapshots: root.join("snapshots"),
        cache: root.join("cache"),
        log: root.join("var/log/pacman.log"),
    }
}

fn query<'a>(root: &'a str, rel: &'a RelPath, template: &'a RelPath) -> BaseQuery<'a> {
    BaseQuery {
        root,
        rel,
        template,
    }
}

fn rel(path: &str) -> RelPath {
    RelPath::new(path).unwrap()
}

#[test]
fn each_source_wins_in_order_and_a_miss_is_none() {
    let cached_hash = digest(CACHED);
    let other_hash = digest(OTHER);
    let built = lay(|builder| {
        builder
            .file("share/bases/config/hypr/reviewed.lua", REVIEWED)
            .unwrap()
            .file("state/reviewed-version", "1.0.0-1\n")
            .unwrap()
            .file("snapshots/1.0.0-1/config/hypr/snap.lua", SNAPSHOT)
            .unwrap()
            .file("snapshots/1.0.0-1/config/hypr/reviewed.lua", b"older\n")
            .unwrap()
            .file(
                "cache/omarchy-settings-1.0.0-1-x86_64.pkg.tar.zst",
                archive(&[
                    (
                        ".MTREE",
                        &mtree(&[
                            (
                                "usr/share/omarchy/config/hypr/cache.lua",
                                cached_hash.as_str(),
                            ),
                            (
                                "usr/share/omarchy/config/hypr/snap.lua",
                                other_hash.as_str(),
                            ),
                            (
                                "usr/share/omarchy/config/hypr/tampered.lua",
                                digest(CACHED).as_str(),
                            ),
                        ]),
                    ),
                    ("usr/share/omarchy/config/hypr/cache.lua", CACHED),
                    ("usr/share/omarchy/config/hypr/snap.lua", OTHER),
                    ("usr/share/omarchy/config/hypr/tampered.lua", b"tampered\n"),
                ]),
            )
            .unwrap()
            .file(
                "var/log/pacman.log",
                "[2026-04-01T00:00:00+0000] [ALPM] upgraded omarchy-settings (1.0.0-1 -> 1.2.0-1)\n",
            )
            .unwrap();
    });
    let rel_reviewed = rel("hypr/reviewed.lua");
    let rel_snap = rel("hypr/snap.lua");
    let rel_cache = rel("hypr/cache.lua");
    let rel_tampered = rel("hypr/tampered.lua");
    let rel_missing = rel("hypr/missing.lua");
    let template_reviewed = rel("config/hypr/reviewed.lua");
    let template_snap = rel("config/hypr/snap.lua");
    let template_cache = rel("config/hypr/cache.lua");
    let template_tampered = rel("config/hypr/tampered.lua");
    let template_missing = rel("config/hypr/missing.lua");
    let files = [
        query("config", &rel_reviewed, &template_reviewed),
        query("config", &rel_snap, &template_snap),
        query("config", &rel_cache, &template_cache),
        query("config", &rel_tampered, &template_tampered),
        query("config", &rel_missing, &template_missing),
    ];
    let resolved = resolve_bases(&paths(&built), "1.2.0-1", &files).unwrap();
    assert_eq!(resolved[0].provenance, BaseProvenance::Reviewed);
    assert_eq!(resolved[0].bytes.as_deref(), Some(REVIEWED));
    assert_eq!(resolved[1].provenance, BaseProvenance::Snapshot);
    assert_eq!(resolved[1].bytes.as_deref(), Some(SNAPSHOT));
    assert_eq!(resolved[2].provenance, BaseProvenance::Cache);
    assert_eq!(resolved[2].bytes.as_deref(), Some(CACHED));
    assert_eq!(resolved[3].provenance, BaseProvenance::None);
    assert_eq!(resolved[3].bytes, None);
    assert_eq!(resolved[4].provenance, BaseProvenance::None);
    assert_eq!(resolved[4].bytes, None);
}

#[test]
fn the_recorded_version_is_tried_before_the_log_and_a_link_is_ignored() {
    let cached_hash = digest(CACHED);
    let archive_bytes = package("config/hypr/a.lua", CACHED, &cached_hash);
    let built = lay(|builder| {
        builder
            .file("state/reviewed-version", "1.0.0-1\n")
            .unwrap()
            .file("snapshots/1.1.0-1/config/hypr/a.lua", SNAPSHOT)
            .unwrap()
            .file("outside/secret", b"secret\n")
            .unwrap()
            .file(
                "cache/omarchy-settings-1.0.0-1-x86_64.pkg.tar.zst",
                &archive_bytes,
            )
            .unwrap()
            .link(
                "snapshots/1.0.0-1/config/hypr/linked.lua",
                "../../../../outside/secret",
            )
            .unwrap()
            .file(
                "var/log/pacman.log",
                "[2026-04-01T00:00:00+0000] [ALPM] upgraded omarchy-settings (1.1.0-1 -> 1.2.0-1)\n",
            )
            .unwrap();
    });
    let rel_file = rel("hypr/a.lua");
    let template = rel("config/hypr/a.lua");
    let resolved = resolve_bases(
        &paths(&built),
        "1.2.0-1",
        &[query("config", &rel_file, &template)],
    )
    .unwrap();
    assert_eq!(resolved[0].provenance, BaseProvenance::Cache);
    assert_eq!(resolved[0].bytes.as_deref(), Some(CACHED));
    let linked_rel = rel("hypr/linked.lua");
    let linked_template = rel("config/hypr/linked.lua");
    let linked = resolve_bases(
        &paths(&built),
        "1.2.0-1",
        &[query("config", &linked_rel, &linked_template)],
    )
    .unwrap();
    assert_eq!(linked[0].provenance, BaseProvenance::None);
    assert_eq!(
        std::fs::read(built.root.path().join("outside/secret")).unwrap(),
        b"secret\n"
    );
}

#[test]
fn a_second_root_does_not_reuse_another_roots_reviewed_base() {
    let cached_hash = digest(OTHER);
    let built = lay(|builder| {
        builder
            .file("share/bases/config/shared.txt", REVIEWED)
            .unwrap()
            .file("snapshots/1.0.0-1/applications/shared.txt", SNAPSHOT)
            .unwrap()
            .file(
                "cache/omarchy-settings-1.0.0-1-x86_64.pkg.tar.zst",
                package("applications/shared.txt", OTHER, &cached_hash),
            )
            .unwrap()
            .file(
                "var/log/pacman.log",
                "[2026-04-01T00:00:00+0000] [ALPM] upgraded omarchy-settings (1.0.0-1 -> 1.2.0-1)\n",
            )
            .unwrap();
    });
    let rel_path = rel("shared.txt");
    let config_template = rel("config/shared.txt");
    let apps_template = rel("applications/shared.txt");
    let resolved = resolve_bases(
        &paths(&built),
        "1.2.0-1",
        &[
            query("config", &rel_path, &config_template),
            query("applications", &rel_path, &apps_template),
        ],
    )
    .unwrap();
    assert_eq!(resolved[0].provenance, BaseProvenance::Reviewed);
    assert_eq!(resolved[0].bytes.as_deref(), Some(REVIEWED));
    assert_eq!(resolved[1].provenance, BaseProvenance::Snapshot);
    assert_eq!(resolved[1].bytes.as_deref(), Some(SNAPSHOT));
}

#[test]
fn without_a_recorded_version_the_log_selects_the_snapshot() {
    let built = lay(|builder| {
        builder
            .file("snapshots/1.1.0-1/config/hypr/a.lua", SNAPSHOT)
            .unwrap()
            .file(
                "cache/omarchy-settings-1.0.0-1-x86_64.pkg.tar.zst",
                package("config/hypr/a.lua", CACHED, &digest(CACHED)),
            )
            .unwrap()
            .file(
                "var/log/pacman.log",
                "[2026-04-01T00:00:00+0000] [ALPM] upgraded omarchy-settings (1.1.0-1 -> 1.2.0-1)\n",
            )
            .unwrap();
    });
    let rel_file = rel("hypr/a.lua");
    let template = rel("config/hypr/a.lua");
    let resolved = resolve_bases(
        &paths(&built),
        "1.2.0-1",
        &[query("config", &rel_file, &template)],
    )
    .unwrap();
    assert_eq!(resolved[0].provenance, BaseProvenance::Snapshot);
    assert_eq!(resolved[0].bytes.as_deref(), Some(SNAPSHOT));
}
