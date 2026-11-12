use std::path::Path;

use drift_core::Prepared;

use crate::apply::{
    apply_checked, ApplyError, CheckedApply, HyprlandCheck, Payload, Request, Roots,
};
use crate::seed::record_bases;

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
    let checked = apply_checked(
        roots,
        &Request {
            plan,
            files: &payloads,
            fail_at: None,
        },
        checker,
    )?;
    let recorded = record_bases(share, &prepared.bases);
    Ok(Executed {
        checked,
        recorded: recorded.recorded,
        unrecorded: recorded.failed,
    })
}
