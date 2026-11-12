use std::io::Write;

use drift_core::{
    HashOnlyReason, LiveState, LiveTreatment, Origin, Registry, RelPath, Root, Treatment,
    Verification, MAX_DIFF_BYTES,
};
use drift_fixtures::RootBuilder;
use drift_source::{enumerate, ReadPaths, SourcePaths};
use flate2::write::GzEncoder;
use flate2::Compression;

const MARKER: &[u8] = b"x\n";
const MARKER_SHA: &str = "73cb3858a687a8494ca3323053016282f3dad39d42cf62ca4e79dda2aac7d9ac";
const OTHER_SHA: &str = "98ea6e4f216f2fb4b69fff9b3a44842c38686ca685f3f55dc48c5d3fb1107be4";

fn gzip(text: &str) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(text.as_bytes()).unwrap();
    encoder.finish().unwrap()
}

fn mtree(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut text = String::from("#mtree\n/set type=file uid=0 gid=0 mode=644\n");
    for (path, digest) in entries {
        text.push_str(&format!("./{path} time=1.0 size=1 sha256digest={digest}\n"));
    }
    gzip(&text)
}

fn desc(version: &str) -> String {
    format!("%NAME%\nomarchy-settings\n\n%VERSION%\n{version}\n")
}

fn file_list(paths: &[String]) -> String {
    let mut text = String::from("%FILES%\n");
    for path in paths {
        text.push_str(path);
        text.push('\n');
    }
    text
}

struct Tree {
    root: drift_fixtures::MaterializedRoot,
}

fn tree(
    version: &str,
    listed: &[String],
    digests: &[(&str, &str)],
    fill: impl FnOnce(&mut RootBuilder),
) -> Tree {
    let mut builder = RootBuilder::new();
    let dir = format!("var/lib/pacman/local/omarchy-settings-{version}");
    builder
        .file(format!("{dir}/desc"), desc(version))
        .unwrap()
        .file(format!("{dir}/files"), file_list(listed))
        .unwrap()
        .file(format!("{dir}/mtree"), mtree(digests))
        .unwrap();
    fill(&mut builder);
    Tree {
        root: builder.materialize().unwrap(),
    }
}

fn paths(tree: &Tree) -> SourcePaths {
    SourcePaths::new(
        ReadPaths::new(tree.root.path(), tree.root.path().join("usr/share/omarchy")),
        tree.root.path().join("home"),
        tree.root.path().join("skel"),
    )
}

fn scan(tree: &Tree) -> Vec<drift_source::Tracked> {
    enumerate(&Registry::v1(), &paths(tree)).unwrap()
}

#[test]
fn forty_nine_config_templates_are_paired() {
    let mut listed = Vec::new();
    let mut digests = Vec::new();
    for index in 0..49 {
        let package_path = format!("usr/share/omarchy/config/n/{index:02}.txt");
        listed.push(package_path.clone());
        digests.push((package_path, MARKER_SHA));
    }
    listed.push("usr/share/omarchy/bin/tool".to_string());
    let digest_refs: Vec<(&str, &str)> = digests
        .iter()
        .map(|(path, digest)| (path.as_str(), *digest))
        .collect();
    let built = tree("9.9.9-1", &listed, &digest_refs, |builder| {
        for index in 0..49 {
            let package_path = format!("usr/share/omarchy/config/n/{index:02}.txt");
            builder.file(package_path.as_str(), MARKER).unwrap();
            if index != 7 {
                builder
                    .file(format!("home/.config/n/{index:02}.txt"), MARKER)
                    .unwrap();
            }
        }
        builder.file("usr/share/omarchy/bin/tool", MARKER).unwrap();
    });
    let tracked = scan(&built);
    let templates: Vec<_> = tracked
        .iter()
        .filter(|item| item.template.is_some())
        .collect();
    assert_eq!(templates.len(), 49);
    assert!(tracked.iter().all(|item| {
        item.template
            .as_ref()
            .is_none_or(|template| template.path().as_str().starts_with("config/"))
    }));
    let missing_live: Vec<_> = templates
        .iter()
        .filter(|item| item.live.is_none())
        .collect();
    assert_eq!(missing_live.len(), 1);
    let missing = missing_live[0].template.as_ref().unwrap();
    assert_eq!(missing.path().as_str(), "config/n/07.txt");
    assert_eq!(missing.live_path().as_str(), ".config/n/07.txt");
    assert_eq!(missing.verification(), Verification::Verified);
    assert!(!missing.has_warning());
    let present = templates
        .iter()
        .find(|item| item.template.as_ref().unwrap().path().as_str() == "config/n/00.txt")
        .unwrap();
    assert_eq!(
        present.live.as_ref().unwrap().path().as_str(),
        ".config/n/00.txt"
    );
    assert_eq!(
        present.live.as_ref().unwrap().treatment(),
        LiveTreatment::Content(Treatment::Diffable)
    );
    assert!(templates
        .iter()
        .all(|item| { !item.template.as_ref().unwrap().is_volatile() }));
}

