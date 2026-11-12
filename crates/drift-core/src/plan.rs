use serde::Deserialize;
use serde::Serialize;

use crate::error::Error;
use crate::path::RelPath;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    #[serde(rename = "keep mine")]
    KeepMine,
    #[serde(rename = "take upstream")]
    TakeUpstream,
    #[serde(rename = "merge")]
    Merge,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::KeepMine => "keep mine",
            Action::TakeUpstream => "take upstream",
            Action::Merge => "merge",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFile {
    path: RelPath,
    action: Action,
    sha256_before: Option<String>,
    sha256_after: Option<String>,
    mode: Option<u32>,
    backup: Option<RelPath>,
}

impl PlannedFile {
    pub fn new(
        path: &str,
        action: Action,
        sha256_before: Option<&str>,
        sha256_after: Option<&str>,
        mode: Option<u32>,
        backup: Option<&str>,
    ) -> Result<Self, Error> {
        Ok(Self {
            path: RelPath::new(path)?,
            action,
            sha256_before: digest(sha256_before)?,
            sha256_after: digest(sha256_after)?,
            mode,
            backup: match backup {
                Some(path) => Some(RelPath::new(path)?),
                None => None,
            },
        })
    }

    pub fn path(&self) -> &RelPath {
        &self.path
    }

    pub fn action(&self) -> Action {
        self.action
    }

    pub fn sha256_before(&self) -> Option<&str> {
        self.sha256_before.as_deref()
    }

    pub fn sha256_after(&self) -> Option<&str> {
        self.sha256_after.as_deref()
    }

    pub fn mode(&self) -> Option<u32> {
        self.mode
    }

