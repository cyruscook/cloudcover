use std::{
    ffi::{CStr, CString, NulError},
    fmt,
    os::raw::c_char,
    path::{Path, PathBuf},
};

use cloudcover_core::GoMethodReference;
use serde::Deserialize;

unsafe extern "C" {
    fn CloudCoverAnalyzeGo(path: *const c_char) -> *mut c_char;
    fn CloudCoverFreeCString(ptr: *mut c_char);
}

/// # Errors
///
/// Returns [`GoAnalysisError`] when the path is invalid for analysis, the FFI
/// analyzer fails, or the analyzer response cannot be parsed.
pub fn analyze_dir(path: impl AsRef<Path>) -> Result<Vec<GoMethodReference>, GoAnalysisError> {
    let path = path.as_ref();
    if !path.is_dir() {
        return Err(GoAnalysisError::NotDirectory(path.to_path_buf()));
    }

    let path_str = path
        .to_str()
        .ok_or_else(|| GoAnalysisError::NonUtf8Path(path.to_path_buf()))?;
    let path = CString::new(path_str).map_err(GoAnalysisError::InteriorNul)?;

    let response_ptr = unsafe { CloudCoverAnalyzeGo(path.as_ptr()) };
    if response_ptr.is_null() {
        return Err(GoAnalysisError::NullResponse);
    }

    let response = AnalyzerResponseGuard(response_ptr);
    let response = unsafe { CStr::from_ptr(response.0) }
        .to_str()
        .map_err(|error| {
            GoAnalysisError::Analyzer(format!("analyzer returned non-utf-8 response: {error}"))
        })?;
    let response: AnalyzerResponse =
        serde_json::from_str(response).map_err(GoAnalysisError::InvalidResponse)?;
    if let Some(error) = response.error.filter(|message| !message.is_empty()) {
        return Err(GoAnalysisError::Analyzer(error));
    }

    Ok(response
        .methods
        .unwrap_or_default()
        .into_iter()
        .map(|method| GoMethodReference::new(method.package, method.receiver, method.name))
        .collect())
}

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

struct AnalyzerResponseGuard(*mut c_char);

impl Drop for AnalyzerResponseGuard {
    fn drop(&mut self) {
        unsafe { CloudCoverFreeCString(self.0) };
    }
}

#[derive(Deserialize)]
struct AnalyzerResponse {
    methods: Option<Vec<AnalyzerMethod>>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct AnalyzerMethod {
    package: String,
    receiver: Option<String>,
    name: String,
}
