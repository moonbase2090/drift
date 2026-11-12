use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub line: usize,
    pub column: usize,
}

impl Location {
    pub fn at(text: &str, offset: usize) -> Location {
        let end = offset.min(text.len());
        let before = text.get(..end).unwrap_or("");
        let line = before.matches('\n').count() + 1;
        let column = before
            .rsplit('\n')
            .next()
            .map_or(0, |last| last.chars().count())
            + 1;
        Location { line, column }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidPath {
        path: String,
        reason: &'static str,
    },
    InvalidRootId(String),
    DuplicateRoot(String),
    OverlappingRoots {
        first: String,
        second: String,
    },
    Config {
        message: String,
        location: Option<Location>,
    },
    UnresolvedHunk(usize),
    ChoiceCount {
        expected: usize,
        got: usize,
    },
    BadPlan {
        message: String,
    },
    BadManifest {
        message: String,
    },
    Integration {
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnresolvedHunk(index) => write!(f, "conflict hunk {index} has no choice"),
            Error::ChoiceCount { expected, got } => {
                write!(f, "expected {expected} hunk choices, got {got}")
            }
            Error::InvalidPath { path, reason } => write!(f, "invalid path {path:?}: {reason}"),
            Error::InvalidRootId(id) => write!(f, "invalid root id {id:?}"),
            Error::DuplicateRoot(id) => write!(f, "root {id:?} is defined twice"),
            Error::OverlappingRoots { first, second } => {
                write!(f, "roots {first:?} and {second:?} overlap")
            }
            Error::Config {
                message,
                location: Some(at),
            } => write!(
                f,
                "invalid configuration at line {}, column {}: {message}",
                at.line, at.column
            ),
            Error::Config {
                message,
                location: None,
            } => write!(f, "invalid configuration: {message}"),
            Error::BadPlan { message } => write!(f, "invalid plan: {message}"),
            Error::BadManifest { message } => write!(f, "invalid manifest: {message}"),
            Error::Integration { message } => write!(f, "cannot integrate: {message}"),
        }
    }
}

impl std::error::Error for Error {}
