use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ApiMethod {
    service: String,
    name: String,
}

impl ApiMethod {
    #[must_use]
    pub fn new(service: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            name: name.into(),
        }
    }

    #[must_use]
    pub fn service(&self) -> &str {
        &self.service
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}
