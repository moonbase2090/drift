use crate::{merge3, Badge, BaseProvenance, HashOnlyReason, HunkKind, Merge, Status};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffContent {
    Absent,
    Unreadable,
    Text {
        sha256: String,
        text: String,
    },
    HashOnly {
        sha256: String,
        len: u64,
        reason: HashOnlyReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub path: String,
    pub status: Status,
    pub badge: Option<Badge>,
    pub provenance: BaseProvenance,
    pub note: String,
    pub base: DiffContent,
    pub ours: DiffContent,
    pub theirs: DiffContent,
}

impl DiffContent {
    fn text(&self) -> Option<&str> {
        match self {
            Self::Text { text, .. } => Some(text),
            _ => None,
        }
    }

    fn json(&self) -> Value {
        match self {
            Self::Absent => json!({"state":"absent"}),
            Self::Unreadable => json!({"state":"unreadable"}),
            Self::Text { sha256, text } => json!({"state":"text","sha256":sha256,"text":text}),
            Self::HashOnly {
                sha256,
                len,
                reason,
            } => {
                json!({"state":"hash-only","sha256":sha256,"bytes":len,"reason":reason_label(*reason)})
            }
        }
    }

    fn shown(&self) -> String {
        match self {
            Self::Text { text, .. } => {
                let mut shown = text.clone();
                if !text.is_empty() && !text.ends_with('\n') {
                    shown.push_str("\n[no newline at end]\n");
                }
                shown
            }
            Self::Absent => "[absent]\n".to_string(),
            Self::Unreadable => "[unreadable]\n".to_string(),
            Self::HashOnly {
                sha256,
                len,
                reason,
            } => format!(
                "[hash-only: {}, {len} bytes, sha256 {sha256}]\n",
                reason_label(*reason)
            ),
        }
    }
}

fn reason_label(reason: HashOnlyReason) -> &'static str {
    match reason {
        HashOnlyReason::NonUtf8 => "non-UTF-8",
        HashOnlyReason::Oversize => "over 1 MiB",
    }
}

fn kind_label(kind: HunkKind) -> &'static str {
    match kind {
        HunkKind::OursOnly => "ours",
        HunkKind::TheirsOnly => "theirs",
        HunkKind::BothSame => "both",
        HunkKind::Conflict => "conflict",
    }
}

impl FileDiff {
    fn merge(&self) -> Option<Merge> {
        Some(merge3(
            self.base.text()?,
            self.ours.text()?,
            self.theirs.text()?,
        ))
    }

    pub fn exit_code(&self) -> u8 {
        u8::from(matches!(
            self.status,
            Status::UpstreamChanged | Status::Conflict | Status::Missing
        ))
    }

    pub fn to_json(&self) -> String {
        let merge = self.merge();
        let hunks:Vec<Value>=merge.iter().flat_map(Merge::hunks).map(|hunk|json!({
            "kind":kind_label(hunk.kind),"base":hunk.base,"ours":hunk.ours,"theirs":hunk.theirs
        })).collect();
        let result = merge.as_ref().and_then(|merge| merge.auto().ok());
        let value = json!({"schema":1,"path":self.path,"status":self.status.label(),
            "badge":self.badge.map(Badge::label),"base_provenance":self.provenance.label(),"note":self.note,
            "base":self.base.json(),"ours":self.ours.json(),"theirs":self.theirs.json(),
            "merge_available":merge.is_some(),"hunks":hunks,"result":result});
        let mut out = serde_json::to_string_pretty(&value).unwrap_or_default();
        out.push('\n');
        out
    }

    pub fn plain_text(&self) -> String {
        let mut out = format!(
            "{}: {} (base: {})\n",
            self.path,
            self.status.label(),
            self.provenance.label()
        );
        if let Some(badge) = self.badge {
            out.push_str(&format!("{}\n", badge.label()));
        }
        if !self.note.is_empty() {
            out.push_str(&format!("{}\n", self.note));
        }
        for (name, side) in [
            ("base", &self.base),
            ("ours", &self.ours),
            ("theirs", &self.theirs),
        ] {
            out.push_str(&format!("\n=== {name} ===\n{}", side.shown()));
        }
        out.push_str("\n=== hunks ===\n");
        if let Some(merge) = self.merge() {
            if merge.hunk_count() == 0 {
                out.push_str("[no changes]\n");
            }
            for (index, hunk) in merge.hunks().enumerate() {
                out.push_str(&format!("{}: {}\n", index + 1, kind_label(hunk.kind)));
            }
            out.push_str("\n=== result ===\n");
            match merge.auto() {
                Ok(result) => {
                    out.push_str(&result);
                    if !result.ends_with('\n') {
                        out.push('\n');
                    }
                }
                Err(_) => out.push_str("[unresolved conflicts]\n"),
            }
        } else {
            out.push_str("[three-way merge unavailable]\n");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> DiffContent {
        DiffContent::Text {
            sha256: "digest".to_string(),
            text: value.to_string(),
        }
    }

    fn sample(status: Status) -> FileDiff {
        FileDiff {
            path: ".config/a".to_string(),
            status,
            badge: None,
            provenance: BaseProvenance::Reviewed,
            note: String::new(),
            base: text("a\nb\n"),
            ours: text("A\nb\n"),
            theirs: text("a\nB\n"),
        }
    }

    #[test]
    fn the_exit_code_matches_the_selected_files_status() {
        for (status, code) in [
            (Status::Unchanged, 0),
            (Status::LocalOverride, 0),
            (Status::UserOnly, 0),
            (Status::Missing, 1),
            (Status::UpstreamChanged, 1),
            (Status::Conflict, 1),
        ] {
            assert_eq!(sample(status).exit_code(), code, "{status:?}");
        }
    }

    #[test]
    fn a_clean_merge_exposes_the_actual_result_and_both_hunks() {
        let diff = sample(Status::UpstreamChanged);
        let json: serde_json::Value = serde_json::from_str(&diff.to_json()).unwrap();
        assert_eq!(json["result"], "A\nB\n");
        assert_eq!(json["hunks"].as_array().unwrap().len(), 2);
        assert_eq!(json["base_provenance"], "reviewed");
        assert_eq!(json["ours"]["text"], "A\nb\n");
        let shown = diff.plain_text();
        assert!(shown.contains("=== base ===\na\nb\n"), "{shown}");
        assert!(shown.contains("=== result ===\nA\nB\n"), "{shown}");
    }

    #[test]
    fn conflicts_are_explicit_and_have_no_automatic_result() {
        let mut diff = sample(Status::Conflict);
        diff.ours = text("left\n");
        diff.theirs = text("right\n");
        let json: serde_json::Value = serde_json::from_str(&diff.to_json()).unwrap();
        assert_eq!(json["hunks"][0]["kind"], "conflict");
        assert!(json["result"].is_null());
        assert!(diff.plain_text().contains("unresolved conflicts"));
    }
}
