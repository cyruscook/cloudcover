use cloudcover_core::{GoMethodReference, SdkModule, SdkModuleReplacement};
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct AnalyzerResponse {
    methods: Option<Vec<AnalyzerMethod>>,
    modules: Option<Vec<AnalyzerModule>>,
    error: Option<String>,
}

impl AnalyzerResponse {
    pub(crate) fn into_analysis(self) -> Result<(Vec<GoMethodReference>, Vec<SdkModule>), String> {
        if let Some(error) = self.error.filter(|message| !message.is_empty()) {
            return Err(error);
        }

        let methods = self
            .methods
            .unwrap_or_default()
            .into_iter()
            .map(|method| GoMethodReference::new(method.package, method.receiver, method.name))
            .collect();
        let modules = self
            .modules
            .unwrap_or_default()
            .into_iter()
            .map(AnalyzerModule::into_sdk_module)
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
    replace: Option<AnalyzerModuleReplacement>,
}

impl AnalyzerModule {
    fn into_sdk_module(self) -> SdkModule {
        let module = SdkModule::new(self.path, self.version);
        match self.replace {
            Some(replacement) => module.with_replacement(SdkModuleReplacement::new(
                replacement.path,
                (!replacement.version.is_empty()).then_some(replacement.version),
            )),
            None => module,
        }
    }
}

#[derive(Deserialize)]
struct AnalyzerModuleReplacement {
    path: String,
    #[serde(default)]
    version: String,
}
