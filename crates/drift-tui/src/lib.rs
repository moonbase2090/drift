mod audit;
mod commands;
#[cfg(test)]
mod contrast;
mod diff;
mod diff_palette;
mod differ;
mod historian;
mod history_screen;
mod overlay;
mod plan_dialog;
mod resolver;
mod screen;
mod suspend;
mod tree_screen;
mod wake;
mod worker;

pub use audit::input_map;
pub use commands::{
    chord_text, chords_text, help_lines, help_sections, keymap_of, Context, GlobalCommand,
    HistoryCommand, TreeCommand, GLOBAL_COMMANDS, HISTORY_COMMANDS, TREE_COMMANDS,
};
pub use diff::DiffScreen;
pub use differ::Differ;
pub use historian::{Historian, UndoProposal};
pub use history_screen::{HistoryAction, HistoryScreen};
pub use overlay::{Help, Outcome, Palette};
pub use plan_dialog::{apply_decision, review_plan, Decision, Focus, PlanDialog};
pub use resolver::{EditedFile, Proposal, Resolver};
pub use screen::{
    frame_text, keymap, launch, launch_failed, launch_with, launch_worker, launch_worker_with,
    quit_bounds, render_frame, Screen,
};
pub use tree_screen::{glyph, Action, Hit, TreeScreen};
pub use worker::{spawn_scan, ChangeSink, WorkerHandle};

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
