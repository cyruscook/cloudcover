use serde::{Deserialize, Serialize};

use crate::{ApiMethod, MethodReference, Sdk};

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct SdkMethodMapping {
    sdk: Sdk,
    method: MethodReference,
    api_methods: Vec<ApiMethod>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    unresolved_api_methods: Vec<ApiMethod>,
}

impl SdkMethodMapping {
    #[must_use]
    pub fn new(sdk: Sdk, method: MethodReference, api_methods: Vec<ApiMethod>) -> Self {
        Self {
            sdk,
            method,
            api_methods,
            unresolved_api_methods: Vec::new(),
        }
    }

    #[must_use]
    pub fn sdk(&self) -> &Sdk {
        &self.sdk
    }

    #[must_use]
    pub fn method(&self) -> &MethodReference {
        &self.method
    }

    #[must_use]
    pub fn api_methods(&self) -> &[ApiMethod] {
        &self.api_methods
    }
}

impl SdkMethodMapping {
    /// Retain source operations that the selected cloud catalog cannot resolve.
    #[must_use]
    pub fn with_unresolved_api_methods(mut self, mut methods: Vec<ApiMethod>) -> Self {
        methods.sort();
        methods.dedup();
        self.unresolved_api_methods = methods;
        self
    }

    #[must_use]
    pub fn unresolved_api_methods(&self) -> &[ApiMethod] {
        &self.unresolved_api_methods
    }

    /// An empty resolved list is known to make no calls only when no source
    /// operations remain unresolved. Unsupported SDK releases return errors.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.unresolved_api_methods.is_empty()
    }
}
