use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum Error {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    NotInstalled,
    Ambiguous(usize),
    MissingDesc,
    MissingFileList,
    MissingVersion,
    BadMtree,
}

impl Error {
    pub(crate) fn io(path: impl AsRef<Path>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.as_ref().to_path_buf(),
            source,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::NotInstalled => write!(f, "omarchy-settings is not installed"),
            Self::Ambiguous(count) => {
                write!(f, "found {count} omarchy-settings database entries")
            }
            Self::MissingDesc => write!(f, "package entry has no desc"),
            Self::MissingFileList => write!(f, "package entry has no file list"),
            Self::MissingVersion => write!(f, "package entry has no version"),
            Self::BadMtree => write!(f, "package mtree could not be read"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
