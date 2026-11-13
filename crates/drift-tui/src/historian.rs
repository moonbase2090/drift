use drift_core::{HistoryView, PlanView, UndoPreview};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoProposal {
    pub preview: UndoPreview,
    pub view: PlanView,
    pub forced: Vec<String>,
}

pub trait Historian {
    fn rows(&mut self) -> Result<HistoryView, String>;
    fn propose(&mut self, target: &str, force: bool) -> Result<UndoProposal, String>;
    fn apply(&mut self, proposal: &UndoProposal) -> Result<String, String>;
}
