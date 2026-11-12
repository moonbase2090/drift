use std::io::Write;

use drift_core::{
    HashOnlyReason, LiveState, LiveTreatment, Origin, Registry, RelPath, Root, Treatment,
    Verification, MAX_DIFF_BYTES,
};
use drift_fixtures::RootBuilder;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use drift_source::{
    enumerate, is_backup_name, scan as scan_listing, DirReader, ReadPaths, SourcePaths,
    StdDirReader,
};
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

struct CountingReader {
    reads: RefCell<Vec<PathBuf>>,
}

impl CountingReader {
    fn new() -> Self {
        CountingReader {
            reads: RefCell::new(Vec::new()),
        }
    }

    fn read_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = self.reads.borrow().clone();
        dirs.sort();
        dirs
    }
}

impl DirReader for CountingReader {
    fn children(&self, dir: &Path) -> Option<Vec<String>> {
        self.reads.borrow_mut().push(dir.to_path_buf());
        StdDirReader.children(dir)
    }
}

fn templates_tree(package: &[&str], fill: impl FnOnce(&mut RootBuilder)) -> Tree {
    let listed: Vec<String> = package
        .iter()
        .map(|path| format!("usr/share/omarchy/config/{path}"))
        .collect();
    let digests: Vec<(&str, &str)> = listed.iter().map(|p| (p.as_str(), MARKER_SHA)).collect();
    tree("9.9.9-1", &listed, &digests, |builder| {
        for path in package {
            builder
                .file(format!("usr/share/omarchy/config/{path}"), MARKER)
                .unwrap();
        }
        fill(builder);
    })
}

fn rel(path: &str) -> RelPath {
    RelPath::new(path).unwrap()
}

fn live_paths(tracked: &[drift_source::Tracked]) -> Vec<String> {
    let mut paths: Vec<String> = tracked
        .iter()
        .filter_map(|item| item.live.as_ref())
        .map(|live| live.path().as_str().to_string())
        .collect();
    paths.sort();
    paths
}

#[test]
fn only_directories_that_hold_a_template_are_read() {
    let built = templates_tree(&["hypr/a.lua", "hypr/conf/b.lua"], |builder| {
        builder.file("home/.config/hypr/a.lua", MARKER).unwrap();
        builder.file("home/.config/hypr/mine.lua", MARKER).unwrap();
        builder
            .file("home/.config/hypr/conf/b.lua", MARKER)
            .unwrap();
        builder
            .file("home/.config/hypr/conf/deeper/x.lua", MARKER)
            .unwrap();
        builder
            .file("home/.config/hypr/untracked-dir/y.lua", MARKER)
            .unwrap();
        for index in 0..150 {
            builder
                .file(format!("home/.config/chromium/Default/f{index}"), MARKER)
                .unwrap();
            builder
                .file(format!("home/.config/chromium/Cache/c{index}"), MARKER)
                .unwrap();
        }
        builder
            .file("home/.config/chromium/Local State", MARKER)
            .unwrap();
    });
    let reader = CountingReader::new();
    let listing = scan_listing(&Registry::v1(), &paths(&built), &BTreeSet::new(), &reader).unwrap();
    let home = built.root.path().join("home");
    assert_eq!(
        reader.read_dirs(),
        [home.join(".config/hypr"), home.join(".config/hypr/conf")]
    );
    let live = live_paths(&listing.tracked);
    assert_eq!(
        live,
        [
            ".config/hypr/a.lua",
            ".config/hypr/conf/b.lua",
            ".config/hypr/mine.lua"
        ]
    );
    let user_only: Vec<_> = listing
        .tracked
        .iter()
        .filter(|item| item.template.is_none())
        .collect();
    assert_eq!(user_only.len(), 1);
}

