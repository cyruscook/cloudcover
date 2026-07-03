use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum MethodReference {
    Python(PythonMethodReference),
    Go(GoMethodReference),
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct PythonMethodReference {
    module: String,
    receiver: Option<String>,
    name: String,
}

impl PythonMethodReference {
    #[must_use]
    pub fn new(
        module: impl Into<String>,
        receiver: Option<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            module: module.into(),
            receiver,
            name: name.into(),
        }
    }

    #[must_use]
    pub fn module(&self) -> &str {
        &self.module
    }

    #[must_use]
    pub fn receiver(&self) -> Option<&str> {
        self.receiver.as_deref()
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct GoMethodReference {
    package: String,
    receiver: Option<String>,
    name: String,
}

impl GoMethodReference {
    #[must_use]
    pub fn new(
        package: impl Into<String>,
        receiver: Option<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            package: package.into(),
            receiver,
            name: name.into(),
        }
    }

    #[must_use]
    pub fn package(&self) -> &str {
        &self.package
    }

    #[must_use]
    pub fn receiver(&self) -> Option<&str> {
        self.receiver.as_deref()
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}