#[test]
fn a_hash_mismatch_stays_paired_as_a_warning() {
    let listed = vec!["usr/share/omarchy/config/hypr/a.lua".to_string()];
    let built = tree(
        "1.0.0-1",
        &listed,
        &[("usr/share/omarchy/config/hypr/a.lua", OTHER_SHA)],
        |builder| {
            builder
                .file("usr/share/omarchy/config/hypr/a.lua", MARKER)
                .unwrap();
            builder.file("home/.config/hypr/a.lua", MARKER).unwrap();
        },
    );
    let tracked = scan(&built);
    assert_eq!(tracked.len(), 1);
    let template = tracked[0].template.as_ref().unwrap();
    assert_eq!(template.verification(), Verification::Mismatch);
    assert!(template.has_warning());
    assert!(tracked[0].live.is_some());
}

#[test]
fn a_missing_package_file_matches_the_skel_copy() {
    let listed = vec!["usr/share/omarchy/config/hypr/a.lua".to_string()];
    let built = tree(
        "1.1.0-1",
        &listed,
        &[("usr/share/omarchy/config/hypr/a.lua", MARKER_SHA)],
        |builder| {
            builder.file("skel/.config/hypr/a.lua", MARKER).unwrap();
            builder
                .file("skel/.config/hypr/from-skel.lua", b"skel\n")
                .unwrap();
        },
    );
    let tracked = scan(&built);
    assert_eq!(tracked.len(), 1);
    let template = tracked[0].template.as_ref().unwrap();
    assert_eq!(template.path().as_str(), "config/hypr/a.lua");
    assert_eq!(template.verification(), Verification::Verified);
    assert!(!template.has_warning());
    assert_eq!(template.live_path().as_str(), ".config/hypr/a.lua");
    assert!(tracked[0].live.is_none());
}

#[test]
fn a_skel_copy_with_a_different_digest_is_a_mismatch() {
    let listed = vec!["usr/share/omarchy/config/hypr/a.lua".to_string()];
    let built = tree(
        "1.1.1-1",
        &listed,
        &[("usr/share/omarchy/config/hypr/a.lua", OTHER_SHA)],
        |builder| {
            builder.file("skel/.config/hypr/a.lua", MARKER).unwrap();
        },
    );
    let tracked = scan(&built);
    assert_eq!(tracked.len(), 1);
    let template = tracked[0].template.as_ref().unwrap();
    assert_eq!(template.verification(), Verification::Mismatch);
    assert!(template.has_warning());
}

#[test]
fn a_template_missing_from_both_trees_stays_unchecked() {
    let listed = vec!["usr/share/omarchy/config/hypr/a.lua".to_string()];
    let built = tree(
        "1.1.2-1",
        &listed,
        &[("usr/share/omarchy/config/hypr/a.lua", MARKER_SHA)],
        |_| {},
    );
    let tracked = scan(&built);
    assert_eq!(tracked.len(), 1);
    let template = tracked[0].template.as_ref().unwrap();
    assert_eq!(template.verification(), Verification::Unchecked);
    assert!(!template.has_warning());
}

