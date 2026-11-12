use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidPath { path: String, reason: &'static str },
    InvalidRootId(String),
    DuplicateRoot(String),
    OverlappingRoots { first: String, second: String },
    Config(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidPath { path, reason } => write!(f, "invalid path {path:?}: {reason}"),
            Error::InvalidRootId(id) => write!(f, "invalid root id {id:?}"),
            Error::DuplicateRoot(id) => write!(f, "root {id:?} is defined twice"),
            Error::OverlappingRoots { first, second } => {
                write!(f, "roots {first:?} and {second:?} overlap")
            }
            Error::Config(message) => write!(f, "invalid configuration: {message}"),
        }
    }
}

impl std::error::Error for Error {}
