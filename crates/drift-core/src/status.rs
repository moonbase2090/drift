pub const MAX_DIFF_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    Unchanged,
    UpstreamChanged,
    LocalOverride,
    Conflict,
    UserOnly,
    Missing,
}

impl Status {
    pub const ALL: [Status; 6] = [
        Status::Unchanged,
        Status::UpstreamChanged,
        Status::LocalOverride,
        Status::Conflict,
        Status::UserOnly,
        Status::Missing,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Status::Unchanged => "unchanged",
            Status::UpstreamChanged => "upstream changed",
            Status::LocalOverride => "local override",
            Status::Conflict => "conflict",
            Status::UserOnly => "user-only",
            Status::Missing => "missing",
        }
    }

    pub fn explain(self) -> &'static str {
        match self {
            Status::Unchanged => "Your file matches the shipped template.",
            Status::UpstreamChanged => {
                "The package changed this file and you did not, so taking the update is safe."
            }
            Status::LocalOverride => {
                "You changed this file and the package did not. It stays as you have it."
            }
            Status::Conflict => {
                "You and the package both changed this file. Review the diff to decide."
            }
            Status::UserOnly => "Only you have this file. The package does not ship it.",
            Status::Missing => "The package ships this file but it is not on disk.",
        }
    }

    pub fn from_label(label: &str) -> Option<Status> {
        Status::ALL.into_iter().find(|s| s.label() == label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Badge {
    MergesCleanly,
    BaseUnknown,
    DeletedByYou,
}

impl Badge {
    pub const ALL: [Badge; 3] = [
        Badge::MergesCleanly,
        Badge::BaseUnknown,
        Badge::DeletedByYou,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Badge::MergesCleanly => "merges cleanly",
            Badge::BaseUnknown => "base unknown",
            Badge::DeletedByYou => "deleted by you",
        }
    }

    pub fn explain(self) -> &'static str {
        match self {
            Badge::MergesCleanly => {
                "Your edits and the package's changes touch different lines, so they merge without a conflict."
            }
            Badge::BaseUnknown => {
                "No earlier copy of the template is recorded, so Drift cannot tell which side changed what."
            }
            Badge::DeletedByYou => "You deleted this file, and the package still ships it.",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BaseProvenance {
    Reviewed,
    Snapshot,
    Cache,
    None,
}

impl BaseProvenance {
    pub const PRECEDENCE: [BaseProvenance; 4] = [
        BaseProvenance::Reviewed,
        BaseProvenance::Snapshot,
        BaseProvenance::Cache,
        BaseProvenance::None,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BaseProvenance::Reviewed => "reviewed",
            BaseProvenance::Snapshot => "snapshot",
            BaseProvenance::Cache => "cache",
            BaseProvenance::None => "none",
        }
    }

    pub fn is_known(self) -> bool {
        self != BaseProvenance::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashOnlyReason {
    NonUtf8,
    Oversize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Treatment {
    Diffable,
    HashOnly(HashOnlyReason),
}

pub fn content_treatment(len: u64, is_utf8: bool) -> Treatment {
    if !is_utf8 {
        Treatment::HashOnly(HashOnlyReason::NonUtf8)
    } else if len > MAX_DIFF_BYTES {
        Treatment::HashOnly(HashOnlyReason::Oversize)
    } else {
        Treatment::Diffable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_labels_match_the_requirements_table() {
        let labels: Vec<&str> = Status::ALL.iter().map(|s| s.label()).collect();
        assert_eq!(
            labels,
            [
                "unchanged",
                "upstream changed",
                "local override",
                "conflict",
                "user-only",
                "missing"
            ]
        );
    }

    #[test]
    fn status_labels_round_trip_and_reject_unknown_text() {
        for status in Status::ALL {
            assert_eq!(Status::from_label(status.label()), Some(status));
        }
        assert_eq!(Status::from_label("Conflict"), None);
        assert_eq!(Status::from_label(""), None);
    }

    #[test]
    fn badge_labels_match_the_requirements_table() {
        let labels: Vec<&str> = Badge::ALL.iter().map(|b| b.label()).collect();
        assert_eq!(labels, ["merges cleanly", "base unknown", "deleted by you"]);
    }

    #[test]
    fn provenance_precedence_puts_reviewed_first_and_none_last() {
        let labels: Vec<&str> = BaseProvenance::PRECEDENCE
            .iter()
            .map(|p| p.label())
            .collect();
        assert_eq!(labels, ["reviewed", "snapshot", "cache", "none"]);
        assert!(BaseProvenance::Snapshot.is_known());
        assert!(!BaseProvenance::None.is_known());
    }

    #[test]
    fn one_mebibyte_is_still_diffable_one_byte_more_is_not() {
        assert_eq!(content_treatment(MAX_DIFF_BYTES, true), Treatment::Diffable);
        assert_eq!(
            content_treatment(MAX_DIFF_BYTES + 1, true),
            Treatment::HashOnly(HashOnlyReason::Oversize)
        );
        assert_eq!(content_treatment(0, true), Treatment::Diffable);
    }

    #[test]
    fn non_utf8_content_is_hash_only_even_when_small() {
        assert_eq!(
            content_treatment(10, false),
            Treatment::HashOnly(HashOnlyReason::NonUtf8)
        );
        assert_eq!(
            content_treatment(MAX_DIFF_BYTES + 1, false),
            Treatment::HashOnly(HashOnlyReason::NonUtf8)
        );
    }

    #[test]
    fn every_status_and_badge_explains_itself_in_one_distinct_sentence() {
        let mut seen = Vec::new();
        for status in Status::ALL {
            let text = status.explain();
            assert!(text.ends_with('.') && text.len() > 20, "{status:?}");
            assert!(!seen.contains(&text), "{status:?}");
            seen.push(text);
        }
        for badge in Badge::ALL {
            let text = badge.explain();
            assert!(text.ends_with('.') && text.len() > 20, "{badge:?}");
            assert!(!seen.contains(&text), "{badge:?}");
            seen.push(text);
        }
    }
}
