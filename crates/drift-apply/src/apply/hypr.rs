use std::ffi::{OsStr, OsString};
use std::io::ErrorKind;
use std::process::Command;

use drift_core::{Action, Plan};

use super::{apply, ApplyError, ApplyReport, Request, Roots};

pub trait HyprlandCheck {
    fn config_errors(&self) -> HyprlandStatus;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HyprlandStatus {
    Clean,
    Errors(String),
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteFollowup {
    Clear,
    OfferUndo { transaction: String, errors: String },
}

impl WriteFollowup {
    pub fn alert(&self) -> Option<String> {
        match self {
            Self::Clear => None,
            Self::OfferUndo {
                transaction,
                errors,
            } => Some(format!(
                "Hyprland reported config errors after transaction {transaction}:\n{errors}\nUndo this transaction?"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedApply {
    pub report: ApplyReport,
    pub followup: WriteFollowup,
}

pub struct Hyprctl {
    program: OsString,
}

impl Default for Hyprctl {
    fn default() -> Self {
        Self::at(OsStr::new("hyprctl"))
    }
}

impl Hyprctl {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn at(program: &OsStr) -> Self {
        Self {
            program: program.to_os_string(),
        }
    }

    pub fn program(&self) -> &OsStr {
        &self.program
    }
}

impl HyprlandCheck for Hyprctl {
    fn config_errors(&self) -> HyprlandStatus {
        match Command::new(&self.program).arg("configerrors").output() {
            Err(error) if error.kind() == ErrorKind::NotFound => HyprlandStatus::Missing,
            Err(error) => HyprlandStatus::Errors(error.to_string()),
            Ok(output) => reported(&output.stdout),
        }
    }
}

pub fn touches_hypr_config(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    match parts.iter().position(|part| *part == "hypr") {
        Some(0) => parts.len() > 1,
        Some(1) => parts.len() > 2 && matches!(parts[0], "config" | ".config"),
        _ => false,
    }
}

pub fn apply_checked(
    roots: &Roots,
    request: &Request<'_>,
    checker: &dyn HyprlandCheck,
) -> Result<CheckedApply, ApplyError> {
    let report = apply(roots, request)?;
    let followup = follow_write(request.plan, &report.applied, checker);
    Ok(CheckedApply { report, followup })
}

fn follow_write(plan: &Plan, applied: &[String], checker: &dyn HyprlandCheck) -> WriteFollowup {
    if !wrote_hypr(plan, applied) {
        return WriteFollowup::Clear;
    }
    match checker.config_errors() {
        HyprlandStatus::Errors(errors) => WriteFollowup::OfferUndo {
            transaction: plan.id().to_string(),
            errors,
        },
        HyprlandStatus::Clean | HyprlandStatus::Missing => WriteFollowup::Clear,
    }
}

fn wrote_hypr(plan: &Plan, applied: &[String]) -> bool {
    plan.files().iter().any(|file| {
        file.action() != Action::KeepMine
            && applied.iter().any(|path| path == file.path().as_str())
            && touches_hypr_config(file.path().as_str())
    })
}

fn reported(stdout: &[u8]) -> HyprlandStatus {
    let text = String::from_utf8_lossy(stdout);
    let trimmed = text.trim();
    if trimmed.is_empty() || clean_phrase(trimmed) {
        HyprlandStatus::Clean
    } else {
        HyprlandStatus::Errors(trimmed.to_string())
    }
}

fn clean_phrase(text: &str) -> bool {
    matches!(
        text.to_ascii_lowercase().as_str(),
        "ok" | "no config errors" | "config has no errors" | "no errors"
    )
}
