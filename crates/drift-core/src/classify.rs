use crate::status::{Badge, BaseProvenance, Status};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeOutcome {
    Clean,
    Overlap,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Base<H> {
    pub hash: H,
    pub provenance: BaseProvenance,
}

impl<H> Base<H> {
    pub fn new(hash: H, provenance: BaseProvenance) -> Option<Base<H>> {
        provenance.is_known().then_some(Base { hash, provenance })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Versions<H> {
    pub base: Option<Base<H>>,
    pub ours: Option<H>,
    pub theirs: Option<H>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Classification {
    pub status: Status,
    pub badge: Option<Badge>,
    pub provenance: BaseProvenance,
}

pub fn classify<H, F>(versions: &Versions<H>, merge: F) -> Option<Classification>
where
    H: PartialEq,
    F: FnOnce() -> MergeOutcome,
{
    let provenance = versions
        .base
        .as_ref()
        .map_or(BaseProvenance::None, |b| b.provenance);
    let result = |status, badge| {
        Some(Classification {
            status,
            badge,
            provenance,
        })
    };
    match (&versions.ours, &versions.theirs) {
        (None, None) => None,
        (Some(_), None) => result(Status::UserOnly, None),
        (None, Some(_)) => {
            let badge = versions.base.as_ref().map(|_| Badge::DeletedByYou);
            result(Status::Missing, badge)
        }
        (Some(ours), Some(theirs)) if ours == theirs => result(Status::Unchanged, None),
        (Some(ours), Some(theirs)) => match &versions.base {
            None => result(Status::LocalOverride, Some(Badge::BaseUnknown)),
            Some(base) if base.hash == *ours => result(Status::UpstreamChanged, None),
            Some(base) if base.hash == *theirs => result(Status::LocalOverride, None),
            Some(_) => match merge() {
                MergeOutcome::Clean => result(Status::UpstreamChanged, Some(Badge::MergesCleanly)),
                MergeOutcome::Overlap | MergeOutcome::Unavailable => result(Status::Conflict, None),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const SAME: u8 = 0;
    const BASE_CHANGED: u8 = 1;
    const OURS_CHANGED: u8 = 2;
    const THEIRS_CHANGED: u8 = 3;

    fn side(kind: &str, changed: u8) -> Option<u8> {
        match kind {
            "absent" => None,
            "same" => Some(SAME),
            "changed" => Some(changed),
            other => panic!("unknown presence {other}"),
        }
    }

    fn versions(base: &str, ours: &str, theirs: &str) -> Versions<u8> {
        Versions {
            base: side(base, BASE_CHANGED)
                .and_then(|hash| Base::new(hash, BaseProvenance::Reviewed)),
            ours: side(ours, OURS_CHANGED),
            theirs: side(theirs, THEIRS_CHANGED),
        }
    }

    fn run(
        base: &str,
        ours: &str,
        theirs: &str,
        outcome: MergeOutcome,
    ) -> (Option<Classification>, bool) {
        let called = Cell::new(false);
        let result = classify(&versions(base, ours, theirs), || {
            called.set(true);
            outcome
        });
        (result, called.get())
    }

    fn expect(
        status: Status,
        badge: Option<Badge>,
        provenance: BaseProvenance,
    ) -> Option<Classification> {
        Some(Classification {
            status,
            badge,
            provenance,
        })
    }

    const REVIEWED: BaseProvenance = BaseProvenance::Reviewed;
    const NONE: BaseProvenance = BaseProvenance::None;

    #[test]
    fn every_presence_cell_has_the_documented_status() {
        let cases: [(&str, &str, &str, Option<Classification>); 27] = [
            ("absent", "absent", "absent", None),
            (
                "absent",
                "absent",
                "same",
                expect(Status::Missing, None, NONE),
            ),
            (
                "absent",
                "absent",
                "changed",
                expect(Status::Missing, None, NONE),
            ),
            (
                "absent",
                "same",
                "absent",
                expect(Status::UserOnly, None, NONE),
            ),
            (
                "absent",
                "same",
                "same",
                expect(Status::Unchanged, None, NONE),
            ),
            (
                "absent",
                "same",
                "changed",
                expect(Status::LocalOverride, Some(Badge::BaseUnknown), NONE),
            ),
            (
                "absent",
                "changed",
                "absent",
                expect(Status::UserOnly, None, NONE),
            ),
            (
                "absent",
                "changed",
                "same",
                expect(Status::LocalOverride, Some(Badge::BaseUnknown), NONE),
            ),
            (
                "absent",
                "changed",
                "changed",
                expect(Status::LocalOverride, Some(Badge::BaseUnknown), NONE),
            ),
            ("same", "absent", "absent", None),
            (
                "same",
                "absent",
                "same",
                expect(Status::Missing, Some(Badge::DeletedByYou), REVIEWED),
            ),
            (
                "same",
                "absent",
                "changed",
                expect(Status::Missing, Some(Badge::DeletedByYou), REVIEWED),
            ),
            (
                "same",
                "same",
                "absent",
                expect(Status::UserOnly, None, REVIEWED),
            ),
            (
                "same",
                "same",
                "same",
                expect(Status::Unchanged, None, REVIEWED),
            ),
            (
                "same",
                "same",
                "changed",
                expect(Status::UpstreamChanged, None, REVIEWED),
            ),
            (
                "same",
                "changed",
                "absent",
                expect(Status::UserOnly, None, REVIEWED),
            ),
            (
                "same",
                "changed",
                "same",
                expect(Status::LocalOverride, None, REVIEWED),
            ),
            (
                "same",
                "changed",
                "changed",
                expect(Status::Conflict, None, REVIEWED),
            ),
            ("changed", "absent", "absent", None),
            (
                "changed",
                "absent",
                "same",
                expect(Status::Missing, Some(Badge::DeletedByYou), REVIEWED),
            ),
            (
                "changed",
                "absent",
                "changed",
                expect(Status::Missing, Some(Badge::DeletedByYou), REVIEWED),
            ),
            (
                "changed",
                "same",
                "absent",
                expect(Status::UserOnly, None, REVIEWED),
            ),
            (
                "changed",
                "same",
                "same",
                expect(Status::Unchanged, None, REVIEWED),
            ),
            (
                "changed",
                "same",
                "changed",
                expect(Status::Conflict, None, REVIEWED),
            ),
            (
                "changed",
                "changed",
                "absent",
                expect(Status::UserOnly, None, REVIEWED),
            ),
            (
                "changed",
                "changed",
                "same",
                expect(Status::Conflict, None, REVIEWED),
            ),
            (
                "changed",
                "changed",
                "changed",
                expect(Status::Conflict, None, REVIEWED),
            ),
        ];
        for (base, ours, theirs, expected) in cases {
            let (got, _) = run(base, ours, theirs, MergeOutcome::Overlap);
            assert_eq!(got, expected, "b-{base} o-{ours} t-{theirs}");
        }
    }

    #[test]
    fn a_clean_merge_turns_both_changed_cells_into_upstream_changed() {
        let merged = expect(
            Status::UpstreamChanged,
            Some(Badge::MergesCleanly),
            REVIEWED,
        );
        for (base, ours, theirs) in [
            ("same", "changed", "changed"),
            ("changed", "same", "changed"),
            ("changed", "changed", "same"),
            ("changed", "changed", "changed"),
        ] {
            let (got, called) = run(base, ours, theirs, MergeOutcome::Clean);
            assert_eq!(got, merged, "b-{base} o-{ours} t-{theirs}");
            assert!(called);
        }
    }

    #[test]
    fn an_unavailable_merge_is_a_conflict() {
        let (got, _) = run("same", "changed", "changed", MergeOutcome::Unavailable);
        assert_eq!(got, expect(Status::Conflict, None, REVIEWED));
    }

    #[test]
    fn the_merge_is_consulted_only_when_both_sides_moved_away_from_base() {
        for base in ["absent", "same", "changed"] {
            for ours in ["absent", "same", "changed"] {
                for theirs in ["absent", "same", "changed"] {
                    let (_, called) = run(base, ours, theirs, MergeOutcome::Clean);
                    let ours_h = side(ours, OURS_CHANGED);
                    let theirs_h = side(theirs, THEIRS_CHANGED);
                    let base_h = side(base, BASE_CHANGED);
                    let both_present_and_differ =
                        ours_h.is_some() && theirs_h.is_some() && ours_h != theirs_h;
                    let moved_away = base_h.is_some() && base_h != ours_h && base_h != theirs_h;
                    assert_eq!(
                        called,
                        both_present_and_differ && moved_away,
                        "b-{base} o-{ours} t-{theirs}"
                    );
                }
            }
        }
    }

    #[test]
    fn provenance_is_carried_for_every_known_source() {
        for provenance in [
            BaseProvenance::Reviewed,
            BaseProvenance::Snapshot,
            BaseProvenance::Cache,
        ] {
            let v = Versions {
                base: Base::new(SAME, provenance),
                ours: Some(SAME),
                theirs: Some(THEIRS_CHANGED),
            };
            let got = classify(&v, || MergeOutcome::Overlap).unwrap();
            assert_eq!(got.provenance, provenance);
            assert_eq!(got.status, Status::UpstreamChanged);
        }
    }

    #[test]
    fn a_base_with_no_provenance_is_no_base() {
        assert!(Base::new(SAME, BaseProvenance::None).is_none());
    }

    #[test]
    fn identical_ours_and_theirs_are_unchanged_whatever_the_base() {
        for base in [None, Base::new(BASE_CHANGED, BaseProvenance::Snapshot)] {
            let v = Versions {
                base,
                ours: Some(SAME),
                theirs: Some(SAME),
            };
            let got = classify(&v, || MergeOutcome::Overlap).unwrap();
            assert_eq!(got.status, Status::Unchanged);
            assert_eq!(got.badge, None);
        }
    }
}
