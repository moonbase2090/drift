use drift_core::{Action, PlanView, Prepared};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub prepared: Prepared,
    pub view: PlanView,
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditedFile {
    pub path: String,
    pub before: Option<Vec<u8>>,
    pub template: Option<Vec<u8>>,
    pub result: Vec<u8>,
}

pub trait Resolver {
    fn propose(&mut self, action: Action, paths: &[String]) -> Result<Proposal, String>;
    fn propose_result(&mut self, file: &EditedFile) -> Result<Proposal, String>;
    fn apply(&mut self, proposal: &Proposal) -> Result<String, String>;
}
