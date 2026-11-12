use std::fs;
use std::path::Path;

use drift_core::{
    classify, Badge, Base, BaseProvenance, Classification, MergeOutcome, Status, Versions,
};
use drift_fixtures::{case_dirs, materialize_case, MaterializedCase, Presence, Root};

fn read(materialized: &MaterializedCase, root: Root, rel: &str) -> Option<Vec<u8>> {
    let path = materialized.trees.path(root, rel);
    match fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("{}: {error}", path.display()),
    }
}

fn line_overlap(base: &[u8], ours: &[u8], theirs: &[u8]) -> MergeOutcome {
    let base = String::from_utf8_lossy(base).into_owned();
    let ours = String::from_utf8_lossy(ours).into_owned();
    let theirs = String::from_utf8_lossy(theirs).into_owned();
    let (b, o, t): (Vec<_>, Vec<_>, Vec<_>) = (
        base.lines().collect(),
        ours.lines().collect(),
        theirs.lines().collect(),
    );
    if b.len() != o.len() || b.len() != t.len() {
        return MergeOutcome::Unavailable;
    }
    let overlap = (0..b.len()).any(|i| o[i] != b[i] && t[i] != b[i] && o[i] != t[i]);
    if overlap {
        MergeOutcome::Overlap
    } else {
        MergeOutcome::Clean
    }
}

fn classify_case(materialized: &MaterializedCase) -> Vec<(String, Option<Classification>)> {
    materialized
        .case
        .expected
        .files
        .iter()
        .map(|file| {
            let base = read(materialized, Root::Bases, &file.path);
            let ours = read(materialized, Root::Config, &file.path);
            let theirs = read(materialized, Root::Templates, &file.path);
            let versions = Versions {
                base: base
                    .as_ref()
                    .and_then(|bytes| Base::new(bytes.clone(), BaseProvenance::Reviewed)),
                ours: ours.clone(),
                theirs: theirs.clone(),
            };
            let result = classify(&versions, || {
                line_overlap(
                    base.as_deref().unwrap(),
                    ours.as_deref().unwrap(),
                    theirs.as_deref().unwrap(),
                )
            });
            (file.path.clone(), result)
        })
        .collect()
}

fn in_group(directory: &Path, group: &str) -> bool {
    directory.components().any(|c| c.as_os_str() == group)
}

#[test]
fn every_status_case_classifies_as_its_expected_file_says() {
    let mut checked = 0;
    for directory in case_dirs().unwrap() {
        if !in_group(&directory, "status") {
            continue;
        }
        let materialized = materialize_case(&directory).unwrap();
        let results = classify_case(&materialized);
        for (file, (path, got)) in materialized.case.expected.files.iter().zip(results) {
            assert_eq!(file.path, path);
            let got = got.unwrap_or_else(|| panic!("{} produced no status", directory.display()));
            let want_status = Status::from_label(file.status.as_deref().unwrap()).unwrap();
            let want_badge = file
                .badge
                .as_deref()
                .map(|label| Badge::ALL.into_iter().find(|b| b.label() == label).unwrap());
            assert_eq!(got.status, want_status, "{} {}", directory.display(), path);
            assert_eq!(got.badge, want_badge, "{} {}", directory.display(), path);
            checked += 1;
        }
    }
    assert!(checked >= 10, "only {checked} files checked");
}

#[test]
fn every_status_and_badge_appears_in_the_fixture_cases() {
    let mut statuses = Vec::new();
    let mut badges = Vec::new();
    for directory in case_dirs().unwrap() {
        if !in_group(&directory, "status") {
            continue;
        }
        let materialized = materialize_case(&directory).unwrap();
        for (_, got) in classify_case(&materialized) {
            let got = got.unwrap();
            statuses.push(got.status);
            badges.extend(got.badge);
        }
    }
    for status in Status::ALL {
        assert!(statuses.contains(&status), "{}", status.label());
    }
    for badge in Badge::ALL {
        assert!(badges.contains(&badge), "{}", badge.label());
    }
}

#[test]
fn presence_matrix_cells_classify_from_the_presence_flags() {
    let mut cells = 0;
    for directory in case_dirs().unwrap() {
        if !in_group(&directory, "presence") {
            continue;
        }
        let materialized = materialize_case(&directory).unwrap();
        let file = &materialized.case.expected.files[0];
        let got = classify_case(&materialized)[0].1;
        let base = file.base.unwrap();
        let ours = file.ours.unwrap();
        let theirs = file.theirs.unwrap();
        let name = directory
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        match (ours, theirs) {
            (Presence::Absent, Presence::Absent) => assert!(got.is_none(), "{name}"),
            (Presence::Absent, _) => {
                let got = got.unwrap();
                assert_eq!(got.status, Status::Missing, "{name}");
                assert_eq!(got.badge.is_some(), base != Presence::Absent, "{name}");
            }
            (_, Presence::Absent) => assert_eq!(got.unwrap().status, Status::UserOnly, "{name}"),
            _ => assert!(got.is_some(), "{name}"),
        }
        let want_provenance = if base == Presence::Absent {
            BaseProvenance::None
        } else {
            BaseProvenance::Reviewed
        };
        if let Some(got) = got {
            assert_eq!(got.provenance, want_provenance, "{name}");
        }
        cells += 1;
    }
    assert_eq!(cells, 27);
}
