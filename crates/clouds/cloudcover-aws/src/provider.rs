use cloudcover_core::{
    ApiMethod, CloudProvider, GoMethodReference, Language, MethodReference, PythonMethodReference,
    Sdk, SdkMethodMapping, TerraformMethodReference,
};

use crate::{generated, policy};

#[derive(Clone, Copy, Debug, Default)]
pub struct AwsProvider;

impl AwsProvider {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AwsError {
    #[error("unknown AWS API method {service}:{name}")]
    UnknownApiMethod { service: String, name: String },
}

impl CloudProvider for AwsProvider {
    type Error = AwsError;

    fn list_api_methods(&self) -> Vec<ApiMethod> {
        generated::OPERATIONS
            .iter()
            .map(|operation| ApiMethod::new(operation.service, operation.name))
            .collect()
    }

    fn list_sdks(&self) -> Vec<Sdk> {
        vec![
            Sdk::new(cloudcover_aws_sdk_go_v2::SDK_NAME, Language::Go),
            Sdk::new("boto3", Language::Python),
            Sdk::new(
                cloudcover_terraform_provider_aws::SDK_NAME,
                Language::Terraform,
            ),
        ]
    }

    fn sdk_method_mappings(&self) -> Vec<SdkMethodMapping> {
        let mut mappings = python_sdk_method_mappings();
        mappings.extend(go_sdk_method_mappings());
        mappings.extend(terraform_provider_aws_sdk_method_mappings());
        mappings.sort();
        mappings.dedup();
        mappings
    }

    fn permissions_policy(&self, methods: &[ApiMethod]) -> Result<serde_json::Value, Self::Error> {
        policy::build_permissions_policy(methods)
    }
}

fn python_sdk_method_mappings() -> Vec<SdkMethodMapping> {
    generated::SDK_METHOD_MAPPINGS
        .iter()
        .map(|row| {
            SdkMethodMapping::new(
                Sdk::new(row.sdk_package, Language::Python),
                MethodReference::Python(PythonMethodReference::new(
                    "boto3",
                    Some(row.sdk_name.to_owned()),
                    row.sdk_method,
                )),
                vec![ApiMethod::new(row.api_service, row.api_name)],
            )
        })
        .collect()
}

fn go_sdk_method_mappings() -> impl Iterator<Item = SdkMethodMapping> {
    cloudcover_aws_sdk_go_v2::SDK_METHOD_MAPPINGS
        .iter()
        .filter_map(|row| {
            let api_methods = row
                .api_methods
                .iter()
                .filter(|api_method| operation_exists(api_method.service, api_method.name))
                .map(|api_method| ApiMethod::new(api_method.service, api_method.name))
                .collect::<Vec<_>>();
            if api_methods.is_empty() {
                return None;
            }

            Some(SdkMethodMapping::new(
                Sdk::new(cloudcover_aws_sdk_go_v2::SDK_NAME, Language::Go),
                MethodReference::Go(GoMethodReference::new(
                    row.package,
                    Some(row.receiver.to_owned()),
                    row.method,
                )),
                api_methods,
            ))
        })
}

fn terraform_provider_aws_sdk_method_mappings() -> Vec<SdkMethodMapping> {
    cloudcover_terraform_provider_aws::SDK_METHOD_MAPPINGS
        .iter()
        .map(|row| {
            let api_methods = row
                .api_methods
                .iter()
                .filter(|api_method| operation_exists(api_method.service, api_method.name))
                .map(|api_method| ApiMethod::new(api_method.service, api_method.name))
                .collect::<Vec<_>>();

            SdkMethodMapping::new(
                Sdk::new(
                    cloudcover_terraform_provider_aws::SDK_NAME,
                    Language::Terraform,
                ),
                MethodReference::Terraform(TerraformMethodReference::new(
                    row.kind,
                    row.type_name,
                    row.action,
                )),
                api_methods,
            )
        })
        .collect()
}

fn operation_exists(service: &str, name: &str) -> bool {
    generated::OPERATIONS
        .binary_search_by(|operation| (operation.service, operation.name).cmp(&(service, name)))
        .is_ok()
}
