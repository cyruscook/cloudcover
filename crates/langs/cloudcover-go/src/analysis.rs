use std::{ffi::CString, path::Path};

use cloudcover_core::GoMethodReference;

use crate::{
    GoAnalysisError,
    ffi::{AnalyzerResponseGuard, analyze_go},
    response::AnalyzerResponse,
};

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

    let response_ptr = analyze_go(path.as_ptr());
    if response_ptr.is_null() {
        return Err(GoAnalysisError::NullResponse);
    }

    let response = AnalyzerResponseGuard::from_raw(response_ptr);
    let response = response.as_c_str().to_str().map_err(|error| {
        GoAnalysisError::Analyzer(format!("analyzer returned non-utf-8 response: {error}"))
    })?;
    let response: AnalyzerResponse =
        serde_json::from_str(response).map_err(GoAnalysisError::InvalidResponse)?;

    response.into_methods().map_err(GoAnalysisError::Analyzer)
}