#[test]
fn the_package_file_wins_when_the_skel_copy_differs() {
    let listed = vec!["usr/share/omarchy/config/hypr/a.lua".to_string()];
    let built = tree(
        "1.1.3-1",
        &listed,
        &[("usr/share/omarchy/config/hypr/a.lua", MARKER_SHA)],
        |builder| {
            builder
                .file("usr/share/omarchy/config/hypr/a.lua", MARKER)
                .unwrap();
            builder.file("skel/.config/hypr/a.lua", b"other\n").unwrap();
        },
    );
    let tracked = scan(&built);
    assert_eq!(tracked.len(), 1);
    let template = tracked[0].template.as_ref().unwrap();
    assert_eq!(template.verification(), Verification::Verified);
    assert!(!template.has_warning());
}

#[test]
fn an_unreadable_skel_copy_stays_unchecked() {
    let listed = vec!["usr/share/omarchy/config/hypr/a.lua".to_string()];
    let built = tree(
        "1.1.4-1",
        &listed,
        &[("usr/share/omarchy/config/hypr/a.lua", MARKER_SHA)],
        |builder| {
            builder
                .file_with_mode("skel/.config/hypr/a.lua", b"no\n", 0)
                .unwrap();
        },
    );
    let tracked = scan(&built);
    assert_eq!(tracked.len(), 1);
    assert_eq!(
        tracked[0].template.as_ref().unwrap().verification(),
        Verification::Unchecked
    );
}

#[test]
fn a_live_file_without_a_template_is_kept_beside_one() {
    let listed = vec!["usr/share/omarchy/config/hypr/a.lua".to_string()];
    let built = tree(
        "1.2.0-1",
        &listed,
        &[("usr/share/omarchy/config/hypr/a.lua", MARKER_SHA)],
        |builder| {
            builder
                .file("usr/share/omarchy/config/hypr/a.lua", MARKER)
                .unwrap();
            builder.file("home/.config/hypr/a.lua", MARKER).unwrap();
            builder
                .file("home/.config/hypr/only-mine.lua", b"mine\n")
                .unwrap();
            builder
                .file("home/.config/hypr/sub/deep.lua", b"deep\n")
                .unwrap();
            builder
                .file("home/.config/lonely/x.lua", b"nope\n")
                .unwrap();
        },
    );
    let tracked = scan(&built);
    assert_eq!(tracked.len(), 2);
    let extra = tracked.iter().find(|item| item.template.is_none()).unwrap();
    assert_eq!(
        extra.live.as_ref().unwrap().path().as_str(),
        ".config/hypr/only-mine.lua"
    );
    assert!(!extra.live.as_ref().unwrap().is_volatile());
}

#[test]
fn symlinks_keep_their_destination_and_are_not_followed() {
    let listed = vec!["usr/share/omarchy/config/hypr/a.lua".to_string()];
    let built = tree(
        "1.3.0-1",
        &listed,
        &[("usr/share/omarchy/config/hypr/a.lua", MARKER_SHA)],
        |builder| {
            builder
                .file("usr/share/omarchy/config/hypr/a.lua", MARKER)
                .unwrap();
            builder
                .link("home/.config/hypr/a.lua", "../elsewhere/a.lua")
                .unwrap();
            builder
                .file("home/.config/hypr/elsewhere/a.lua", b"hidden\n")
                .unwrap();
        },
    );
    let tracked = scan(&built);
    let live = tracked
        .iter()
        .find(|item| item.template.is_some())
        .unwrap()
        .live
        .as_ref()
        .unwrap();
    assert_eq!(live.treatment(), LiveTreatment::ReadOnlySymlink);
    assert_eq!(live.link_target(), Some("../elsewhere/a.lua"));
    assert!(!live.is_writable());
    assert!(matches!(
        live.state(),
        LiveState::Symlink { target } if target == "../elsewhere/a.lua"
    ));
}

