use serde::{Deserialize, Serialize};

use crate::Language;

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Sdk {
    name: String,
    language: Language,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    version: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct SdkModuleReplacement {
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    version: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct SdkModule {
    path: String,
    version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    replacement: Option<SdkModuleReplacement>,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ResolvedSdk {
    sdk: Sdk,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    modules: Vec<SdkModule>,
}

impl Sdk {
    #[must_use]
    pub fn new(name: impl Into<String>, language: Language) -> Self {
        Self {
            name: name.into(),
            language,
            version: None,
        }
    }

    #[must_use]
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn language(&self) -> Language {
        self.language
    }

    #[must_use]
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }
}

impl SdkModuleReplacement {
    #[must_use]
    pub fn new(path: impl Into<String>, version: Option<String>) -> Self {
        Self {
            path: path.into(),
            version,
        }
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }
}

impl SdkModule {
    #[must_use]
    pub fn new(path: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            version: version.into(),
            replacement: None,
        }
    }

    #[must_use]
    pub fn with_replacement(mut self, replacement: SdkModuleReplacement) -> Self {
        self.replacement = Some(replacement);
        self
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    #[must_use]
    pub const fn replacement(&self) -> Option<&SdkModuleReplacement> {
        self.replacement.as_ref()
    }
}

impl ResolvedSdk {
    #[must_use]
    pub fn new(sdk: Sdk) -> Self {
        Self {
            sdk,
            modules: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_modules(mut self, modules: impl IntoIterator<Item = SdkModule>) -> Self {
        self.modules = modules.into_iter().collect();
        self.modules.sort();
        self.modules.dedup();
        self
    }

    #[must_use]
    pub const fn sdk(&self) -> &Sdk {
        &self.sdk
    }

    #[must_use]
    pub fn modules(&self) -> &[SdkModule] {
        &self.modules
    }
}
