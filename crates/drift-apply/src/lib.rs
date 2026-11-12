mod apply;
mod edit;
mod seed;

pub use apply::{
    apply, apply_checked, history, inspect_undo, integrate, touches_hypr_config, undo, ApplyError,
    ApplyReport, CheckedApply, Divergence, FailAt, FileStep, History, Hyprctl, HyprlandCheck,
    HyprlandStatus, IntegrateMode, IntegrateOutcome, Payload, Request, Roots, Unapplied, UndoItem,
    UndoOutcome, UndoRequest, UndoStatus, WriteFollowup,
};
pub use edit::edit_text;
pub use seed::{scan, Scan, StoreRoots};

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
