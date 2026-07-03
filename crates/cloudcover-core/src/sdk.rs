use serde::{Deserialize, Serialize};

use crate::Language;

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Sdk {
    name: String,
    language: Language,
}

impl Sdk {
    #[must_use]
    pub fn new(name: impl Into<String>, language: Language) -> Self {
        Self {
            name: name.into(),
            language,
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn language(&self) -> Language {
        self.language
    }
}