    pub fn backup(&self) -> Option<&RelPath> {
        self.backup.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedFile<'a> {
    pub path: &'a str,
    pub sha256: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    id: String,
    time: String,
    package_version: String,
    files: Vec<PlannedFile>,
    prune: Vec<String>,
}

impl Plan {
    pub fn new(
        id: &str,
        time: &str,
        package_version: &str,
        files: Vec<PlannedFile>,
        prune: &[&str],
    ) -> Result<Self, Error> {
        let id = token(id, "transaction id")?;
        let time = required(time, "time")?;
        let package_version = required(package_version, "package version")?;
        let mut seen = Vec::new();
        for file in &files {
            if seen.iter().any(|path| path == &file.path) {
                return Err(bad(format!("duplicate path {}", file.path)));
            }
            seen.push(file.path.clone());
        }
        let mut prune_ids = Vec::with_capacity(prune.len());
        for id in prune {
            prune_ids.push(token(id, "prune id")?);
        }
        Ok(Self {
            id,
            time,
            package_version,
            files,
            prune: prune_ids,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn time(&self) -> &str {
        &self.time
    }

    pub fn package_version(&self) -> &str {
        &self.package_version
    }

    pub fn files(&self) -> &[PlannedFile] {
        &self.files
    }

    pub fn prune(&self) -> &[String] {
        &self.prune
    }

    pub fn to_json(&self) -> Result<String, Error> {
        serde_json::to_string_pretty(&self.to_raw()).map_err(manifest)
    }

    pub fn from_json(text: &str) -> Result<Self, Error> {
        let raw: RawPlan = serde_json::from_str(text).map_err(manifest)?;
        let mut files = Vec::with_capacity(raw.files.len());
        for file in raw.files {
            files.push(PlannedFile::new(
                &file.path,
                file.action,
                file.sha256_before.as_deref(),
                file.sha256_after.as_deref(),
                file.mode,
                file.backup.as_deref(),
            )?);
        }
        let prune: Vec<&str> = raw.prune.iter().map(String::as_str).collect();
        Self::new(&raw.id, &raw.time, &raw.package_version, files, &prune)
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("plan {}\n", self.id));
        out.push_str(&format!("time {}\n", self.time));
        out.push_str(&format!("package {}\n", self.package_version));
        out.push('\n');
        for (index, file) in self.files.iter().enumerate() {
            if index > 0 {
                out.push('\n');
            }
            out.push_str(&format!("{}  {}\n", file.action.label(), file.path));
            if let Some(backup) = &file.backup {
                out.push_str(&format!("  backup  {backup}\n"));
            }
            if let Some(mode) = file.mode {
                out.push_str(&format!("  mode  {mode:04o}\n"));
            }
            out.push_str(&format!("  before  {}\n", hash_text(file.sha256_before())));
            out.push_str(&format!("  after  {}\n", hash_text(file.sha256_after())));
        }
        if !self.prune.is_empty() {
            out.push('\n');
            for id in &self.prune {
                out.push_str(&format!("prune  {id}\n"));
            }
        }
        out
    }

    pub fn stale<'a>(&'a self, observed: &[ObservedFile<'_>]) -> Vec<&'a RelPath> {
        self.files
            .iter()
            .filter(|file| !hash_matches(file, observed))
            .map(|file| &file.path)
            .collect()
    }

    fn to_raw(&self) -> RawPlan {
        RawPlan {
            id: self.id.clone(),
            time: self.time.clone(),
            package_version: self.package_version.clone(),
            files: self
                .files
                .iter()
                .map(|file| RawFile {
                    path: file.path.as_str().to_string(),
                    action: file.action,
                    sha256_before: file.sha256_before.clone(),
                    sha256_after: file.sha256_after.clone(),
                    mode: file.mode,
                    backup: file.backup.as_ref().map(|path| path.as_str().to_string()),
                })
                .collect(),
            prune: self.prune.clone(),
        }
    }
}

fn hash_matches(file: &PlannedFile, observed: &[ObservedFile<'_>]) -> bool {
    observed
        .iter()
        .find(|item| item.path == file.path.as_str())
        .is_some_and(|item| item.sha256 == file.sha256_before.as_deref())
}

fn hash_text(hash: Option<&str>) -> &str {
    hash.unwrap_or("absent")
}

fn digest(value: Option<&str>) -> Result<Option<String>, Error> {
    let Some(value) = value else {
        return Ok(None);
    };
    let hex = value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
    if hex {
        Ok(Some(value.to_string()))
    } else {
        Err(bad(format!("SHA-256 digest {value:?}")))
    }
}

fn token(value: &str, what: &str) -> Result<String, Error> {
    if value.is_empty()
        || value.contains('/')
        || value.contains('\0')
        || value == "."
        || value == ".."
    {
        return Err(bad(format!("{what} {value:?}")));
    }
    Ok(value.to_string())
}

fn required(value: &str, what: &str) -> Result<String, Error> {
    if value.is_empty() || value.contains('\0') {
        return Err(bad(format!("missing {what}")));
    }
    Ok(value.to_string())
}

fn bad(message: String) -> Error {
    Error::BadPlan { message }
}

fn manifest(source: serde_json::Error) -> Error {
    Error::BadManifest {
        message: source.to_string(),
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlan {
    id: String,
    time: String,
    package_version: String,
    files: Vec<RawFile>,
    prune: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    path: String,
    action: Action,
    sha256_before: Option<String>,
    sha256_after: Option<String>,
    mode: Option<u32>,
    backup: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const BEFORE: &str = "73cb3858a687a8494ca3323053016282f3dad39d42cf62ca4e79dda2aac7d9ac";
    const AFTER: &str = "98ea6e4f216f2fb4b69fff9b3a44842c38686ca685f3f55dc48c5d3fb1107be4";
    const MERGED: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    fn file(
        path: &str,
        action: Action,
        before: Option<&str>,
        after: Option<&str>,
        mode: Option<u32>,
        backup: Option<&str>,
    ) -> PlannedFile {
        PlannedFile::new(path, action, before, after, mode, backup).unwrap()
    }

    fn sample() -> Plan {
        Plan::new(
            "txn-7",
            "2026-04-01T00:00:00Z",
            "4.5.0-3",
            vec![
                file(
                    "config/hypr/monitors.lua",
                    Action::TakeUpstream,
                    Some(BEFORE),
                    Some(AFTER),
                    Some(0o644),
                    Some("backups/txn-7/config/hypr/monitors.lua"),
                ),
                file(
                    "config/hypr/looknfeel.lua",
                    Action::KeepMine,
                    Some(BEFORE),
                    Some(BEFORE),
                    Some(0o644),
                    None,
                ),
                file(
                    "config/omarchy/shell.json",
                    Action::Merge,
                    Some(AFTER),
                    Some(MERGED),
                    Some(0o600),
                    Some("backups/txn-7/config/omarchy/shell.json"),
                ),
                file(
                    "config/hypr/new.lua",
                    Action::TakeUpstream,
                    None,
                    Some(BEFORE),
                    Some(0o644),
                    None,
                ),
            ],
            &["txn-1"],
        )
        .unwrap()
    }

    #[test]
    fn a_plan_round_trips_through_the_manifest() {
        let plan = sample();
        let parsed = Plan::from_json(include_str!("../snapshots/manifest.json")).unwrap();
        assert_eq!(parsed, plan);
        assert_eq!(plan.id(), "txn-7");
        assert_eq!(plan.time(), "2026-04-01T00:00:00Z");
        assert_eq!(plan.package_version(), "4.5.0-3");
        assert_eq!(plan.prune(), ["txn-1"]);
        assert_eq!(plan.files().len(), 4);
        let created = &plan.files()[3];
        assert_eq!(created.path().as_str(), "config/hypr/new.lua");
        assert_eq!(created.action(), Action::TakeUpstream);
        assert_eq!(created.action().label(), "take upstream");
        assert_eq!(created.sha256_before(), None);
        assert_eq!(created.sha256_after(), Some(BEFORE));
        assert_eq!(created.mode(), Some(0o644));
        assert_eq!(created.backup(), None);
        let again = Plan::from_json(&plan.to_json().unwrap()).unwrap();
        assert_eq!(again, plan);
    }

    #[test]
    fn plan_text_matches_the_reviewed_snapshot() {
        assert_eq!(sample().render(), include_str!("../snapshots/plan.txt"));
    }

    #[test]
    fn a_stale_before_hash_is_reported() {
        let plan = sample();
        let observed = [
            ObservedFile {
                path: "config/hypr/monitors.lua",
                sha256: Some(AFTER),
            },
            ObservedFile {
                path: "config/hypr/looknfeel.lua",
                sha256: Some(BEFORE),
            },
            ObservedFile {
                path: "config/hypr/new.lua",
                sha256: None,
            },
            ObservedFile {
                path: "config/other.lua",
                sha256: Some(BEFORE),
            },
        ];
        let stale: Vec<&str> = plan
            .stale(&observed)
            .iter()
            .map(|path| path.as_str())
            .collect();
        assert_eq!(
            stale,
            ["config/hypr/monitors.lua", "config/omarchy/shell.json",]
        );
    }

    #[test]
    fn a_matching_absent_file_is_not_stale() {
        let plan = Plan::new(
            "txn-8",
            "2026-04-01T00:00:00Z",
            "4.5.0-3",
            vec![file(
                "config/hypr/new.lua",
                Action::TakeUpstream,
                None,
                Some(BEFORE),
                None,
                None,
            )],
            &[],
        )
        .unwrap();
        let text = plan.render();
        assert!(!text.contains("prune"));
        assert!(!text.contains("mode"));
        assert!(!text.contains("backup"));
        assert!(text.contains("before  absent"));
        let observed = [ObservedFile {
            path: "config/hypr/new.lua",
            sha256: None,
        }];
        assert!(plan.stale(&observed).is_empty());
    }

    #[test]
    fn rejected_plans_and_manifests_name_the_problem() {
        let err = PlannedFile::new("config/../x", Action::KeepMine, None, None, None, None);
        assert!(err.unwrap_err().to_string().contains(".."));
        let err = PlannedFile::new("config/a", Action::Merge, Some("abc"), None, None, None);
        assert!(err.unwrap_err().to_string().contains("SHA-256"));
        let err = PlannedFile::new(
            "config/a",
            Action::Merge,
            Some(&BEFORE.to_uppercase()),
            None,
            None,
            None,
        );
        assert!(err.is_err());
        let err = Plan::new("", "t", "1", Vec::new(), &[]);
        assert!(err.unwrap_err().to_string().contains("transaction id"));
        let files = vec![
            file(
                "config/a",
                Action::KeepMine,
                Some(BEFORE),
                Some(BEFORE),
                None,
                None,
            ),
            file(
                "config/a",
                Action::KeepMine,
                Some(BEFORE),
                Some(BEFORE),
                None,
                None,
            ),
        ];
        let err = Plan::new("txn-9", "t", "1", files, &[]);
        assert!(err.unwrap_err().to_string().contains("duplicate path"));
        let err = Plan::from_json(
            r#"{"id":"txn-9","time":"t","package_version":"1","files":[{"path":"config/a","action":"undo","sha256_before":null,"sha256_after":null,"mode":null,"backup":null}],"prune":[]}"#,
        );
        assert!(err.unwrap_err().to_string().contains("undo"));
        let err = Plan::from_json(
            r#"{"id":"txn-9","time":"t","package_version":"1","files":[],"prune":[],"extra":1}"#,
        );
        assert!(err.unwrap_err().to_string().contains("extra"));
        assert!(Plan::new("txn-9", "", "1", Vec::new(), &[]).is_err());
        assert!(Plan::new("txn-9", "t", "1", Vec::new(), &["../old"]).is_err());
    }
}
