use std::{ffi::NulError, fmt, path::PathBuf};

#[derive(Debug)]
pub enum GoAnalysisError {
    NotDirectory(PathBuf),
    NonUtf8Path(PathBuf),
    InteriorNul(NulError),
    NullResponse,
    InvalidResponse(serde_json::Error),
    Analyzer(String),
}

impl fmt::Display for GoAnalysisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotDirectory(path) => {
                write!(formatter, "path is not a directory: {}", path.display())
            }
            Self::NonUtf8Path(path) => {
                write!(formatter, "path is not valid utf-8: {}", path.display())
            }
            Self::InteriorNul(error) => {
                write!(formatter, "path contains interior NUL byte: {error}")
            }
            Self::NullResponse => formatter.write_str("analyzer returned null response"),
            Self::InvalidResponse(error) => {
                write!(formatter, "failed to parse analyzer response: {error}")
            }
            Self::Analyzer(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for GoAnalysisError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InteriorNul(error) => Some(error),
            Self::InvalidResponse(error) => Some(error),
            Self::NotDirectory(_)
            | Self::NonUtf8Path(_)
            | Self::NullResponse
            | Self::Analyzer(_) => None,
        }
    }
}
