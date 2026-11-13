use drift_core::{Action, PlanView, Prepared};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub prepared: Prepared,
    pub view: PlanView,
    pub skipped: Vec<String>,
}

pub trait Resolver {
    fn propose(&mut self, action: Action, paths: &[String]) -> Result<Proposal, String>;
    fn apply(&mut self, proposal: &Proposal) -> Result<String, String>;
}
