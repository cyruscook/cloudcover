use std::{path::Path, process::Command};

use cloudcover_core::{JavaScriptMethodReference, SdkModule};

use crate::{JavaScriptAnalysisError, response::AnalyzerResponse};

const ANALYZER: &str = include_str!("../analyzer/analyzer.js");

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JavaScriptAnalysis {
    methods: Vec<JavaScriptMethodReference>,
    modules: Vec<SdkModule>,
}

impl JavaScriptAnalysis {
    #[must_use]
    pub fn methods(&self) -> &[JavaScriptMethodReference] {
        &self.methods
    }

    #[must_use]
    pub fn modules(&self) -> &[SdkModule] {
        &self.modules
    }
}

/// Analyzes JavaScript or TypeScript source without executing the project or its dependencies.
///
/// The analyzed directory must have a locally resolvable `typescript` package. The embedded
/// analyzer loads that compiler through Node's module resolver anchored at the directory.
///
/// # Errors
///
/// Returns [`JavaScriptAnalysisError`] when the directory is invalid, Node or TypeScript cannot
/// be used, or the analyzer reports an actionable source-resolution failure.
pub fn analyze_dir(path: impl AsRef<Path>) -> Result<JavaScriptAnalysis, JavaScriptAnalysisError> {
    let path = path.as_ref();
    if !path.is_dir() {
        return Err(JavaScriptAnalysisError::NotDirectory {
            path: path.to_path_buf(),
        });
    }

    let path = path
        .to_str()
        .ok_or_else(|| JavaScriptAnalysisError::NonUtf8Path {
            path: path.to_path_buf(),
        })?;
    let output = Command::new("node")
        .arg("-e")
        .arg(ANALYZER)
        .arg(path)
        .output()
        .map_err(JavaScriptAnalysisError::NodeUnavailable)?;

    if !output.status.success() {
        return Err(JavaScriptAnalysisError::NodeFailed {
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    let response: AnalyzerResponse =
        serde_json::from_slice(&output.stdout).map_err(JavaScriptAnalysisError::InvalidResponse)?;
    let (methods, modules) = response
        .into_analysis()
        .map_err(JavaScriptAnalysisError::Analyzer)?;
    Ok(JavaScriptAnalysis { methods, modules })
}
