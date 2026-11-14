use sha2::{Digest, Sha256};

use crate::error::Error;
use crate::merge::merge3;
use crate::path::RelPath;
use crate::plan::{Action, Plan, PlannedFile};
use crate::plan_view::FileContents;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveInput {
    pub path: String,
    pub root: String,
    pub rel: RelPath,
    pub volatile: bool,
    pub link: bool,
    pub unreadable: bool,
    pub base: Option<Vec<u8>>,
    pub ours: Option<Vec<u8>>,
    pub theirs: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    NoTemplate,
    NothingToDo,
    Symlink,
    Unreadable,
    Volatile,
    FileMissing,
    NoBase,
    NotText,
    Conflicts,
}

impl SkipReason {
    pub fn label(self) -> &'static str {
        match self {
            SkipReason::NoTemplate => "has no template",
            SkipReason::NothingToDo => "nothing to do",
            SkipReason::Symlink => "is a symlink",
            SkipReason::Unreadable => "cannot be read",
            SkipReason::Volatile => "is rewritten by its app",
            SkipReason::FileMissing => "does not exist",
            SkipReason::NoBase => "has no base to merge from",
            SkipReason::NotText => "is not text or is over 1 MiB",
            SkipReason::Conflicts => "has conflicting hunks",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skip {
    pub path: String,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseRecord {
    pub root: String,
    pub rel: RelPath,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Write {
    pub path: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub action: Action,
    pub plan: Option<Plan>,
    pub writes: Vec<Write>,
    pub bases: Vec<BaseRecord>,
    pub contents: Vec<FileContents>,
    pub skipped: Vec<Skip>,
}

const MAX_MERGE_BYTES: usize = 1024 * 1024;

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

enum Outcome {
    Skip(SkipReason),
    Keep,
    Write(Vec<u8>),
}

fn refuse(input: &ResolveInput) -> Option<SkipReason> {
    if input.link {
        Some(SkipReason::Symlink)
    } else if input.unreadable {
        Some(SkipReason::Unreadable)
    } else {
        None
    }
}

fn decide(action: Action, input: &ResolveInput) -> Outcome {
    let Some(theirs) = input.theirs.as_ref() else {
        return Outcome::Skip(SkipReason::NoTemplate);
    };
    if let Some(reason) = refuse(input) {
        return Outcome::Skip(reason);
    }
    match action {
        Action::KeepMine => {
            if input.ours.as_ref() == Some(theirs) {
                Outcome::Skip(SkipReason::NothingToDo)
            } else {
                Outcome::Keep
            }
        }
        Action::TakeUpstream => {
            if input.volatile {
                Outcome::Skip(SkipReason::Volatile)
            } else if input.ours.as_ref() == Some(theirs) {
                Outcome::Skip(SkipReason::NothingToDo)
            } else {
                Outcome::Write(theirs.clone())
            }
        }
        Action::Merge => merge(input, theirs),
        Action::Undo => Outcome::Skip(SkipReason::NothingToDo),
    }
}

fn merge(input: &ResolveInput, theirs: &[u8]) -> Outcome {
    if input.volatile {
        return Outcome::Skip(SkipReason::Volatile);
    }
    let Some(ours) = input.ours.as_deref() else {
        return Outcome::Skip(SkipReason::FileMissing);
    };
    if ours == theirs {
        return Outcome::Skip(SkipReason::NothingToDo);
    }
    let Some(base) = input.base.as_deref() else {
        return Outcome::Skip(SkipReason::NoBase);
    };
    if [base, ours, theirs]
        .iter()
        .any(|bytes| bytes.len() > MAX_MERGE_BYTES)
    {
        return Outcome::Skip(SkipReason::NotText);
    }
    let (Ok(base), Ok(ours_text), Ok(theirs_text)) = (
        std::str::from_utf8(base),
        std::str::from_utf8(ours),
        std::str::from_utf8(theirs),
    ) else {
        return Outcome::Skip(SkipReason::NotText);
    };
    let merged = merge3(base, ours_text, theirs_text);
    match merged.auto() {
        Ok(text) if text.as_bytes() == ours => Outcome::Skip(SkipReason::NothingToDo),
        Ok(text) => Outcome::Write(text.into_bytes()),
        Err(_) => Outcome::Skip(SkipReason::Conflicts),
    }
}

pub fn prepare(
    action: Action,
    inputs: &[ResolveInput],
    id: &str,
    time: &str,
    package_version: &str,
    prune: &[&str],
) -> Result<Prepared, Error> {
    prepare_outcomes(action, inputs, id, time, package_version, prune, |input| {
        decide(action, input)
    })
}

pub fn prepare_result(
    input: &ResolveInput,
    result: &[u8],
    id: &str,
    time: &str,
    package_version: &str,
    prune: &[&str],
) -> Result<Prepared, Error> {
    let action = if input.ours.as_deref() == Some(result) {
        Action::KeepMine
    } else {
        Action::Merge
    };
    prepare_outcomes(
        action,
        std::slice::from_ref(input),
        id,
        time,
        package_version,
        prune,
        |input| {
            if input.theirs.is_none() {
                Outcome::Skip(SkipReason::NoTemplate)
            } else if let Some(reason) = refuse(input) {
                Outcome::Skip(reason)
            } else if input.volatile {
                Outcome::Skip(SkipReason::Volatile)
            } else if result.len() > MAX_MERGE_BYTES || std::str::from_utf8(result).is_err() {
                Outcome::Skip(SkipReason::NotText)
            } else if action == Action::KeepMine {
                decide(action, input)
            } else {
                Outcome::Write(result.to_vec())
            }
        },
    )
}

fn prepare_outcomes(
    action: Action,
    inputs: &[ResolveInput],
    id: &str,
    time: &str,
    package_version: &str,
    prune: &[&str],
    outcome: impl Fn(&ResolveInput) -> Outcome,
) -> Result<Prepared, Error> {
    let mut files = Vec::new();
    let mut writes = Vec::new();
    let mut bases = Vec::new();
    let mut contents = Vec::new();
    let mut skipped = Vec::new();
    for input in inputs {
        let outcome = outcome(input);
        let (after, replaces) = match outcome {
            Outcome::Skip(reason) => {
                skipped.push(Skip {
                    path: input.path.clone(),
                    reason,
                });
                continue;
            }
            Outcome::Keep => (input.ours.clone().unwrap_or_default(), false),
            Outcome::Write(bytes) => (bytes, true),
        };
        let before = input.ours.as_deref().map(digest);
        let after_hash = if replaces || input.ours.is_some() {
            Some(digest(&after))
        } else {
            None
        };
        files.push(PlannedFile::new(
            &input.path,
            action,
            before.as_deref(),
            after_hash.as_deref(),
            None,
            None,
        )?);
        let shown = if replaces {
            Some(after.clone())
        } else {
            input.theirs.clone().or_else(|| input.ours.clone())
        };
        contents.push(FileContents {
            path: input.path.clone(),
            before: input.ours.clone(),
            after: shown,
        });
        writes.push(Write {
            path: input.path.clone(),
            bytes: after,
        });
        if let Some(theirs) = input.theirs.clone() {
            bases.push(BaseRecord {
                root: input.root.clone(),
                rel: input.rel.clone(),
                bytes: theirs,
            });
        }
    }
    let plan = if files.is_empty() {
        None
    } else {
        Some(Plan::new(id, time, package_version, files, prune)?)
    };
    Ok(Prepared {
        action,
        plan,
        writes,
        bases,
        contents,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(
        path: &str,
        base: Option<&str>,
        ours: Option<&str>,
        theirs: Option<&str>,
    ) -> ResolveInput {
        ResolveInput {
            path: format!(".config/{path}"),
            root: "config".to_string(),
            rel: RelPath::new(path).unwrap(),
            volatile: false,
            link: false,
            unreadable: false,
            base: base.map(|text| text.as_bytes().to_vec()),
            ours: ours.map(|text| text.as_bytes().to_vec()),
            theirs: theirs.map(|text| text.as_bytes().to_vec()),
        }
    }

    fn run(action: Action, inputs: &[ResolveInput]) -> Prepared {
        prepare(
            action,
            inputs,
            "txn-1",
            "2026-06-01T00:00:00Z",
            "4.5.0-3",
            &[],
        )
        .unwrap()
    }

    fn sha(text: &str) -> String {
        digest(text.as_bytes())
    }

    #[test]
    fn a_resolved_result_keeps_the_original_hash_and_the_shipped_base() {
        let file = input(
            "alacritty/alacritty.toml",
            Some("old\n"),
            Some("mine\n"),
            Some("shipped\n"),
        );
        let prepared = prepare_result(&file, b"edited result\n", "txn-1", "t", "v", &[]).unwrap();
        let plan = prepared.plan.as_ref().unwrap();
        assert_eq!(
            plan.files()[0].path().as_str(),
            ".config/alacritty/alacritty.toml"
        );
        assert_eq!(
            plan.files()[0].sha256_before(),
            Some(sha("mine\n").as_str())
        );
        assert_eq!(
            plan.files()[0].sha256_after(),
            Some(sha("edited result\n").as_str())
        );
        assert_eq!(prepared.writes[0].bytes, b"edited result\n");
        assert_eq!(prepared.bases[0].bytes, b"shipped\n");
        let view = crate::PlanView::build(plan, "STATE", &prepared.contents);
        assert_eq!(
            view.files[0].backup,
            "STATE/backups/txn-1/.config/alacritty/alacritty.toml"
        );
    }

    #[test]
    fn resolved_results_refuse_unsafe_inputs_and_keep_mine_records_without_writing() {
        for (link, volatile, unreadable) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            let mut file = input("a.conf", Some("old"), Some("mine"), Some("shipped"));
            file.link = link;
            file.volatile = volatile;
            file.unreadable = unreadable;
            let prepared = prepare_result(&file, b"result", "txn", "t", "v", &[]).unwrap();
            assert!(prepared.plan.is_none());
            assert!(prepared.writes.is_empty());
            assert_eq!(prepared.skipped.len(), 1);
        }
        let file = input("a.conf", Some("old"), Some("mine"), Some("shipped"));
        let prepared = prepare_result(&file, b"mine", "txn", "t", "v", &[]).unwrap();
        assert_eq!(prepared.action, Action::KeepMine);
        assert_eq!(prepared.bases[0].bytes, b"shipped");
        let plan = prepared.plan.as_ref().unwrap();
        assert_eq!(plan.files()[0].action(), Action::KeepMine);
    }

    #[test]
    fn keep_mine_writes_nothing_but_records_upstream_as_reviewed() {
        let prepared = run(
            Action::KeepMine,
            &[input("a.conf", Some("b\n"), Some("mine\n"), Some("new\n"))],
        );
        let plan = prepared.plan.unwrap();
        let file = &plan.files()[0];
        assert_eq!(file.action(), Action::KeepMine);
        assert_eq!(file.sha256_before(), Some(sha("mine\n").as_str()));
        assert_eq!(file.sha256_after(), Some(sha("mine\n").as_str()));
        assert_eq!(prepared.writes[0].bytes, b"mine\n");
        assert_eq!(prepared.bases.len(), 1);
        assert_eq!(prepared.bases[0].bytes, b"new\n");
        assert_eq!(prepared.bases[0].root, "config");
        assert_eq!(prepared.bases[0].rel.as_str(), "a.conf");
    }

    #[test]
    fn keep_mine_on_a_deleted_file_keeps_it_deleted() {
        let prepared = run(
            Action::KeepMine,
            &[input("a.conf", Some("b\n"), None, Some("new\n"))],
        );
        let plan = prepared.plan.unwrap();
        assert_eq!(plan.files()[0].sha256_before(), None);
        assert_eq!(plan.files()[0].sha256_after(), None);
        assert_eq!(prepared.writes[0].bytes, b"");
        assert_eq!(prepared.bases[0].bytes, b"new\n");
    }

    #[test]
    fn take_upstream_replaces_or_creates_the_file_with_the_template() {
        let prepared = run(
            Action::TakeUpstream,
            &[
                input("a.conf", Some("b\n"), Some("mine\n"), Some("new\n")),
                input("c.conf", None, None, Some("fresh\n")),
            ],
        );
        let plan = prepared.plan.unwrap();
        assert_eq!(plan.files().len(), 2);
        assert_eq!(
            plan.files()[0].sha256_before(),
            Some(sha("mine\n").as_str())
        );
        assert_eq!(plan.files()[0].sha256_after(), Some(sha("new\n").as_str()));
        assert_eq!(plan.files()[1].sha256_before(), None);
        assert_eq!(
            plan.files()[1].sha256_after(),
            Some(sha("fresh\n").as_str())
        );
        assert!(plan.files().iter().all(|file| file.mode().is_none()));
        assert_eq!(prepared.writes[0].bytes, b"new\n");
        assert_eq!(prepared.writes[1].bytes, b"fresh\n");
        assert_eq!(prepared.bases.len(), 2);
        assert_eq!(prepared.contents[1].before, None);
    }

    #[test]
    fn merge_applies_a_clean_three_way_result() {
        let prepared = run(
            Action::Merge,
            &[input(
                "a.conf",
                Some("one\ntwo\nthree\n"),
                Some("ONE\ntwo\nthree\n"),
                Some("one\ntwo\nTHREE\n"),
            )],
        );
        assert_eq!(prepared.writes[0].bytes, b"ONE\ntwo\nTHREE\n");
        let plan = prepared.plan.unwrap();
        assert_eq!(
            plan.files()[0].sha256_after(),
            Some(sha("ONE\ntwo\nTHREE\n").as_str())
        );
        assert_eq!(prepared.bases[0].bytes, b"one\ntwo\nTHREE\n");
    }

    #[test]
    fn merge_skips_conflicts_missing_bases_missing_files_and_non_text() {
        let conflicting = input("a.conf", Some("x\n"), Some("mine\n"), Some("theirs\n"));
        let no_base = input("b.conf", None, Some("mine\n"), Some("theirs\n"));
        let missing = input("c.conf", Some("x\n"), None, Some("theirs\n"));
        let mut binary = input("d.conf", Some("x\n"), Some("mine\n"), Some("theirs\n"));
        binary.ours = Some(vec![0xff, 0xfe]);
        let mut big = input("e.conf", Some("x\n"), Some("mine\n"), Some("theirs\n"));
        big.base = Some(vec![b'a'; 1024 * 1024 + 1]);
        let prepared = run(Action::Merge, &[conflicting, no_base, missing, binary, big]);
        assert!(prepared.plan.is_none());
        assert!(prepared.writes.is_empty());
        assert!(prepared.bases.is_empty());
        let reasons: Vec<SkipReason> = prepared.skipped.iter().map(|skip| skip.reason).collect();
        assert_eq!(
            reasons,
            [
                SkipReason::Conflicts,
                SkipReason::NoBase,
                SkipReason::FileMissing,
                SkipReason::NotText,
                SkipReason::NotText
            ]
        );
    }

    #[test]
    fn merge_with_only_local_changes_has_nothing_to_do() {
        let prepared = run(
            Action::Merge,
            &[input(
                "a.conf",
                Some("one\ntwo\n"),
                Some("ONE\ntwo\n"),
                Some("one\ntwo\n"),
            )],
        );
        assert!(prepared.plan.is_none());
        assert_eq!(prepared.skipped[0].reason, SkipReason::NothingToDo);
    }

    #[test]
    fn keep_mine_hands_the_upstream_text_to_the_plan_view_and_writes_nothing() {
        let prepared = run(
            Action::KeepMine,
            &[input(
                "a.conf",
                Some("b\n"),
                Some("mine\n"),
                Some("theirs\n"),
            )],
        );
        assert_eq!(prepared.contents[0].before.as_deref(), Some(&b"mine\n"[..]));
        assert_eq!(
            prepared.contents[0].after.as_deref(),
            Some(&b"theirs\n"[..])
        );
        assert_eq!(prepared.writes[0].bytes, b"mine\n");
    }

    #[test]
    fn files_that_cannot_be_acted_on_are_skipped_with_a_reason() {
        let mut link = input("a.conf", None, Some("x\n"), Some("y\n"));
        link.link = true;
        let mut unreadable = input("b.conf", None, None, Some("y\n"));
        unreadable.unreadable = true;
        let mut volatile = input("c.conf", Some("x\n"), Some("m\n"), Some("y\n"));
        volatile.volatile = true;
        let user_only = input("d.conf", None, Some("x\n"), None);
        let same = input("e.conf", None, Some("x\n"), Some("x\n"));
        for action in [Action::KeepMine, Action::TakeUpstream, Action::Merge] {
            let prepared = run(
                action,
                &[
                    link.clone(),
                    unreadable.clone(),
                    user_only.clone(),
                    same.clone(),
                ],
            );
            let reasons: Vec<SkipReason> =
                prepared.skipped.iter().map(|skip| skip.reason).collect();
            assert_eq!(
                reasons,
                [
                    SkipReason::Symlink,
                    SkipReason::Unreadable,
                    SkipReason::NoTemplate,
                    SkipReason::NothingToDo
                ],
                "{action:?}"
            );
            assert!(prepared.plan.is_none(), "{action:?}");
        }
        let take = run(Action::TakeUpstream, &[volatile.clone()]);
        assert_eq!(take.skipped[0].reason, SkipReason::Volatile);
        let merge = run(Action::Merge, &[volatile.clone()]);
        assert_eq!(merge.skipped[0].reason, SkipReason::Volatile);
        let keep = run(Action::KeepMine, &[volatile]);
        assert!(keep.plan.is_some());
    }

    #[test]
    fn a_bulk_action_is_one_plan_and_skipped_files_do_not_stop_the_rest() {
        let prepared = run(
            Action::TakeUpstream,
            &[
                input("a.conf", Some("b\n"), Some("m\n"), Some("n\n")),
                input("b.conf", None, Some("x\n"), Some("x\n")),
                input("c.conf", Some("b\n"), Some("m\n"), Some("n2\n")),
            ],
        );
        let plan = prepared.plan.unwrap();
        let paths: Vec<&str> = plan
            .files()
            .iter()
            .map(|file| file.path().as_str())
            .collect();
        assert_eq!(paths, [".config/a.conf", ".config/c.conf"]);
        assert_eq!(plan.id(), "txn-1");
        assert_eq!(prepared.skipped.len(), 1);
        assert_eq!(prepared.skipped[0].path, ".config/b.conf");
        assert_eq!(prepared.contents.len(), 2);
    }

    #[test]
    fn skip_reasons_read_as_sentences_fragments() {
        for reason in [
            SkipReason::NoTemplate,
            SkipReason::NothingToDo,
            SkipReason::Symlink,
            SkipReason::Unreadable,
            SkipReason::Volatile,
            SkipReason::FileMissing,
            SkipReason::NoBase,
            SkipReason::NotText,
            SkipReason::Conflicts,
        ] {
            assert!(!reason.label().is_empty());
        }
    }
}
