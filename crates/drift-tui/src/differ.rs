use drift_core::FileDiff;

pub trait Differ {
    fn diff(&mut self, path: &str) -> Result<FileDiff, String>;
}
