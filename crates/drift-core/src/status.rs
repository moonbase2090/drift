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
}
