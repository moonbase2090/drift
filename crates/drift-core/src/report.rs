use serde_json::{json, Value};

use crate::status::{Badge, BaseProvenance, Status};

pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanEntry {
    pub path: String,
    pub template: Option<String>,
    pub status: Status,
    pub badge: Option<Badge>,
    pub base: BaseProvenance,
    pub volatile: bool,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupSummary {
    pub dir: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanReport {
    pub package_version: String,
    pub reviewed_version: Option<String>,
    pub entries: Vec<ScanEntry>,
    pub backups: Vec<BackupSummary>,
    pub ignored: usize,
}

pub fn counts_as_drift(status: Status) -> bool {
    matches!(
        status,
        Status::UpstreamChanged | Status::Conflict | Status::Missing
    )
}

fn key(status: Status) -> &'static str {
    match status {
        Status::Unchanged => "unchanged",
        Status::UpstreamChanged => "upstream_changed",
        Status::LocalOverride => "local_override",
        Status::Conflict => "conflict",
        Status::UserOnly => "user_only",
        Status::Missing => "missing",
    }
}

impl ScanReport {
    pub fn count(&self, status: Status) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.status == status)
            .count()
    }

    pub fn drifted(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| counts_as_drift(entry.status))
            .count()
    }

    pub fn exit_code(&self) -> u8 {
        u8::from(self.drifted() > 0)
    }

    pub fn update_pending(&self) -> bool {
        self.reviewed_version
            .as_deref()
            .is_some_and(|reviewed| reviewed != self.package_version)
    }

    pub fn to_json(&self) -> String {
        let mut counts = serde_json::Map::new();
        for status in Status::ALL {
            counts.insert(key(status).to_string(), json!(self.count(status)));
        }
        let files: Vec<Value> = self
            .entries
            .iter()
            .map(|entry| {
                json!({
                    "path": entry.path,
                    "template": entry.template,
                    "status": entry.status.label(),
                    "badge": entry.badge.map(Badge::label),
                    "base": entry.base.label(),
                    "volatile": entry.volatile,
                    "note": entry.note,
                })
            })
            .collect();
        let backups: Vec<Value> = self
            .backups
            .iter()
            .map(|group| json!({"dir": group.dir, "count": group.count}))
            .collect();
        let value = json!({
            "schema": SCHEMA,
            "package_version": self.package_version,
            "reviewed_version": self.reviewed_version,
            "update_pending": self.update_pending(),
            "drifted": self.drifted(),
            "counts": Value::Object(counts),
            "files": files,
            "backups": backups,
            "ignored": self.ignored,
        });
        let mut text = serde_json::to_string_pretty(&value).unwrap_or_default();
        text.push('\n');
        text
    }

    pub fn bar_json(&self) -> String {
        let drifted = self.drifted();
        let value = if drifted == 0 {
            json!({"text": "", "tooltip": "No configuration drift", "class": "clean", "alt": "clean"})
        } else {
            let noun = if drifted == 1 { "config" } else { "configs" };
            json!({
                "text": drifted.to_string(),
                "tooltip": format!("{drifted} {noun} drifted"),
                "class": "drifted",
                "alt": "drifted",
            })
        };
        let mut text = serde_json::to_string(&value).unwrap_or_default();
        text.push('\n');
        text
    }

    pub fn status_text(&self) -> String {
        let mut out = format!("omarchy-settings {}\n", self.package_version);
        if self.update_pending() {
            out.push_str(&format!(
                "update pending review: {} -> {}\n",
                self.reviewed_version.as_deref().unwrap_or(""),
                self.package_version
            ));
        }
        let drifted = self.drifted();
        let parts: Vec<String> = Status::ALL
            .into_iter()
            .filter(|status| self.count(*status) > 0)
            .map(|status| format!("{} {}", self.count(status), status.label()))
            .collect();
        if drifted == 0 {
            out.push_str("No drift");
        } else {
            out.push_str(&format!("{drifted} drifted"));
        }
        if !parts.is_empty() {
            out.push_str(&format!(" ({})", parts.join(", ")));
        }
        out.push('\n');
        out
    }

    pub fn scan_text(&self) -> String {
        let mut out = self.status_text();
        if self.entries.is_empty() {
            return out;
        }
        out.push('\n');
        let width = Status::ALL
            .iter()
            .map(|status| status.label().len())
            .max()
            .unwrap_or(0);
        for entry in &self.entries {
            out.push_str(&format!(
                "{:<width$}  {}",
                entry.status.label(),
                entry.path,
                width = width
            ));
            if let Some(badge) = entry.badge {
                out.push_str(&format!("  [{}]", badge.label()));
            }
            out.push_str(&format!("  (base: {})", entry.base.label()));
            if !entry.note.is_empty() {
                out.push_str(&format!("  {}", entry.note));
            }
            out.push('\n');
        }
        for group in &self.backups {
            out.push_str(&format!("backups ({})  {}\n", group.count, group.dir));
        }
        if self.ignored > 0 {
            out.push_str(&format!("ignored ({})\n", self.ignored));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, status: Status) -> ScanEntry {
        ScanEntry {
            path: path.to_string(),
            template: Some(format!("config/{path}")),
            status,
            badge: None,
            base: BaseProvenance::None,
            volatile: false,
            note: String::new(),
        }
    }

    fn report(entries: Vec<ScanEntry>) -> ScanReport {
        ScanReport {
            package_version: "4.5.0-3".to_string(),
            reviewed_version: None,
            entries,
            backups: Vec::new(),
            ignored: 0,
        }
    }

    #[test]
    fn only_upstream_changes_conflicts_and_missing_files_are_drift() {
        for status in Status::ALL {
            let expected = matches!(
                status,
                Status::UpstreamChanged | Status::Conflict | Status::Missing
            );
            assert_eq!(counts_as_drift(status), expected, "{status:?}");
        }
    }

    #[test]
    fn the_exit_code_follows_the_drift_count() {
        assert_eq!(report(Vec::new()).exit_code(), 0);
        assert_eq!(
            report(vec![
                entry("a", Status::Unchanged),
                entry("b", Status::LocalOverride),
                entry("c", Status::UserOnly)
            ])
            .exit_code(),
            0
        );
        for status in [Status::UpstreamChanged, Status::Conflict, Status::Missing] {
            assert_eq!(report(vec![entry("a", status)]).exit_code(), 1);
        }
    }

    #[test]
    fn counts_and_drift_are_computed_per_status() {
        let r = report(vec![
            entry("a", Status::Conflict),
            entry("b", Status::Conflict),
            entry("c", Status::Unchanged),
            entry("d", Status::Missing),
        ]);
        assert_eq!(r.count(Status::Conflict), 2);
        assert_eq!(r.count(Status::UserOnly), 0);
        assert_eq!(r.drifted(), 3);
    }

    #[test]
    fn the_bar_is_empty_at_zero_drift_and_counts_otherwise() {
        let clean: Value = serde_json::from_str(&report(Vec::new()).bar_json()).unwrap();
        assert_eq!(clean["text"], "");
        assert_eq!(clean["class"], "clean");
        let one: Value =
            serde_json::from_str(&report(vec![entry("a", Status::Conflict)]).bar_json()).unwrap();
        assert_eq!(one["text"], "1");
        assert_eq!(one["tooltip"], "1 config drifted");
        assert_eq!(one["class"], "drifted");
        let two: Value = serde_json::from_str(
            &report(vec![
                entry("a", Status::Conflict),
                entry("b", Status::Missing),
            ])
            .bar_json(),
        )
        .unwrap();
        assert_eq!(two["text"], "2");
        assert_eq!(two["tooltip"], "2 configs drifted");
        assert!(report(Vec::new()).bar_json().ends_with("}\n"));
        assert_eq!(report(Vec::new()).bar_json().matches('\n').count(), 1);
    }

    #[test]
    fn json_lists_every_count_in_a_fixed_order_and_every_file() {
        let mut r = report(vec![entry("a", Status::UpstreamChanged)]);
        r.entries[0].badge = Some(Badge::MergesCleanly);
        r.entries[0].base = BaseProvenance::Cache;
        r.reviewed_version = Some("4.4.0-1".to_string());
        r.backups.push(BackupSummary {
            dir: "hypr".to_string(),
            count: 2,
        });
        r.ignored = 1;
        let text = r.to_json();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["schema"], 1);
        assert_eq!(value["drifted"], 1);
        assert_eq!(value["update_pending"], true);
        assert_eq!(value["reviewed_version"], "4.4.0-1");
        assert_eq!(value["counts"]["upstream_changed"], 1);
        assert_eq!(value["counts"]["unchanged"], 0);
        assert_eq!(value["files"][0]["status"], "upstream changed");
        assert_eq!(value["files"][0]["badge"], "merges cleanly");
        assert_eq!(value["files"][0]["base"], "cache");
        assert_eq!(value["files"][0]["template"], "config/a");
        assert_eq!(value["backups"][0]["count"], 2);
        assert_eq!(value["ignored"], 1);
        let order: Vec<&str> = text
            .lines()
            .filter(|line| line.starts_with("    \""))
            .map(|line| line.trim().split('"').nth(1).unwrap())
            .collect();
        assert_eq!(
            order,
            [
                "conflict",
                "local_override",
                "missing",
                "unchanged",
                "upstream_changed",
                "user_only"
            ]
        );
    }

    #[test]
    fn a_pending_update_needs_a_different_reviewed_version() {
        let mut r = report(Vec::new());
        assert!(!r.update_pending());
        r.reviewed_version = Some("4.5.0-3".to_string());
        assert!(!r.update_pending());
        r.reviewed_version = Some("4.4.0-1".to_string());
        assert!(r.update_pending());
        assert!(r
            .status_text()
            .contains("update pending review: 4.4.0-1 -> 4.5.0-3"));
    }

    #[test]
    fn the_text_views_summarise_and_list() {
        let mut r = report(vec![
            entry("hypr/a.lua", Status::UpstreamChanged),
            entry("hypr/b.lua", Status::Unchanged),
        ]);
        r.entries[0].badge = Some(Badge::MergesCleanly);
        r.entries[0].base = BaseProvenance::Reviewed;
        r.entries[1].note = "app rewrites this file".to_string();
        r.backups.push(BackupSummary {
            dir: "hypr".to_string(),
            count: 3,
        });
        r.ignored = 2;
        assert_eq!(
            r.status_text(),
            "omarchy-settings 4.5.0-3\n1 drifted (1 unchanged, 1 upstream changed)\n"
        );
        let scan = r.scan_text();
        assert!(
            scan.contains("upstream changed  hypr/a.lua  [merges cleanly]  (base: reviewed)\n"),
            "{scan}"
        );
        assert!(
            scan.contains("unchanged         hypr/b.lua  (base: none)  app rewrites this file\n"),
            "{scan}"
        );
        assert!(scan.contains("backups (3)  hypr\n"), "{scan}");
        assert!(scan.ends_with("ignored (2)\n"), "{scan}");
        assert_eq!(
            report(Vec::new()).status_text(),
            "omarchy-settings 4.5.0-3\nNo drift\n"
        );
        assert_eq!(
            report(Vec::new()).scan_text(),
            report(Vec::new()).status_text()
        );
    }
}
