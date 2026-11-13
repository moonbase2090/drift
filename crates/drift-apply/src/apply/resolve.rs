use std::path::Path;

use drift_core::Prepared;

use super::{apply_locked, ApplyError, CheckedApply, HyprlandCheck, Payload, Request, Roots};
use crate::seed::{record_bases, Recorded};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Executed {
    pub checked: CheckedApply,
    pub recorded: Vec<String>,
    pub unrecorded: Vec<String>,
}

pub fn execute(
    roots: &Roots,
    share: &Path,
    prepared: &Prepared,
    checker: &dyn HyprlandCheck,
) -> Result<Executed, ApplyError> {
    let Some(plan) = prepared.plan.as_ref() else {
        return Err(ApplyError {
            message: "there is nothing to apply".to_string(),
            applied: Vec::new(),
            unapplied: Vec::new(),
        });
    };
    let payloads: Vec<Payload<'_>> = prepared
        .writes
        .iter()
        .map(|write| Payload {
            path: &write.path,
            bytes: &write.bytes,
            volatile: false,
            allow_volatile: false,
            allow_symlink: false,
        })
        .collect();
    let mut recorded = Recorded {
        recorded: Vec::new(),
        failed: Vec::new(),
    };
    let report = apply_locked(
        roots,
        &Request {
            plan,
            files: &payloads,
            fail_at: None,
        },
        &mut |_| recorded = record_bases(share, &prepared.bases),
    )?;
    let followup = super::hypr::follow_write(plan, &report.applied, checker);
    let checked = CheckedApply { report, followup };
    Ok(Executed {
        checked,
        recorded: recorded.recorded,
        unrecorded: recorded.failed,
    })
}
