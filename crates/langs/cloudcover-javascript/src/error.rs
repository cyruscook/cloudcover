use std::{io, path::PathBuf, process::ExitStatus};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum JavaScriptAnalysisError {
    #[error("path is not a directory: {}", path.display())]
    NotDirectory { path: PathBuf },
    #[error("path is not valid UTF-8: {}", path.display())]
    NonUtf8Path { path: PathBuf },
    #[error("Node.js is required to analyze JavaScript or TypeScript: {0}")]
    NodeUnavailable(#[source] io::Error),
    #[error("Node.js analyzer exited with {status}: {stderr}")]
    NodeFailed { status: ExitStatus, stderr: String },
    #[error("failed to parse JavaScript analyzer response: {0}")]
    InvalidResponse(#[source] serde_json::Error),
    #[error("{0}")]
    Analyzer(String),
}
