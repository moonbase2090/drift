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

#[cfg(test)]
mod tests {
    use std::error::Error as StdError;

    use super::*;

    #[test]
    fn messages_name_the_failure() {
        let missing = Error::io(
            "var/lib/pacman/local",
            std::io::Error::new(std::io::ErrorKind::NotFound, "gone"),
        );
        let text = missing.to_string();
        assert!(text.contains("var/lib/pacman/local"), "{text}");
        assert!(text.contains("gone"), "{text}");
        assert!(StdError::source(&missing).is_some());
        let cases = [
            (Error::NotInstalled, "omarchy-settings is not installed"),
            (
                Error::Ambiguous(2),
                "found 2 omarchy-settings database entries",
            ),
            (Error::MissingDesc, "package entry has no desc"),
            (Error::MissingFileList, "package entry has no file list"),
            (Error::MissingVersion, "package entry has no version"),
            (Error::BadMtree, "package mtree could not be read"),
        ];
        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
            assert!(StdError::source(&error).is_none());
        }
    }
}
