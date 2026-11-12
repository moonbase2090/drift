use std::fmt;
use std::path::{Path, PathBuf};

use crate::error::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RelPath(String);

impl RelPath {
    pub fn new(path: &str) -> Result<Self, Error> {
        let invalid = |reason| Error::InvalidPath {
            path: path.to_string(),
            reason,
        };
        if path.is_empty() {
            return Err(invalid("empty"));
        }
        if path.starts_with('/') {
            return Err(invalid("absolute"));
        }
        if path.contains('\0') {
            return Err(invalid("contains a NUL byte"));
        }
        for segment in path.split('/') {
            match segment {
                "" => return Err(invalid("empty segment")),
                "." => return Err(invalid("contains '.'")),
                ".." => return Err(invalid("contains '..'")),
                _ => {}
            }
        }
        Ok(RelPath(path.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }

    pub fn starts_with(&self, prefix: &RelPath) -> bool {
        let mut mine = self.segments();
        prefix.segments().all(|p| mine.next() == Some(p))
    }

    pub fn strip_prefix(&self, prefix: &RelPath) -> Option<RelPath> {
        if !self.starts_with(prefix) {
            return None;
        }
        let rest = self.segments().skip(prefix.segments().count());
        let rest: Vec<&str> = rest.collect();
        if rest.is_empty() {
            None
        } else {
            Some(RelPath(rest.join("/")))
        }
    }

    pub fn join(&self, other: &RelPath) -> RelPath {
        RelPath(format!("{}/{}", self.0, other.0))
    }

    pub fn overlaps(&self, other: &RelPath) -> bool {
        self.starts_with(other) || other.starts_with(self)
    }

    pub fn under(&self, base: &Path) -> PathBuf {
        let mut path = base.to_path_buf();
        for segment in self.segments() {
            path.push(segment);
        }
        path
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(s: &str) -> RelPath {
        RelPath::new(s).unwrap()
    }

    #[test]
    fn accepts_plain_relative_paths() {
        assert_eq!(rel("hypr/hyprland.lua").as_str(), "hypr/hyprland.lua");
        assert_eq!(rel(".config").as_str(), ".config");
    }

    #[test]
    fn rejects_unsafe_or_malformed_paths() {
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../b",
            "a/..",
            "./a",
            "a/./b",
            "a//b",
            "a/",
            "a\0b",
        ] {
            assert!(RelPath::new(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn prefix_matching_is_per_segment() {
        assert!(rel("config/hypr/a.lua").starts_with(&rel("config")));
        assert!(rel("config").starts_with(&rel("config")));
        assert!(!rel("configx/a.lua").starts_with(&rel("config")));
        assert!(!rel("conf").starts_with(&rel("config")));
    }

    #[test]
    fn strip_prefix_returns_the_remainder_only_below_the_prefix() {
        assert_eq!(
            rel("config/hypr/a.lua").strip_prefix(&rel("config")),
            Some(rel("hypr/a.lua"))
        );
        assert_eq!(rel("config").strip_prefix(&rel("config")), None);
        assert_eq!(rel("configx/a").strip_prefix(&rel("config")), None);
    }

    #[test]
    fn overlap_is_symmetric_and_segment_based() {
        assert!(rel("a/b").overlaps(&rel("a")));
        assert!(rel("a").overlaps(&rel("a/b")));
        assert!(rel("a").overlaps(&rel("a")));
        assert!(!rel("a").overlaps(&rel("ab")));
    }

    #[test]
    fn under_joins_each_segment_onto_the_base() {
        assert_eq!(
            rel(".config/hypr/a.lua").under(Path::new("/home/u")),
            PathBuf::from("/home/u/.config/hypr/a.lua")
        );
    }

    #[test]
    fn join_concatenates_with_a_separator() {
        assert_eq!(
            rel(".config").join(&rel("hypr/a.lua")),
            rel(".config/hypr/a.lua")
        );
    }
}
