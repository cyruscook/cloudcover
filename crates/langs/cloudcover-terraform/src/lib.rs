use cloudcover_core::TerraformMethodReference;

pub type TerraformAnalysisError = cloudcover_go::GoAnalysisError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerraformAnalysis {
    methods: Vec<TerraformMethodReference>,
    provider_version: String,
}

impl TerraformAnalysis {
    #[must_use]
    pub fn methods(&self) -> &[TerraformMethodReference] {
        &self.methods
    }

    #[must_use]
    pub fn provider_version(&self) -> &str {
        &self.provider_version
    }
}

/// Analyze a Terraform root module and its initialized module graph.
///
/// # Errors
///
/// Returns [`TerraformAnalysisError`] when the path is invalid, Terraform's
/// initialization metadata is missing or invalid, or analysis fails.
pub fn analyze_dir(
    path: impl AsRef<std::path::Path>,
) -> Result<TerraformAnalysis, TerraformAnalysisError> {
    let (methods, provider_version) = cloudcover_go::analyze_terraform(path)?;
    Ok(TerraformAnalysis {
        methods,
        provider_version,
    })
}
