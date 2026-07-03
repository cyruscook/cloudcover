use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
};

use cloudcover_aws::AwsProvider;
use cloudcover_core::{
    ApiMethod, CloudProvider, GoMethodReference, Language, MethodReference, Sdk,
};

use crate::error::CliError;

pub(crate) fn build_go_policy(
    path: OsString,
    language: Language,
) -> Result<serde_json::Value, CliError> {
    if language != Language::Go {
        return Err(CliError::Usage(format!(
            "unsupported language: {language:?}"
        )));
    }

    let provider = AwsProvider::new();
    let sdk = Sdk::new("aws-sdk-go-v2", Language::Go);
    let mut methods_by_sdk = BTreeMap::<(String, Option<String>, String), Vec<ApiMethod>>::new();
    for mapping in provider.sdk_method_mappings() {
        if mapping.sdk() != &sdk {
            continue;
        }
        let MethodReference::Go(go_method) = mapping.method() else {
            continue;
        };
        methods_by_sdk
            .entry(go_method_key(go_method))
            .or_default()
            .extend(mapping.api_methods().iter().cloned());
    }

    let mut api_methods = BTreeSet::new();
    for go_method in cloudcover_go::analyze_dir(path)
        .map_err(|error| CliError::Runtime(format!("failed to analyze Go code: {error}")))?
    {
        if let Some(mapped_methods) = methods_by_sdk.get(&go_method_key(&go_method)) {
            api_methods.extend(mapped_methods.iter().cloned());
        }
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
