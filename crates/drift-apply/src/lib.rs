mod apply;

pub use apply::{
    apply, history, inspect_undo, undo, ApplyError, ApplyReport, Divergence, FailAt, FileStep,
    History, Payload, Request, Roots, Unapplied, UndoItem, UndoOutcome, UndoRequest, UndoStatus,
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
