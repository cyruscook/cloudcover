use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
};

use cloudcover_aws::AwsProvider;
use cloudcover_core::{
    ApiMethod, CloudProvider, GoMethodReference, Language, MethodReference, ResolvedSdk, Sdk,
    TerraformMethodReference,
};

use crate::error::CliError;

pub(crate) fn build_policy(
    path: OsString,
    language: Language,
) -> Result<serde_json::Value, CliError> {
    match language {
        Language::Go => build_go_policy(path),
        Language::Terraform => build_terraform_policy(&path),
        Language::Python => Err(CliError::Usage(format!(
            "unsupported language: {language:?}"
        ))),
    }
}

fn build_go_policy(path: OsString) -> Result<serde_json::Value, CliError> {
    let provider = AwsProvider::new();
    let analysis = cloudcover_go::analyze_dir(path)
        .map_err(|error| CliError::Runtime(format!("failed to analyze Go code: {error}")))?;
    let resolved_sdk = ResolvedSdk::new(Sdk::new("aws-sdk-go-v2", Language::Go))
        .with_modules(analysis.modules().iter().cloned());
    let mut methods_by_sdk = BTreeMap::<(String, Option<String>, String), Vec<ApiMethod>>::new();
    for mapping in provider
        .sdk_method_mappings(&resolved_sdk)
        .map_err(|error| CliError::Runtime(error.to_string()))?
    {
        let MethodReference::Go(go_method) = mapping.method() else {
            continue;
        };
        methods_by_sdk
            .entry(go_method_key(go_method))
            .or_default()
            .extend(mapping.api_methods().iter().cloned());
    }

    let mut api_methods = BTreeSet::new();
    for go_method in analysis.methods() {
        if let Some(mapped_methods) = methods_by_sdk.get(&go_method_key(go_method)) {
            api_methods.extend(mapped_methods.iter().cloned());
        }
    }
    provider
        .permissions_policy(&api_methods.into_iter().collect::<Vec<_>>())
        .map_err(|error| CliError::Runtime(error.to_string()))
}

fn build_terraform_policy(path: &OsString) -> Result<serde_json::Value, CliError> {
    let provider = AwsProvider::new();
    let analysis = cloudcover_terraform::analyze_dir(path)
        .map_err(|error| CliError::Runtime(format!("failed to analyze Terraform code: {error}")))?;
    let sdk = Sdk::new("terraform-provider-aws", Language::Terraform)
        .with_version(analysis.provider_version());
    let resolved_sdk = ResolvedSdk::new(sdk);
    let mut methods_by_reference = BTreeMap::<(String, String, String), Vec<ApiMethod>>::new();
    for mapping in provider
        .sdk_method_mappings(&resolved_sdk)
        .map_err(|error| CliError::Runtime(error.to_string()))?
    {
        let MethodReference::Terraform(terraform_method) = mapping.method() else {
            continue;
        };
        methods_by_reference
            .entry(terraform_method_key(terraform_method))
            .or_default()
            .extend(mapping.api_methods().iter().cloned());
    }

    let mut api_methods = BTreeSet::new();
    for method in analysis.methods() {
        let key = terraform_method_key(method);
        let Some(mapped_methods) = methods_by_reference.get(&key) else {
            if methods_by_reference.keys().any(|(kind, type_name, _)| {
                kind == method.kind() && type_name == method.type_name()
            }) {
                continue;
            }
            return Err(CliError::Runtime(format!(
                "Terraform reference is not mapped by AWS provider data: {} {} {}",
                method.kind(),
                method.type_name(),
                method.action()
            )));
        };
        if mapped_methods.is_empty() {
            continue;
        }
        api_methods.extend(mapped_methods.iter().cloned());
    }
    provider
        .permissions_policy(&api_methods.into_iter().collect::<Vec<_>>())
        .map_err(|error| CliError::Runtime(error.to_string()))
}

fn go_method_key(method: &GoMethodReference) -> (String, Option<String>, String) {
    (
        method.package().to_owned(),
        method.receiver().map(str::to_owned),
        method.name().to_owned(),
    )
}

fn terraform_method_key(method: &TerraformMethodReference) -> (String, String, String) {
    (
        method.kind().to_owned(),
        method.type_name().to_owned(),
        method.action().to_owned(),
    )
}