#[test]
fn unreadable_non_utf8_and_oversize_files_keep_their_treatment() {
    let listed = vec![
        "usr/share/omarchy/config/hypr/secret.lua".to_string(),
        "usr/share/omarchy/config/hypr/binary.lua".to_string(),
        "usr/share/omarchy/config/hypr/huge.lua".to_string(),
    ];
    let digests = [
        ("usr/share/omarchy/config/hypr/secret.lua", MARKER_SHA),
        ("usr/share/omarchy/config/hypr/binary.lua", MARKER_SHA),
        ("usr/share/omarchy/config/hypr/huge.lua", MARKER_SHA),
    ];
    let huge = vec![b'a'; MAX_DIFF_BYTES as usize + 1];
    let built = tree("1.4.0-1", &listed, &digests, |builder| {
        for name in ["secret.lua", "binary.lua", "huge.lua"] {
            builder
                .file(format!("usr/share/omarchy/config/hypr/{name}"), MARKER)
                .unwrap();
        }
        builder
            .file_with_mode("home/.config/hypr/secret.lua", b"no\n", 0)
            .unwrap();
        builder
            .file("home/.config/hypr/binary.lua", [0xff, 0xfe])
            .unwrap();
        builder.file("home/.config/hypr/huge.lua", huge).unwrap();
    });
    let tracked = scan(&built);
    let treatment = |name: &str| {
        tracked
            .iter()
            .find(|item| {
                item.template
                    .as_ref()
                    .is_some_and(|template| template.path().as_str().ends_with(name))
            })
            .unwrap()
            .live
            .as_ref()
            .unwrap()
            .treatment()
    };
    assert_eq!(treatment("secret.lua"), LiveTreatment::Unreadable);
    assert_eq!(
        treatment("binary.lua"),
        LiveTreatment::Content(Treatment::HashOnly(HashOnlyReason::NonUtf8))
    );
    assert_eq!(
        treatment("huge.lua"),
        LiveTreatment::Content(Treatment::HashOnly(HashOnlyReason::Oversize))
    );
}

#[test]
fn the_registry_volatile_file_is_marked_on_both_sides() {
    let listed = vec![
        "usr/share/omarchy/config/chromium/Default/Preferences".to_string(),
        "usr/share/omarchy/config/chromium/Default/Bookmarks".to_string(),
    ];
    let digests = [
        (
            "usr/share/omarchy/config/chromium/Default/Preferences",
            MARKER_SHA,
        ),
        (
            "usr/share/omarchy/config/chromium/Default/Bookmarks",
            MARKER_SHA,
        ),
    ];
    let built = tree("1.5.0-1", &listed, &digests, |builder| {
        for name in ["Preferences", "Bookmarks"] {
            let rel = format!("chromium/Default/{name}");
            builder
                .file(format!("usr/share/omarchy/config/{rel}"), MARKER)
                .unwrap();
            builder.file(format!("home/.config/{rel}"), MARKER).unwrap();
        }
    });
    let tracked = scan(&built);
    let prefs = tracked
        .iter()
        .find(|item| {
            item.template.as_ref().is_some_and(|template| {
                template.path().as_str() == "config/chromium/Default/Preferences"
            })
        })
        .unwrap();
    assert!(prefs.template.as_ref().unwrap().is_volatile());
    assert!(prefs.live.as_ref().unwrap().is_volatile());
    let bookmarks = tracked
        .iter()
        .find(|item| {
            item.template
                .as_ref()
                .is_some_and(|template| template.path().as_str().ends_with("Bookmarks"))
        })
        .unwrap();
    assert!(!bookmarks.template.as_ref().unwrap().is_volatile());
    assert!(!bookmarks.live.as_ref().unwrap().is_volatile());
}

