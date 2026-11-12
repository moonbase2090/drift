mod diff;
mod screen;

pub use diff::DiffScreen;
pub use screen::{frame_text, keymap, launch, quit_bounds, render_frame, Screen};

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
