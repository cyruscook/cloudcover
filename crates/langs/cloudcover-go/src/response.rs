use cloudcover_core::GoMethodReference;
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct AnalyzerResponse {
    methods: Option<Vec<AnalyzerMethod>>,
    error: Option<String>,
}

impl AnalyzerResponse {
    pub(crate) fn into_methods(self) -> Result<Vec<GoMethodReference>, String> {
        if let Some(error) = self.error.filter(|message| !message.is_empty()) {
            return Err(error);
        }

        Ok(self
            .methods
            .unwrap_or_default()
            .into_iter()
            .map(|method| GoMethodReference::new(method.package, method.receiver, method.name))
            .collect())
    }
}

#[derive(Deserialize)]
struct AnalyzerMethod {
    package: String,
    receiver: Option<String>,
    name: String,
}
