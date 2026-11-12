mod screen;
mod tree_screen;

pub use screen::{
    frame_text, keymap, launch, launch_failed, launch_with, quit_bounds, render_frame, Screen,
};
pub use tree_screen::{glyph, Action, Hit, TreeScreen};

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
