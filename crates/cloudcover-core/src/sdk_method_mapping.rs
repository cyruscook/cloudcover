use serde::{Deserialize, Serialize};

use crate::{ApiMethod, MethodReference, Sdk};

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct SdkMethodMapping {
    sdk: Sdk,
    method: MethodReference,
    api_methods: Vec<ApiMethod>,
}

impl SdkMethodMapping {
    #[must_use]
    pub fn new(sdk: Sdk, method: MethodReference, api_methods: Vec<ApiMethod>) -> Self {
        Self {
            sdk,
            method,
            api_methods,
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
