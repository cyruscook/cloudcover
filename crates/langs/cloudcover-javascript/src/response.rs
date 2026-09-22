use cloudcover_core::{JavaScriptMethodReference, SdkModule};
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct AnalyzerResponse {
    methods: Option<Vec<AnalyzerMethod>>,
    modules: Option<Vec<AnalyzerModule>>,
    error: Option<String>,
}

impl AnalyzerResponse {
    pub(crate) fn into_analysis(
        self,
    ) -> Result<(Vec<JavaScriptMethodReference>, Vec<SdkModule>), String> {
        if let Some(error) = self.error.filter(|message| !message.is_empty()) {
            return Err(error);
        }

        let methods = self
            .methods
            .unwrap_or_default()
            .into_iter()
            .map(|method| {
                JavaScriptMethodReference::new(method.package, method.receiver, method.name)
            })
            .collect();
        let modules = self
            .modules
            .unwrap_or_default()
            .into_iter()
            .map(|module| SdkModule::new(module.path, module.version))
            .collect();
        Ok((methods, modules))
    }
}

#[derive(Deserialize)]
struct AnalyzerMethod {
    package: String,
    receiver: Option<String>,
    name: String,
}

#[derive(Deserialize)]
struct AnalyzerModule {
    path: String,
    version: String,
}
