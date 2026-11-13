mod apply;
mod edit;
mod notify;
mod seed;
mod snapshot;
mod widget;

pub use apply::{
    apply, apply_checked, execute, history, inspect_undo, integrate, pending_prune, recorded_bases,
    restore_reviewed_bases, touches_hypr_config, undo, undo_with_bases, undo_with_bases_expecting,
    undo_with_bases_probed, ApplyError, ApplyReport, CheckedApply, Divergence, Executed, FailAt,
    FileStep, History, Hyprctl, HyprlandCheck, HyprlandStatus, IntegrateMode, IntegrateOutcome,
    Payload, Request, Restored, Roots, Unapplied, UndoItem, UndoOutcome, UndoRequest, UndoStatus,
    UndoneWithBases, WriteFollowup,
};
pub use edit::edit_text;
pub use notify::{
    notice_for, notifications_enabled, send, set_notifications, should_send, update_running,
    Notice, NotifyError, APP_NAME, CLICK, URGENCY,
};
pub use seed::{record_bases, scan, Recorded, Scan, StoreRoots};
pub use snapshot::{snapshot_templates, SnapshotError, SnapshotReport};
pub use widget::{
    widget_install, widget_remove, widget_sync, OmarchyShell, SystemOmarchy, WidgetMode,
    WidgetOptions, WidgetOutcome, WidgetPaths, RECORD_FILE,
};

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
