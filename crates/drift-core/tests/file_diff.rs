use drift_core::{BaseProvenance, DiffContent, FileDiff, HashOnlyReason, Status};

fn side(text: Option<&str>) -> DiffContent {
    text.map_or(DiffContent::Absent, |text| DiffContent::Text {
        sha256: "digest".to_string(),
        text: text.to_string(),
    })
}

#[test]
fn every_status_has_a_reviewed_plain_and_json_golden() {
    for (name, status, base, ours, theirs) in [
        (
            "unchanged",
            Status::Unchanged,
            Some("x\n"),
            Some("x\n"),
            Some("x\n"),
        ),
        (
            "upstream",
            Status::UpstreamChanged,
            Some("x\n"),
            Some("x\n"),
            Some("y\n"),
        ),
        (
            "override",
            Status::LocalOverride,
            Some("x\n"),
            Some("y\n"),
            Some("x\n"),
        ),
        (
            "conflict",
            Status::Conflict,
            Some("x\n"),
            Some("y\n"),
            Some("z\n"),
        ),
        ("missing", Status::Missing, Some("x\n"), None, Some("y\n")),
        ("user-only", Status::UserOnly, None, Some("x\n"), None),
    ] {
        let diff = FileDiff {
            path: ".config/a".to_string(),
            status,
            badge: None,
            provenance: if base.is_some() {
                BaseProvenance::Reviewed
            } else {
                BaseProvenance::None
            },
            note: String::new(),
            base: side(base),
            ours: side(ours),
            theirs: side(theirs),
        };
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/file-diff");
        assert_eq!(
            diff.plain_text(),
            std::fs::read_to_string(root.join(format!("{name}.txt"))).unwrap(),
            "{name}"
        );
        assert_eq!(
            diff.to_json(),
            std::fs::read_to_string(root.join(format!("{name}.json"))).unwrap(),
            "{name}"
        );
    }
}

#[test]
fn unavailable_content_and_missing_final_newlines_are_explicit() {
    let diff = FileDiff {
        path: ".config/a".to_string(),
        status: Status::Conflict,
        badge: None,
        provenance: BaseProvenance::Reviewed,
        note: "read-only symlink".to_string(),
        base: DiffContent::HashOnly {
            sha256: "deadbeef".to_string(),
            len: 2,
            reason: HashOnlyReason::NonUtf8,
        },
        ours: DiffContent::Unreadable,
        theirs: side(Some("no newline")),
    };
    let output = diff.plain_text();
    assert!(output.contains("hash-only: non-UTF-8"));
    assert!(output.contains("[unreadable]"));
    assert!(output.contains("no newline\n[no newline at end]"));
    let json: serde_json::Value = serde_json::from_str(&diff.to_json()).unwrap();
    assert_eq!(json["merge_available"], false);
    assert_eq!(json["base"]["sha256"], "deadbeef");
    assert!(json["result"].is_null());
}
