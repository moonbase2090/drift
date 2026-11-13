use serde::Serialize;

use crate::plan::{Action, Plan};
use crate::plan_view::{DiffSummary, FileView, PlanView};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryRow {
    pub id: String,
    pub time: String,
    pub package_version: String,
    pub files: usize,
    pub actions: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryView {
    pub rows: Vec<HistoryRow>,
    pub unreadable: Vec<String>,
}

const ORDER: [Action; 4] = [
    Action::TakeUpstream,
    Action::Merge,
    Action::KeepMine,
    Action::Undo,
];

fn actions_of(plan: &Plan) -> String {
    let mut parts = Vec::new();
    for action in ORDER {
        let count = plan
            .files()
            .iter()
            .filter(|file| file.action() == action)
            .count();
        if count > 0 {
            parts.push(format!("{count} {}", action.label()));
        }
    }
    if parts.is_empty() {
        "no file changes".to_string()
    } else {
        parts.join(", ")
    }
}

impl HistoryView {
    pub fn build(entries: &[Plan], unreadable: &[String]) -> HistoryView {
        HistoryView {
            rows: entries
                .iter()
                .map(|plan| HistoryRow {
                    id: plan.id().to_string(),
                    time: plan.time().to_string(),
                    package_version: plan.package_version().to_string(),
                    files: plan.files().len(),
                    actions: actions_of(plan),
                })
                .collect(),
            unreadable: unreadable.to_vec(),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.unreadable.is_empty()
    }

    pub fn render_text(&self) -> String {
        if self.is_empty() {
            return "No transactions yet.\n".to_string();
        }
        let id_width = self
            .rows
            .iter()
            .map(|row| row.id.chars().count())
            .max()
            .unwrap_or(2)
            .max("ID".len());
        let package_width = self
            .rows
            .iter()
            .map(|row| row.package_version.chars().count())
            .max()
            .unwrap_or(7)
            .max("PACKAGE".len());
        let mut out = format!(
            "{:<id_width$}  {:<20}  {:<package_width$}  {:>5}  ACTIONS\n",
            "ID", "TIME", "PACKAGE", "FILES"
        );
        for row in &self.rows {
            out.push_str(&format!(
                "{:<id_width$}  {:<20}  {:<package_width$}  {:>5}  {}\n",
                row.id, row.time, row.package_version, row.files, row.actions
            ));
        }
        for id in &self.unreadable {
            out.push_str(&format!(
                "{id}  (backups/{id}/manifest.json could not be read)\n"
            ));
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoKind {
    Restore,
    Remove,
    AlreadyOriginal,
    Kept,
    Diverged {
        expected: Option<String>,
        found: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoLine {
    pub path: String,
    pub kind: UndoKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoPreview {
    pub target: String,
    pub id: String,
    pub time: String,
    pub package_version: String,
    pub lines: Vec<UndoLine>,
    pub bases: Vec<String>,
    pub prune: Vec<String>,
}

fn short(hash: Option<&String>) -> String {
    match hash {
        Some(hash) => hash.chars().take(12).collect(),
        None => "absent".to_string(),
    }
}

impl UndoPreview {
    pub fn diverged(&self) -> Vec<&str> {
        self.lines
            .iter()
            .filter(|line| matches!(line.kind, UndoKind::Diverged { .. }))
            .map(|line| line.path.as_str())
            .collect()
    }

    pub fn acts_on(&self, line: &UndoLine, forced: &[String]) -> bool {
        match line.kind {
            UndoKind::Restore | UndoKind::Remove => true,
            UndoKind::Diverged { .. } => forced.contains(&line.path),
            UndoKind::AlreadyOriginal | UndoKind::Kept => false,
        }
    }

    pub fn changes(&self, forced: &[String]) -> usize {
        self.lines
            .iter()
            .filter(|line| self.acts_on(line, forced))
            .count()
    }

    pub fn render_body(&self, forced: &[String]) -> String {
        let mut out = format!(
            "undo of {}\nplan {}\ntime {}\npackage {}\n\n",
            self.target, self.id, self.time, self.package_version
        );
        for line in &self.lines {
            let text = match &line.kind {
                UndoKind::Restore => format!("restore          {}\n", line.path),
                UndoKind::Remove => format!("remove           {}\n", line.path),
                UndoKind::AlreadyOriginal => format!("already original {}\n", line.path),
                UndoKind::Kept => {
                    format!("kept             {}  (nothing was written)\n", line.path)
                }
                UndoKind::Diverged { expected, found } => {
                    let tail = if forced.contains(&line.path) {
                        "restored anyway".to_string()
                    } else {
                        format!("left alone; --force {} restores it anyway", line.path)
                    };
                    format!(
                        "changed since    {}  (expected {}, found {}; {tail})\n",
                        line.path,
                        short(expected.as_ref()),
                        short(found.as_ref())
                    )
                }
            };
            out.push_str(&text);
        }
        for base in &self.bases {
            out.push_str(&format!("reviewed base    {base}  (put back as it was)\n"));
        }
        if !self.prune.is_empty() {
            out.push('\n');
            for id in &self.prune {
                out.push_str(&format!("prune  {id}\n"));
            }
        }
        out
    }

    pub fn render_text(&self, forced: &[String]) -> String {
        let mut out = self.render_body(forced);
        out.push_str("\nDry run: nothing was written.\n");
        out
    }

    pub fn plan_view(&self, state_dir: &str, forced: &[String]) -> PlanView {
        let files = self
            .lines
            .iter()
            .filter(|line| self.acts_on(line, forced))
            .map(|line| {
                let (summary, diff) = match &line.kind {
                    UndoKind::Remove => {
                        (DiffSummary::Text("remove the file".to_string()), Vec::new())
                    }
                    UndoKind::Diverged { expected, found } => (
                        DiffSummary::Text("changed since; restored anyway".to_string()),
                        vec![format!(
                            "expected {}, found {}",
                            short(expected.as_ref()),
                            short(found.as_ref())
                        )],
                    ),
                    _ => (
                        DiffSummary::Text("restore the saved original".to_string()),
                        Vec::new(),
                    ),
                };
                FileView {
                    path: line.path.clone(),
                    action: Action::Undo,
                    backup: format!("{state_dir}/backups/{}/{}", self.id, line.path),
                    mode: None,
                    summary,
                    diff,
                    diff_truncated: false,
                }
            })
            .chain(self.bases.iter().map(|base| FileView {
                path: format!("reviewed base {base}"),
                action: Action::Undo,
                backup: "none (only the reviewed base changes)".to_string(),
                mode: None,
                summary: DiffSummary::Text("put the reviewed base back".to_string()),
                diff: Vec::new(),
                diff_truncated: false,
            }))
            .collect();
        PlanView {
            id: self.id.clone(),
            time: self.time.clone(),
            package_version: self.package_version.clone(),
            files,
            prune: self.prune.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::PlannedFile;

    const H1: &str = "73cb3858a687a8494ca3323053016282f3dad39d42cf62ca4e79dda2aac7d9ac";
    const H2: &str = "98ea6e4f216f2fb4b69fff9b3a44842c38686ca685f3f55dc48c5d3fb1107be4";

    fn file(path: &str, action: Action) -> PlannedFile {
        PlannedFile::new(path, action, Some(H1), Some(H2), Some(0o644), None).unwrap()
    }

    fn plan(id: &str, time: &str, files: Vec<PlannedFile>) -> Plan {
        Plan::new(id, time, "4.5.0-3", files, &[]).unwrap()
    }

    #[test]
    fn rows_count_files_by_action_in_a_fixed_order() {
        let entries = [
            plan(
                "resolve-2",
                "2026-06-02T09:00:00Z",
                vec![
                    file("a", Action::KeepMine),
                    file("b", Action::TakeUpstream),
                    file("c", Action::TakeUpstream),
                    file("d", Action::Merge),
                ],
            ),
            plan(
                "undo-1",
                "2026-06-01T09:00:00Z",
                vec![file("e", Action::Undo)],
            ),
            plan("empty", "2026-05-31T09:00:00Z", Vec::new()),
        ];
        let view = HistoryView::build(&entries, &["broken-1".to_string()]);
        assert_eq!(
            view.render_text(),
            "\
ID         TIME                  PACKAGE  FILES  ACTIONS
resolve-2  2026-06-02T09:00:00Z  4.5.0-3      4  2 take upstream, 1 merge, 1 keep mine
undo-1     2026-06-01T09:00:00Z  4.5.0-3      1  1 undo
empty      2026-05-31T09:00:00Z  4.5.0-3      0  no file changes
broken-1  (backups/broken-1/manifest.json could not be read)
"
        );
        assert!(!view.is_empty());
    }

    #[test]
    fn json_lists_the_rows_and_the_unreadable_ids() {
        let entries = [plan(
            "undo-1",
            "2026-06-01T09:00:00Z",
            vec![file("e", Action::Undo)],
        )];
        let view = HistoryView::build(&entries, &["broken-1".to_string()]);
        let value: serde_json::Value = serde_json::from_str(&view.to_json()).unwrap();
        assert_eq!(value["rows"][0]["id"], "undo-1");
        assert_eq!(value["rows"][0]["files"], 1);
        assert_eq!(value["rows"][0]["actions"], "1 undo");
        assert_eq!(value["unreadable"][0], "broken-1");
    }

    #[test]
    fn an_empty_history_says_so() {
        let view = HistoryView::build(&[], &[]);
        assert!(view.is_empty());
        assert_eq!(view.render_text(), "No transactions yet.\n");
    }

    fn preview() -> UndoPreview {
        let line = |path: &str, kind| UndoLine {
            path: path.to_string(),
            kind,
        };
        UndoPreview {
            target: "resolve-2".to_string(),
            id: "undo-9".to_string(),
            time: "2026-06-03T09:00:00Z".to_string(),
            package_version: "4.5.0-3".to_string(),
            lines: vec![
                line(".config/a", UndoKind::Restore),
                line(".config/b", UndoKind::Remove),
                line(".config/c", UndoKind::AlreadyOriginal),
                line(".config/d", UndoKind::Kept),
                line(
                    ".config/e",
                    UndoKind::Diverged {
                        expected: Some(H1.to_string()),
                        found: None,
                    },
                ),
            ],
            bases: vec!["config/a".to_string()],
            prune: Vec::new(),
        }
    }

    #[test]
    fn the_preview_lists_every_file_and_names_the_forced_ones() {
        let view = preview();
        assert_eq!(
            view.render_text(&[]),
            "\
undo of resolve-2
plan undo-9
time 2026-06-03T09:00:00Z
package 4.5.0-3

restore          .config/a
remove           .config/b
already original .config/c
kept             .config/d  (nothing was written)
changed since    .config/e  (expected 73cb3858a687, found absent; left alone; --force .config/e restores it anyway)
reviewed base    config/a  (put back as it was)

Dry run: nothing was written.
"
        );
        let forced = [".config/e".to_string()];
        assert!(view
            .render_body(&forced)
            .contains("found absent; restored anyway)"));
        assert_eq!(view.diverged(), [".config/e"]);
    }

    #[test]
    fn only_restores_removes_and_forced_divergences_count_as_changes() {
        let view = preview();
        assert_eq!(view.changes(&[]), 2);
        assert_eq!(view.changes(&[".config/e".to_string()]), 3);
        assert_eq!(view.changes(&[".config/c".to_string()]), 2);
    }

    #[test]
    fn the_plan_view_keeps_only_the_files_that_would_change_with_their_backups() {
        let view = preview().plan_view("STATE", &[]);
        assert_eq!(view.id, "undo-9");
        let paths: Vec<&str> = view.files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(paths, [".config/a", ".config/b", "reviewed base config/a"]);
        assert_eq!(view.files[0].action, Action::Undo);
        assert_eq!(view.files[0].backup, "STATE/backups/undo-9/.config/a");
        assert_eq!(view.files[0].summary.label(), "restore the saved original");
        assert_eq!(view.files[1].summary.label(), "remove the file");
        assert_eq!(view.files[2].action, Action::Undo);
        assert_eq!(view.files[2].summary.label(), "put the reviewed base back");
        assert_eq!(
            view.files[2].backup,
            "none (only the reviewed base changes)"
        );

        let forced = preview().plan_view("STATE", &[".config/e".to_string()]);
        assert_eq!(forced.files.len(), 4);
        assert_eq!(
            forced.files[2].summary.label(),
            "changed since; restored anyway"
        );
        assert_eq!(
            forced.files[2].diff,
            ["expected 73cb3858a687, found absent"]
        );
    }

    #[test]
    fn a_keep_mine_undo_has_only_a_base_to_put_back_and_still_has_something_to_show() {
        let view = UndoPreview {
            target: "resolve-3".to_string(),
            id: "undo-10".to_string(),
            time: "2026-06-03T09:00:00Z".to_string(),
            package_version: "4.5.0-3".to_string(),
            lines: vec![UndoLine {
                path: ".config/k".to_string(),
                kind: UndoKind::Kept,
            }],
            bases: vec!["config/k".to_string()],
            prune: Vec::new(),
        };
        assert_eq!(view.changes(&[]), 0);
        let plan = view.plan_view("STATE", &[]);
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].path, "reviewed base config/k");
        assert!(view
            .render_body(&[])
            .contains("reviewed base    config/k  (put back as it was)"));
    }

    #[test]
    fn the_transactions_an_undo_would_delete_are_listed_in_the_text_and_the_plan() {
        let mut view = preview();
        view.prune = vec!["resolve-0001".to_string(), "resolve-0002".to_string()];
        let body = view.render_body(&[]);
        assert!(
            body.ends_with("(put back as it was)\n\nprune  resolve-0001\nprune  resolve-0002\n"),
            "{body}"
        );
        assert!(view
            .render_text(&[])
            .ends_with("prune  resolve-0002\n\nDry run: nothing was written.\n"));
        assert_eq!(
            view.plan_view("STATE", &[]).prune,
            ["resolve-0001", "resolve-0002"]
        );
        assert!(preview().plan_view("STATE", &[]).prune.is_empty());
    }
}
