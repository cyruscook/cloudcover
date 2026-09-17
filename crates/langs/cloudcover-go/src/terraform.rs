use std::{ffi::CString, path::Path};

use cloudcover_core::TerraformMethodReference;
use serde::Deserialize;

use crate::{
    GoAnalysisError,
    ffi::{TerraformAnalyzerResponseGuard, analyze_terraform},
};
/// Analyze a Terraform root module and its initialized module graph.
///
/// # Errors
///
/// Returns [`GoAnalysisError`] when Terraform initialization metadata is
/// missing or invalid, or analysis fails.
pub fn analyze_terraform_dir(
    path: impl AsRef<Path>,
) -> Result<(Vec<TerraformMethodReference>, String), GoAnalysisError> {
    let path = path.as_ref();
    if !path.is_dir() {
        return Err(GoAnalysisError::NotDirectory(path.to_path_buf()));
    }
    let path =
        CString::new(path.to_string_lossy().as_bytes()).map_err(GoAnalysisError::InteriorNul)?;
    let response_ptr = analyze_terraform(path.as_ptr());
    if response_ptr.is_null() {
        return Err(GoAnalysisError::NullResponse);
    }
    let response = TerraformAnalyzerResponseGuard::from_raw(response_ptr);
    let response = response.as_c_str().to_str().map_err(|error| {
        GoAnalysisError::Analyzer(format!("analyzer returned non-utf-8 response: {error}"))
    })?;
    let response: AnalyzerResponse =
        serde_json::from_str(response).map_err(GoAnalysisError::InvalidResponse)?;
    response.into_analysis().map_err(GoAnalysisError::Analyzer)
}

#[derive(Deserialize)]
struct AnalyzerResponse {
    methods: Option<Vec<AnalyzerMethod>>,
    provider_version: Option<String>,
    error: Option<String>,
}

impl AnalyzerResponse {
    fn into_analysis(self) -> Result<(Vec<TerraformMethodReference>, String), String> {
        if let Some(error) = self.error.filter(|message| !message.is_empty()) {
            return Err(error);
        }
        let version = self
            .provider_version
            .filter(|version| !version.is_empty())
            .ok_or_else(|| "analyzer response omitted provider version".to_owned())?;
        let methods = self
            .methods
            .unwrap_or_default()
            .into_iter()
            .map(|method| {
                TerraformMethodReference::new(method.kind, method.type_name, method.action)
            })
            .collect();
        Ok((methods, version))
    }
}

#[derive(Deserialize)]
struct AnalyzerMethod {
    kind: String,
    type_name: String,
    action: String,
}
