use std::path::Path;

use drift_core::{BaseRecord, Plan, Prepared, RelPath};
use drift_source::{base_path, read_base};

use super::*;
use crate::seed::{record_bases, Recorded};

const RECORD: &str = "reviewed-bases.json";

type Previous = Vec<(String, RelPath, Option<Vec<u8>>)>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Executed {
    pub checked: CheckedApply,
    pub recorded: Vec<String>,
    pub unrecorded: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    pub restored: Vec<String>,
    pub failed: Vec<String>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).ok())
        .collect()
}

fn record_path(state: &Path, txn: &str) -> PathBuf {
    state.join("backups").join(txn).join(RECORD)
}

fn save_previous(state: &Path, txn: &str, entries: &Previous) -> Result<(), ApplyError> {
    let list: Vec<serde_json::Value> = entries
        .iter()
        .map(|(root, rel, previous)| {
            serde_json::json!({
                "root": root,
                "rel": rel.as_str(),
                "previous": previous.as_deref().map(hex),
            })
        })
        .collect();
    let bytes = serde_json::to_vec(&serde_json::json!({ "bases": list }))
        .map_err(|error| bare(error.to_string()))?;
    write_bytes(state, &record_path(state, txn), &bytes, 0o644)
}

fn load_previous(state: &Path, txn: &str) -> Result<Option<Previous>, ApplyError> {
    let path = record_path(state, txn);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(bare_io(&path, source)),
    };
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| bare(error.to_string()))?;
    let mut out = Vec::new();
    for entry in value
        .get("bases")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| bare("reviewed base record is malformed"))?
    {
        let root = entry.get("root").and_then(serde_json::Value::as_str);
        let rel = entry.get("rel").and_then(serde_json::Value::as_str);
        let (Some(root), Some(rel)) = (root, rel) else {
            return Err(bare("reviewed base record is malformed"));
        };
        let rel = RelPath::new(rel).map_err(|error| bare(error.to_string()))?;
        let previous = match entry.get("previous") {
            Some(serde_json::Value::String(text)) => {
                Some(unhex(text).ok_or_else(|| bare("reviewed base record is malformed"))?)
            }
            _ => None,
        };
        out.push((root.to_string(), rel, previous));
    }
    Ok(Some(out))
}

pub fn execute(
    roots: &Roots,
    share: &Path,
    prepared: &Prepared,
    checker: &dyn HyprlandCheck,
) -> Result<Executed, ApplyError> {
    let Some(plan) = prepared.plan.as_ref() else {
        return Err(bare("there is nothing to apply"));
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
        &mut |_| {
            let previous: Previous = prepared
                .bases
                .iter()
                .map(|record| {
                    (
                        record.root.clone(),
                        record.rel.clone(),
                        read_base(share, &record.root, &record.rel),
                    )
                })
                .collect();
            recorded = record_bases(share, &prepared.bases);
            if save_previous(&roots.state, plan.id(), &previous).is_err() {
                recorded
                    .failed
                    .push("the previous reviewed bases (undo will not restore them)".to_string());
            }
        },
    )?;
    let followup = hypr::follow_write(plan, &report.applied, checker);
    let checked = CheckedApply { report, followup };
    Ok(Executed {
        checked,
        recorded: recorded.recorded,
        unrecorded: recorded.failed,
    })
}

fn put_base(share: &Path, record: &BaseRecord) -> bool {
    crate::seed::record_bases(share, std::slice::from_ref(record))
        .failed
        .is_empty()
}

fn remove_base(share: &Path, root: &str, rel: &RelPath) -> bool {
    match base_path(share, root, rel) {
        Some(dest) => match fs::remove_file(&dest) {
            Ok(()) => true,
            Err(source) => source.kind() == std::io::ErrorKind::NotFound,
        },
        None => false,
    }
}

pub fn recorded_bases(state: &Path, target: &str) -> Result<Vec<String>, ApplyError> {
    Ok(load_previous(state, target)?
        .unwrap_or_default()
        .into_iter()
        .map(|(root, rel, _)| format!("{root}/{}", rel.as_str()))
        .collect())
}

