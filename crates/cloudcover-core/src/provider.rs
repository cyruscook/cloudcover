use crate::{ApiMethod, Sdk, SdkMethodMapping};

pub trait CloudProvider {
    type Error: std::error::Error + Send + Sync + 'static;

    #[must_use]
    fn list_api_methods(&self) -> Vec<ApiMethod>;

    #[must_use]
    fn list_sdks(&self) -> Vec<Sdk>;

    /// # Errors
    ///
    /// Returns an error when the SDK is unsupported or when its required
    /// version is missing or unsupported.
    fn sdk_method_mappings(&self, sdk: &Sdk) -> Result<Vec<SdkMethodMapping>, Self::Error>;

    /// # Errors
    ///
    /// Returns an error when the provider cannot map every requested API method
    /// to a permissions policy entry.
    fn permissions_policy(&self, methods: &[ApiMethod]) -> Result<serde_json::Value, Self::Error>;
}
