use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum MethodReference {
    Python(PythonMethodReference),
    Go(GoMethodReference),
    Terraform(TerraformMethodReference),
    JavaScript(JavaScriptMethodReference),
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

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct TerraformMethodReference {
    kind: String,
    type_name: String,
    action: String,
}

impl TerraformMethodReference {
    #[must_use]
    pub fn new(
        kind: impl Into<String>,
        type_name: impl Into<String>,
        action: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            type_name: type_name.into(),
            action: action.into(),
        }
    }

    #[must_use]
    pub fn kind(&self) -> &str {
        &self.kind
    }

    #[must_use]
    pub fn type_name(&self) -> &str {
        &self.type_name
    }

    #[must_use]
    pub fn action(&self) -> &str {
        &self.action
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct JavaScriptMethodReference {
    package: String,
    receiver: Option<String>,
    name: String,
}

impl JavaScriptMethodReference {
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