#[test]
fn a_missing_package_is_an_error() {
    let empty = RootBuilder::new().materialize().unwrap();
    let paths = SourcePaths::new(
        ReadPaths::new(empty.path(), empty.path().join("usr/share/omarchy")),
        empty.path().join("home"),
        empty.path().join("skel"),
    );
    assert!(matches!(
        enumerate(&Registry::v1(), &paths),
        Err(drift_source::Error::NotInstalled)
    ));
}

#[test]
fn a_user_root_pairs_a_package_file_the_default_registry_skips() {
    let listed = vec![
        "usr/share/omarchy/applications/demo.desktop".to_string(),
        "usr/share/omarchy/bin/tool".to_string(),
    ];
    let built = tree(
        "1.6.0-1",
        &listed,
        &[
            ("usr/share/omarchy/applications/demo.desktop", MARKER_SHA),
            ("usr/share/omarchy/bin/tool", MARKER_SHA),
        ],
        |builder| {
            builder
                .file("usr/share/omarchy/applications/demo.desktop", MARKER)
                .unwrap();
            builder
                .file("home/.local/share/applications/demo.desktop", MARKER)
                .unwrap();
            builder.file("usr/share/omarchy/bin/tool", MARKER).unwrap();
        },
    );
    let stock = scan(&built);
    assert!(stock.is_empty(), "{}", stock.len());
    let root = Root::new(
        "applications",
        RelPath::new("applications").unwrap(),
        RelPath::new(".local/share/applications").unwrap(),
        Vec::new(),
        Origin::User,
    )
    .unwrap();
    let registry = Registry::v1().with_roots(vec![root]).unwrap();
    let tracked = enumerate(&registry, &paths(&built)).unwrap();
    assert_eq!(tracked.len(), 1);
    let template = tracked[0].template.as_ref().unwrap();
    assert_eq!(template.path().as_str(), "applications/demo.desktop");
    assert_eq!(
        template.live_path().as_str(),
        ".local/share/applications/demo.desktop"
    );
    assert_eq!(template.verification(), Verification::Verified);
    assert_eq!(
        tracked[0].live.as_ref().unwrap().path().as_str(),
        ".local/share/applications/demo.desktop"
    );
}

#[test]
#[ignore = "reads the machine package database and home directory"]
fn live_read_of_templates_and_live_files() {
    let tracked = enumerate(&Registry::v1(), &SourcePaths::system()).unwrap();
    let templates: Vec<_> = tracked
        .iter()
        .filter(|item| item.template.is_some())
        .collect();
    assert_eq!(templates.len(), 49);
    for item in &templates {
        let template = item.template.as_ref().unwrap();
        assert!(
            template.path().as_str().starts_with("config/"),
            "{}",
            template.path()
        );
        assert!(
            template.live_path().as_str().starts_with(".config/"),
            "{}",
            template.live_path()
        );
        let rest = template.path().as_str().trim_start_matches("config");
        assert_eq!(template.live_path().as_str(), format!(".config{rest}"));
    }
    let missing_live = templates.iter().filter(|item| item.live.is_none()).count();
    let symlinks = tracked
        .iter()
        .filter(|item| {
            item.live
                .as_ref()
                .is_some_and(|live| live.link_target().is_some())
        })
        .count();
    let unreadable = tracked
        .iter()
        .filter(|item| {
            item.live
                .as_ref()
                .is_some_and(|live| live.treatment() == LiveTreatment::Unreadable)
        })
        .count();
    let volatile = templates
        .iter()
        .filter(|item| item.template.as_ref().unwrap().is_volatile())
        .count();
    let warnings = templates
        .iter()
        .filter(|item| item.template.as_ref().unwrap().has_warning())
        .count();
    eprintln!(
        "live templates {} missing_live {} symlinks {} unreadable {} volatile {} warnings {}",
        templates.len(),
        missing_live,
        symlinks,
        unreadable,
        volatile,
        warnings
    );
}
