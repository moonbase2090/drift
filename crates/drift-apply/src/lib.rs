mod apply;
mod resolve;
mod seed;

pub use apply::{
    apply, apply_checked, history, inspect_undo, integrate, pending_prune, touches_hypr_config,
    undo, ApplyError, ApplyReport, CheckedApply, Divergence, FailAt, FileStep, History, Hyprctl,
    HyprlandCheck, HyprlandStatus, IntegrateMode, IntegrateOutcome, Payload, Request, Roots,
    Unapplied, UndoItem, UndoOutcome, UndoRequest, UndoStatus, WriteFollowup,
};
pub use resolve::{execute, Executed};
pub use seed::{record_bases, scan, Recorded, Scan, StoreRoots};

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_its_name() {
        assert!(crate_name().starts_with("drift-"));
    }
}
