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
    let api_methods = build_api_methods(path, language)?;
    AwsProvider::new()
        .permissions_policy(&api_methods)
        .map_err(|error| CliError::Runtime(error.to_string()))
}

pub(crate) fn build_terraform_policy(
    path: OsString,
    language: Language,
) -> Result<String, CliError> {
    let api_methods = build_api_methods(path, language)?;
    AwsProvider::new()
        .permissions_policy_hcl(&api_methods)
        .map_err(|error| CliError::Runtime(error.to_string()))
}

fn build_api_methods(path: OsString, language: Language) -> Result<Vec<ApiMethod>, CliError> {
    match language {
        Language::Go => build_go_api_methods(path),
        Language::Terraform => build_terraform_api_methods(&path),
        Language::JavaScript | Language::TypeScript => {
            build_javascript_api_methods(&path, language)
        }
        Language::Python => Err(CliError::Usage(format!(
            "unsupported language: {language:?}"
        ))),
    }
}

fn build_go_api_methods(path: OsString) -> Result<Vec<ApiMethod>, CliError> {
    let provider = AwsProvider::new();
    let analysis = cloudcover_go::analyze_dir(path)
        .map_err(|error| CliError::Runtime(format!("failed to analyze Go code: {error}")))?;
    let mut methods_by_sdk = BTreeMap::<(String, Option<String>, String), Vec<ApiMethod>>::new();
    for sdk_name in ["aws-sdk-go-v2", "aws-sdk-go-v1"] {
        let resolved_sdk = ResolvedSdk::new(Sdk::new(sdk_name, Language::Go))
            .with_modules(analysis.modules().iter().cloned());
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
    }

    let mut api_methods = BTreeSet::new();
    for go_method in analysis.methods() {
        if let Some(mapped_methods) = methods_by_sdk.get(&go_method_key(go_method)) {
            api_methods.extend(mapped_methods.iter().cloned());
        }
    }
    Ok(api_methods.into_iter().collect())
}

fn build_terraform_api_methods(path: &OsString) -> Result<Vec<ApiMethod>, CliError> {
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
    Ok(api_methods.into_iter().collect())
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

fn build_javascript_api_methods(
    path: &OsString,
    language: Language,
) -> Result<Vec<ApiMethod>, CliError> {
    let analysis = cloudcover_javascript::analyze_dir(path).map_err(|error| {
        CliError::Runtime(format!(
            "failed to analyze JavaScript/TypeScript code: {error}"
        ))
    })?;
    let resolved_sdk = ResolvedSdk::new(Sdk::new("aws-sdk-js-v3", language))
        .with_modules(analysis.modules().iter().cloned());
    let mappings = AwsProvider::new()
        .sdk_method_mappings(&resolved_sdk)
        .map_err(|error| CliError::Runtime(error.to_string()))?;
    let mut methods_by_reference = BTreeMap::new();
    for mapping in &mappings {
        if let MethodReference::JavaScript(method) = mapping.method() {
            methods_by_reference.insert(method, mapping.api_methods());
        }
    }
    let mut api_methods = BTreeSet::new();
    for method in analysis.methods() {
        let mapped_methods = methods_by_reference.get(method).ok_or_else(|| {
            CliError::Runtime(format!(
                "AWS SDK for JavaScript v3 reference is not mapped: {} {}{}",
                method.package(),
                method
                    .receiver()
                    .map_or(String::new(), |receiver| format!("{receiver}.")),
                method.name(),
            ))
        })?;
        api_methods.extend(mapped_methods.iter().cloned());
    }
    Ok(api_methods.into_iter().collect())
}
