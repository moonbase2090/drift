//! Drift compares local Omarchy and Hyprland configuration against
//! the templates shipped in the installed omarchy package.

/// Crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_semver() {
        let parts: Vec<_> = version().split('.').collect();
        assert_eq!(parts.len(), 3, "version should be major.minor.patch");
        assert!(
            parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())),
            "each version component should be numeric"
        );
    }
}