#[test]
fn each_template_directory_is_read_once_however_many_templates_it_holds() {
    let names: Vec<String> = (0..20).map(|i| format!("waybar/t{i}.jsonc")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let built = templates_tree(&refs, |builder| {
        builder
            .file("home/.config/waybar/extra.jsonc", MARKER)
            .unwrap();
    });
    let reader = CountingReader::new();
    scan_listing(&Registry::v1(), &paths(&built), &BTreeSet::new(), &reader).unwrap();
    assert_eq!(reader.read_dirs().len(), 1);
}

#[test]
fn a_missing_template_directory_is_read_without_failing() {
    let built = templates_tree(&["ghost/a.txt"], |_| {});
    let reader = CountingReader::new();
    let listing = scan_listing(&Registry::v1(), &paths(&built), &BTreeSet::new(), &reader).unwrap();
    assert_eq!(reader.read_dirs().len(), 1);
    assert_eq!(listing.tracked.len(), 1);
    assert!(listing.tracked[0].live.is_none());
}

#[test]
fn backup_names_need_a_stem_and_only_digits() {
    for name in [
        "a.bak.1",
        "waybar.bak.1700000000",
        "x.y.bak.007",
        "a.lua.bak.5",
    ] {
        assert!(is_backup_name(name), "{name}");
    }
    for name in [
        "a.bak.",
        "a.bak",
        ".bak.5",
        "a.bak.x1",
        "a.bak.12.old",
        "a.bak.1 2",
        "bak.5",
        "a.bak.\u{661}",
        "",
    ] {
        assert!(!is_backup_name(name), "{name}");
    }
}

#[test]
fn backups_are_grouped_per_directory_and_kept_out_of_the_tree() {
    let built = templates_tree(&["hypr/a.lua", "waybar/c.jsonc"], |builder| {
        builder.file("home/.config/hypr/a.lua", MARKER).unwrap();
        builder
            .file("home/.config/hypr/a.lua.bak.1700", MARKER)
            .unwrap();
        builder
            .file("home/.config/hypr/a.lua.bak.1800", MARKER)
            .unwrap();
        builder
            .file("home/.config/hypr/notes.bak.9", MARKER)
            .unwrap();
        builder
            .file("home/.config/hypr/keep.bak.x", MARKER)
            .unwrap();
        builder
            .file("home/.config/hypr/odd.bak.12.old", MARKER)
            .unwrap();
        builder.file("home/.config/waybar/c.jsonc", MARKER).unwrap();
        builder
            .file("home/.config/waybar/c.jsonc.bak.5", MARKER)
            .unwrap();
    });
    let listing = scan_listing(
        &Registry::v1(),
        &paths(&built),
        &BTreeSet::new(),
        &StdDirReader,
    )
    .unwrap();
    let groups: Vec<(String, Vec<String>)> = listing
        .backups
        .iter()
        .map(|group| {
            (
                group.dir.clone(),
                group
                    .files
                    .iter()
                    .map(|f| f.path().as_str().to_string())
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        groups,
        [
            (
                ".config/hypr".to_string(),
                vec![
                    ".config/hypr/a.lua.bak.1700".to_string(),
                    ".config/hypr/a.lua.bak.1800".to_string(),
                    ".config/hypr/notes.bak.9".to_string()
                ]
            ),
            (
                ".config/waybar".to_string(),
                vec![".config/waybar/c.jsonc.bak.5".to_string()]
            ),
        ]
    );
    assert_eq!(
        live_paths(&listing.tracked),
        [
            ".config/hypr/a.lua",
            ".config/hypr/keep.bak.x",
            ".config/hypr/odd.bak.12.old",
            ".config/waybar/c.jsonc"
        ]
    );
    assert!(listing.ignored.is_empty());
}

#[test]
fn a_directory_without_backups_has_no_group() {
    let built = templates_tree(&["hypr/a.lua"], |builder| {
        builder.file("home/.config/hypr/a.lua", MARKER).unwrap();
    });
    let listing = scan_listing(
        &Registry::v1(),
        &paths(&built),
        &BTreeSet::new(),
        &StdDirReader,
    )
    .unwrap();
    assert!(listing.backups.is_empty());
}

#[test]
fn ignored_paths_move_out_of_the_tree_and_the_backups() {
    let built = templates_tree(&["hypr/a.lua", "hypr/gone.lua"], |builder| {
        builder.file("home/.config/hypr/a.lua", MARKER).unwrap();
        builder.file("home/.config/hypr/mine.lua", MARKER).unwrap();
        builder.file("home/.config/hypr/other.lua", MARKER).unwrap();
        builder.file("home/.config/hypr/old.bak.4", MARKER).unwrap();
        builder
            .file("home/.config/hypr/old2.bak.5", MARKER)
            .unwrap();
    });
    let ignored: BTreeSet<RelPath> = [
        rel(".config/hypr/a.lua"),
        rel(".config/hypr/gone.lua"),
        rel(".config/hypr/mine.lua"),
        rel(".config/hypr/old.bak.4"),
        rel(".config/hypr/never-existed.lua"),
    ]
    .into_iter()
    .collect();
    let listing = scan_listing(&Registry::v1(), &paths(&built), &ignored, &StdDirReader).unwrap();
    assert_eq!(live_paths(&listing.tracked), [".config/hypr/other.lua"]);
    assert_eq!(
        live_paths(&listing.ignored),
        [
            ".config/hypr/a.lua",
            ".config/hypr/mine.lua",
            ".config/hypr/old.bak.4"
        ]
    );
    assert_eq!(listing.ignored.len(), 4);
    assert!(listing
        .ignored
        .iter()
        .any(|item| item.template.is_some() && item.live.is_none()));
    assert_eq!(listing.backups.len(), 1);
    assert_eq!(listing.backups[0].files.len(), 1);
    assert_eq!(
        listing.backups[0].files[0].path().as_str(),
        ".config/hypr/old2.bak.5"
    );
    let reader = CountingReader::new();
    let none = scan_listing(&Registry::v1(), &paths(&built), &BTreeSet::new(), &reader).unwrap();
    assert!(none.ignored.is_empty());
    assert_eq!(none.tracked.len(), 4);
}

#[test]
fn enumerate_still_returns_the_tracked_files_only() {
    let built = templates_tree(&["hypr/a.lua"], |builder| {
        builder.file("home/.config/hypr/a.lua", MARKER).unwrap();
        builder
            .file("home/.config/hypr/a.lua.bak.3", MARKER)
            .unwrap();
        builder.file("home/.config/hypr/mine.lua", MARKER).unwrap();
    });
    let tracked = enumerate(&Registry::v1(), &paths(&built)).unwrap();
    assert_eq!(
        live_paths(&tracked),
        [".config/hypr/a.lua", ".config/hypr/mine.lua"]
    );
}

struct RotatedReader;

impl DirReader for RotatedReader {
    fn children(&self, dir: &Path) -> Option<Vec<String>> {
        let mut names = StdDirReader.children(dir)?;
        names.sort();
        names.rotate_left(1);
        Some(names)
    }
}

#[test]
fn listing_order_does_not_depend_on_the_directory_order() {
    let built = templates_tree(&["hypr/a.lua"], |builder| {
        builder.file("home/.config/hypr/a.lua", MARKER).unwrap();
        builder.file("home/.config/hypr/x.bak.1", MARKER).unwrap();
        builder.file("home/.config/hypr/y.bak.2", MARKER).unwrap();
        builder.file("home/.config/hypr/z.bak.3", MARKER).unwrap();
        builder.file("home/.config/hypr/u1.lua", MARKER).unwrap();
        builder.file("home/.config/hypr/u2.lua", MARKER).unwrap();
        builder.file("home/.config/hypr/u3.lua", MARKER).unwrap();
    });
    let listing = scan_listing(
        &Registry::v1(),
        &paths(&built),
        &BTreeSet::new(),
        &RotatedReader,
    )
    .unwrap();
    let backups: Vec<&str> = listing.backups[0]
        .files
        .iter()
        .map(|f| f.path().as_str())
        .collect();
    assert_eq!(
        backups,
        [
            ".config/hypr/x.bak.1",
            ".config/hypr/y.bak.2",
            ".config/hypr/z.bak.3"
        ]
    );
    assert_eq!(
        live_paths(&listing.tracked),
        [
            ".config/hypr/a.lua",
            ".config/hypr/u1.lua",
            ".config/hypr/u2.lua",
            ".config/hypr/u3.lua"
        ]
    );
}