pub fn restore_reviewed_bases(
    share: &Path,
    state: &Path,
    target: &str,
    undo_id: &str,
    time: &str,
    package_version: &str,
) -> Result<Restored, ApplyError> {
    let entries = load_previous(state, target)?;
    restore_entries(entries, share, state, undo_id, time, package_version)
}

fn restore_entries(
    entries: Option<Previous>,
    share: &Path,
    state: &Path,
    undo_id: &str,
    time: &str,
    package_version: &str,
) -> Result<Restored, ApplyError> {
    let Some(entries) = entries else {
        return Ok(Restored {
            restored: Vec::new(),
            failed: Vec::new(),
        });
    };
    ensure_transaction(state, undo_id, time, package_version)?;
    let mut now = Vec::new();
    let mut restored = Vec::new();
    let mut failed = Vec::new();
    for (root, rel, previous) in &entries {
        let label = format!("{root}/{}", rel.as_str());
        now.push((root.clone(), rel.clone(), read_base(share, root, rel)));
        let done = match previous {
            Some(bytes) => put_base(
                share,
                &BaseRecord {
                    root: root.clone(),
                    rel: rel.clone(),
                    bytes: bytes.clone(),
                },
            ),
            None => remove_base(share, root, rel),
        };
        if done {
            restored.push(label);
        } else {
            failed.push(label);
        }
    }
    save_previous(state, undo_id, &now)?;
    Ok(Restored { restored, failed })
}

fn ensure_transaction(
    state: &Path,
    id: &str,
    time: &str,
    package_version: &str,
) -> Result<(), ApplyError> {
    let dir = state.join("backups").join(id);
    if node_exists(&dir) {
        return Ok(());
    }
    reserve_transaction(state, id)?;
    let plan = Plan::new(id, time, package_version, Vec::new(), &[])
        .map_err(|error| bare(error.to_string()))?;
    write_manifest(state, &plan)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoneWithBases {
    pub outcome: UndoOutcome,
    pub bases: Restored,
}

pub fn undo_with_bases(
    roots: &Roots,
    share: &Path,
    request: &UndoRequest<'_>,
) -> Result<UndoneWithBases, ApplyError> {
    undo_with_bases_probed(roots, share, request, &mut || {})
}

pub fn undo_with_bases_expecting(
    roots: &Roots,
    share: &Path,
    request: &UndoRequest<'_>,
    prune: &[String],
) -> Result<UndoneWithBases, ApplyError> {
    undo_checked(roots, share, request, Some(prune), &mut || {})
}

#[doc(hidden)]
pub fn undo_with_bases_probed(
    roots: &Roots,
    share: &Path,
    request: &UndoRequest<'_>,
    between: &mut dyn FnMut(),
) -> Result<UndoneWithBases, ApplyError> {
    undo_checked(roots, share, request, None, between)
}

fn undo_checked(
    roots: &Roots,
    share: &Path,
    request: &UndoRequest<'_>,
    expected_prune: Option<&[String]>,
    between: &mut dyn FnMut(),
) -> Result<UndoneWithBases, ApplyError> {
    let mut bases = Restored {
        restored: Vec::new(),
        failed: Vec::new(),
    };
    let outcome = undo_locked(
        roots,
        request,
        || {
            if let Some(expected) = expected_prune {
                let now = transactions_to_prune(&roots.state)?;
                if !same_ids(expected, &now) {
                    return Err(bare(
                        "the transactions to prune changed since the preview; nothing was written",
                    ));
                }
            }
            load_previous(&roots.state, request.target)
        },
        |previous, result| {
            let has_record = previous.is_some();
            let outcome = match result {
                Ok(UndoOutcome::Diverged(items)) => return Ok(UndoOutcome::Diverged(items)),
                Ok(restored) => restored,
                Err(error) if has_record && error.message == "nothing to undo" => {
                    UndoOutcome::Restored(ApplyReport {
                        applied: Vec::new(),
                        pruned: Vec::new(),
                    })
                }
                Err(error) => return Err(error),
            };
            between();
            bases = restore_entries(
                previous,
                share,
                &roots.state,
                request.id,
                request.time,
                request.package_version,
            )?;
            Ok(outcome)
        },
    )?;
    Ok(UndoneWithBases { outcome, bases })
}
