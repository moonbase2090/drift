use similar::{ChangeTag, TextDiff};

use crate::plan::{Action, Plan, PlannedFile};
use crate::status::{content_treatment, HashOnlyReason, Treatment};
use crate::unified_diff;

pub const MAX_DIFF_LINES: usize = 400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileContents {
    pub path: String,
    pub before: Option<Vec<u8>>,
    pub after: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashOnly {
    NotUtf8,
    TooLarge,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffSummary {
    NotWritten,
    NewFile { lines: usize },
    Unchanged,
    Changed { added: usize, removed: usize },
    HashOnly(HashOnly),
}

impl DiffSummary {
    pub fn label(&self) -> String {
        match self {
            DiffSummary::NotWritten => "not written".to_string(),
            DiffSummary::NewFile { lines } => format!("new file, {lines} lines"),
            DiffSummary::Unchanged => "no content change".to_string(),
            DiffSummary::Changed { added, removed } => format!("+{added} -{removed}"),
            DiffSummary::HashOnly(HashOnly::NotUtf8) => "hash only, not UTF-8".to_string(),
            DiffSummary::HashOnly(HashOnly::TooLarge) => "hash only, larger than 1 MiB".to_string(),
            DiffSummary::HashOnly(HashOnly::Unavailable) => "contents not available".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileView {
    pub path: String,
    pub action: Action,
    pub backup: String,
    pub mode: Option<u32>,
    pub summary: DiffSummary,
    pub diff: Vec<String>,
    pub diff_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanView {
    pub id: String,
    pub time: String,
    pub package_version: String,
    pub files: Vec<FileView>,
    pub prune: Vec<String>,
}

impl PlanView {
    pub fn build(plan: &Plan, state_dir: &str, contents: &[FileContents]) -> PlanView {
        let files = plan
            .files()
            .iter()
            .map(|planned| {
                let found = contents
                    .iter()
                    .find(|item| item.path == planned.path().as_str());
                file_view(plan.id(), state_dir, planned, found)
            })
            .collect();
        PlanView {
            id: plan.id().to_string(),
            time: plan.time().to_string(),
            package_version: plan.package_version().to_string(),
            files,
            prune: plan.prune().to_vec(),
        }
    }

    pub fn writes(&self) -> usize {
        self.files
            .iter()
            .filter(|file| file.action != Action::KeepMine)
            .count()
    }

    pub fn render_text(&self) -> String {
        let mut out = format!(
            "plan {}\ntime {}\npackage {}\n",
            self.id, self.time, self.package_version
        );
        for file in &self.files {
            out.push('\n');
            out.push_str(&format!(
                "{}  {}  ({})\n  backup  {}\n",
                file.action.label(),
                file.path,
                file.summary.label(),
                file.backup
            ));
            for line in &file.diff {
                out.push_str(&format!("  {line}\n"));
            }
            if file.diff_truncated {
                out.push_str(&format!("  (diff cut at {MAX_DIFF_LINES} lines)\n"));
            }
        }
        if !self.prune.is_empty() {
            out.push('\n');
            for id in &self.prune {
                out.push_str(&format!("prune  {id}\n"));
            }
        }
        out.push_str("\nDry run: nothing was written.\n");
        out
    }
}

fn file_view(
    id: &str,
    state_dir: &str,
    planned: &PlannedFile,
    contents: Option<&FileContents>,
) -> FileView {
    let path = planned.path().as_str().to_string();
    let backup = if planned.action() == Action::KeepMine {
        "none (nothing is written)".to_string()
    } else if planned.sha256_before().is_none() {
        "none (new file)".to_string()
    } else {
        match planned.backup() {
            Some(custom) => format!("{state_dir}/{}", custom.as_str()),
            None => format!("{state_dir}/backups/{id}/{path}"),
        }
    };
    let (summary, diff, diff_truncated) = if planned.action() == Action::KeepMine {
        (DiffSummary::NotWritten, Vec::new(), false)
    } else {
        summarise(&path, contents)
    };
    FileView {
        path,
        action: planned.action(),
        backup,
        mode: planned.mode(),
        summary,
        diff,
        diff_truncated,
    }
}

fn text_of(bytes: &[u8]) -> Result<&str, HashOnly> {
    let utf8 = std::str::from_utf8(bytes);
    match content_treatment(bytes.len() as u64, utf8.is_ok()) {
        Treatment::Diffable => Ok(utf8.unwrap_or_default()),
        Treatment::HashOnly(HashOnlyReason::NonUtf8) => Err(HashOnly::NotUtf8),
        Treatment::HashOnly(HashOnlyReason::Oversize) => Err(HashOnly::TooLarge),
    }
}

fn summarise(path: &str, contents: Option<&FileContents>) -> (DiffSummary, Vec<String>, bool) {
    let hash_only = |reason| (DiffSummary::HashOnly(reason), Vec::new(), false);
    let Some(item) = contents else {
        return hash_only(HashOnly::Unavailable);
    };
    let Some(after_bytes) = item.after.as_deref() else {
        return hash_only(HashOnly::Unavailable);
    };
    let after = match text_of(after_bytes) {
        Ok(text) => text,
        Err(reason) => return hash_only(reason),
    };
    let before = match item.before.as_deref().map(text_of) {
        None => None,
        Some(Ok(text)) => Some(text),
        Some(Err(reason)) => return hash_only(reason),
    };
    if before == Some(after) {
        return (DiffSummary::Unchanged, Vec::new(), false);
    }
    let diff = TextDiff::from_lines(before.unwrap_or(""), after);
    let (mut added, mut removed) = (0, 0);
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => added += 1,
            ChangeTag::Delete => removed += 1,
            ChangeTag::Equal => {}
        }
    }
    let body: Vec<String> = unified_diff(path, before, after)
        .lines()
        .skip(2)
        .map(str::to_string)
        .collect();
    let truncated = body.len() > MAX_DIFF_LINES;
    let body = body.into_iter().take(MAX_DIFF_LINES).collect();
    let summary = if before.is_none() {
        DiffSummary::NewFile { lines: added }
    } else {
        DiffSummary::Changed { added, removed }
    };
    (summary, body, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    const H1: &str = "73cb3858a687a8494ca3323053016282f3dad39d42cf62ca4e79dda2aac7d9ac";
    const H2: &str = "98ea6e4f216f2fb4b69fff9b3a44842c38686ca685f3f55dc48c5d3fb1107be4";
    const STATE: &str = "~/.local/state/drift";

    fn file(
        path: &str,
        action: Action,
        before: Option<&str>,
        backup: Option<&str>,
    ) -> crate::PlannedFile {
        crate::PlannedFile::new(path, action, before, Some(H2), Some(0o644), backup).unwrap()
    }

    fn contents(path: &str, before: Option<&str>, after: Option<&str>) -> FileContents {
        FileContents {
            path: path.to_string(),
            before: before.map(|text| text.as_bytes().to_vec()),
            after: after.map(|text| text.as_bytes().to_vec()),
        }
    }

    fn plan(files: Vec<crate::PlannedFile>, prune: &[&str]) -> Plan {
        Plan::new("txn-5", "2026-04-02T10:00:00Z", "4.5.0-3", files, prune).unwrap()
    }

    #[test]
    fn a_changed_file_shows_counts_the_default_backup_and_a_diff() {
        let p = plan(
            vec![file("config/a.lua", Action::TakeUpstream, Some(H1), None)],
            &[],
        );
        let view = PlanView::build(
            &p,
            STATE,
            &[contents(
                "config/a.lua",
                Some("one\ntwo\nthree\n"),
                Some("one\nTWO\nthree\nfour\n"),
            )],
        );
        let f = &view.files[0];
        assert_eq!(
            f.summary,
            DiffSummary::Changed {
                added: 2,
                removed: 1
            }
        );
        assert_eq!(f.summary.label(), "+2 -1");
        assert_eq!(f.backup, "~/.local/state/drift/backups/txn-5/config/a.lua");
        assert_eq!(f.mode, Some(0o644));
        assert!(f.diff.iter().any(|line| line == "-two"), "{:?}", f.diff);
        assert!(f.diff.iter().any(|line| line == "+TWO"));
        assert!(f.diff.iter().any(|line| line == "+four"));
        assert!(f.diff.iter().any(|line| line == " one"));
        assert!(f.diff[0].starts_with("@@"), "{:?}", f.diff);
        assert!(!f
            .diff
            .iter()
            .any(|line| line.starts_with("---") || line.starts_with("+++")));
    }

    #[test]
    fn a_custom_backup_path_is_shown_under_the_state_directory() {
        let p = plan(
            vec![file(
                "config/a.lua",
                Action::Merge,
                Some(H1),
                Some("backups/custom/a.lua"),
            )],
            &[],
        );
        let view = PlanView::build(
            &p,
            STATE,
            &[contents("config/a.lua", Some("a\n"), Some("b\n"))],
        );
        assert_eq!(
            view.files[0].backup,
            "~/.local/state/drift/backups/custom/a.lua"
        );
    }

    #[test]
    fn created_unchanged_and_kept_files_say_so() {
        let p = plan(
            vec![
                file("config/new.lua", Action::TakeUpstream, None, None),
                file("config/same.lua", Action::Merge, Some(H1), None),
                file("config/keep.lua", Action::KeepMine, Some(H1), None),
            ],
            &[],
        );
        let view = PlanView::build(
            &p,
            STATE,
            &[
                contents("config/new.lua", None, Some("a\nb\n")),
                contents("config/same.lua", Some("x\n"), Some("x\n")),
                contents("config/keep.lua", Some("k\n"), Some("k\n")),
            ],
        );
        assert_eq!(view.files[0].summary, DiffSummary::NewFile { lines: 2 });
        assert_eq!(view.files[0].summary.label(), "new file, 2 lines");
        assert_eq!(view.files[0].backup, "none (new file)");
        assert_eq!(view.files[1].summary, DiffSummary::Unchanged);
        assert_eq!(view.files[1].summary.label(), "no content change");
        assert!(view.files[1].diff.is_empty());
        assert_eq!(view.files[2].summary, DiffSummary::NotWritten);
        assert_eq!(view.files[2].summary.label(), "not written");
        assert_eq!(view.files[2].backup, "none (nothing is written)");
        assert_eq!(view.writes(), 2);
    }

    #[test]
    fn binary_and_oversize_files_are_hash_only_with_no_diff() {
        let p = plan(
            vec![
                file("config/bin.dat", Action::TakeUpstream, Some(H1), None),
                file("config/big.txt", Action::TakeUpstream, Some(H1), None),
            ],
            &[],
        );
        let big = "x\n".repeat(600_000);
        let view = PlanView::build(
            &p,
            STATE,
            &[
                FileContents {
                    path: "config/bin.dat".into(),
                    before: Some(vec![0xff, 1]),
                    after: Some(vec![0xff, 2]),
                },
                contents("config/big.txt", Some(&big), Some("small\n")),
            ],
        );
        assert_eq!(
            view.files[0].summary,
            DiffSummary::HashOnly(HashOnly::NotUtf8)
        );
        assert_eq!(view.files[0].summary.label(), "hash only, not UTF-8");
        assert_eq!(
            view.files[1].summary,
            DiffSummary::HashOnly(HashOnly::TooLarge)
        );
        assert_eq!(
            view.files[1].summary.label(),
            "hash only, larger than 1 MiB"
        );
        assert!(view.files.iter().all(|f| f.diff.is_empty()));
    }

    #[test]
    fn a_long_diff_is_capped_and_says_so() {
        let before: String = (0..2000).map(|n| format!("line {n}\n")).collect();
        let after: String = (0..2000).map(|n| format!("LINE {n}\n")).collect();
        let p = plan(
            vec![file("config/a.lua", Action::TakeUpstream, Some(H1), None)],
            &[],
        );
        let view = PlanView::build(
            &p,
            STATE,
            &[contents("config/a.lua", Some(&before), Some(&after))],
        );
        let f = &view.files[0];
        assert_eq!(
            f.summary,
            DiffSummary::Changed {
                added: 2000,
                removed: 2000
            }
        );
        assert_eq!(f.diff.len(), MAX_DIFF_LINES);
        assert!(f.diff_truncated);
    }

    #[test]
    fn missing_contents_leave_the_summary_unknown_not_invented() {
        let p = plan(
            vec![file("config/a.lua", Action::TakeUpstream, Some(H1), None)],
            &[],
        );
        let view = PlanView::build(&p, STATE, &[]);
        assert_eq!(
            view.files[0].summary,
            DiffSummary::HashOnly(HashOnly::Unavailable)
        );
    }

    #[test]
    fn header_and_pruned_transactions_are_carried_over() {
        let p = plan(
            vec![file("config/a.lua", Action::TakeUpstream, Some(H1), None)],
            &["old-1", "old-2"],
        );
        let view = PlanView::build(
            &p,
            STATE,
            &[contents("config/a.lua", Some("a\n"), Some("b\n"))],
        );
        assert_eq!(view.id, "txn-5");
        assert_eq!(view.time, "2026-04-02T10:00:00Z");
        assert_eq!(view.package_version, "4.5.0-3");
        assert_eq!(view.prune, ["old-1", "old-2"]);
    }

    #[test]
    fn the_dry_run_text_lists_everything_and_the_full_diff() {
        let p = plan(
            vec![
                file("config/a.lua", Action::TakeUpstream, Some(H1), None),
                file("config/keep.lua", Action::KeepMine, Some(H1), None),
            ],
            &["old-1"],
        );
        let view = PlanView::build(
            &p,
            STATE,
            &[
                contents("config/a.lua", Some("one\ntwo\n"), Some("one\nTWO\n")),
                contents("config/keep.lua", Some("k\n"), Some("k\n")),
            ],
        );
        assert_eq!(
            view.render_text(),
            "plan txn-5\ntime 2026-04-02T10:00:00Z\npackage 4.5.0-3\n\ntake upstream  config/a.lua  (+1 -1)\n  backup  ~/.local/state/drift/backups/txn-5/config/a.lua\n  @@ -1,2 +1,2 @@\n   one\n  -two\n  +TWO\n\nkeep mine  config/keep.lua  (not written)\n  backup  none (nothing is written)\n\nprune  old-1\n\nDry run: nothing was written.\n"
        );
    }
}
