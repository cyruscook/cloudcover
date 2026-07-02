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

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Language {
    Python,
    Go,
}

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

pub trait CloudProvider {
    type Error: std::error::Error + Send + Sync + 'static;

    #[must_use]
    fn list_api_methods(&self) -> Vec<ApiMethod>;

    #[must_use]
    fn list_sdks(&self) -> Vec<Sdk>;

    #[must_use]
    fn sdk_method_mappings(&self) -> Vec<SdkMethodMapping>;

    /// # Errors
    ///
    /// Returns an error when the provider cannot map every requested API method
    /// to a permissions policy entry.
    fn permissions_policy(&self, methods: &[ApiMethod]) -> Result<serde_json::Value, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::{
        ApiMethod, GoMethodReference, Language, MethodReference, PythonMethodReference, Sdk,
        SdkMethodMapping,
    };

    #[test]
    fn api_method_constructor_and_accessors() {
        let method = ApiMethod::new("s3", "GetObject");

        assert_eq!(method.service(), "s3");
        assert_eq!(method.name(), "GetObject");
    }

    #[test]
    fn api_methods_sort_by_service_then_name() {
        let mut methods = vec![
            ApiMethod::new("s3", "PutObject"),
            ApiMethod::new("ec2", "DescribeInstances"),
            ApiMethod::new("s3", "GetObject"),
        ];

        methods.sort();

        assert_eq!(
            methods,
            vec![
                ApiMethod::new("ec2", "DescribeInstances"),
                ApiMethod::new("s3", "GetObject"),
                ApiMethod::new("s3", "PutObject"),
            ]
        );
    }

    #[test]
    fn sdk_constructor_and_accessors() {
        let sdk = Sdk::new("boto3", Language::Python);

        assert_eq!(sdk.name(), "boto3");
        assert_eq!(sdk.language(), Language::Python);
    }

    #[test]
    fn python_method_reference_constructor_and_accessors() {
        let method = PythonMethodReference::new("boto3", Some("s3".to_owned()), "get_object");

        assert_eq!(method.module(), "boto3");
        assert_eq!(method.receiver(), Some("s3"));
        assert_eq!(method.name(), "get_object");
    }

    #[test]
    fn go_method_reference_constructor_and_accessors() {
        let method =
            GoMethodReference::new("example.com/myapp", Some("MyType".to_owned()), "MyMethod");

        assert_eq!(method.package(), "example.com/myapp");
        assert_eq!(method.receiver(), Some("MyType"));
        assert_eq!(method.name(), "MyMethod");
    }

    #[test]
    fn sdk_method_mapping_constructor_and_accessors() {
        let mapping = SdkMethodMapping::new(
            Sdk::new("boto3", Language::Python),
            MethodReference::Python(PythonMethodReference::new(
                "boto3",
                Some("s3".to_owned()),
                "get_object",
            )),
            vec![ApiMethod::new("s3", "GetObject")],
        );

        assert_eq!(mapping.sdk(), &Sdk::new("boto3", Language::Python));
        assert_eq!(
            mapping.method(),
            &MethodReference::Python(PythonMethodReference::new(
                "boto3",
                Some("s3".to_owned()),
                "get_object",
            ))
        );
        assert_eq!(mapping.api_methods(), &[ApiMethod::new("s3", "GetObject")]);
    }
}
