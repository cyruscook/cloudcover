use std::collections::{BTreeMap, BTreeSet};

use cloudcover_core::{
    ApiMethod, CloudProvider, GoMethodReference, Language, MethodReference, PythonMethodReference,
    Sdk, SdkMethodMapping,
};
use serde_json::json;

#[derive(Clone, Copy, Debug, Default)]
pub struct AwsProvider;

impl AwsProvider {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[derive(Clone, Copy, Debug)]
struct AwsAction {
    service: &'static str,
    name: &'static str,
    permission: &'static str,
    resource_types: &'static [&'static str],
    resource_templates: &'static [&'static str],
    has_complete_resource_templates: bool,
}

#[derive(Clone, Copy, Debug)]
struct AwsApiMethodRef {
    service: &'static str,
    name: &'static str,
}

#[derive(Clone, Copy, Debug)]
struct AwsOperation {
    service: &'static str,
    name: &'static str,
    authorized_actions: &'static [AwsApiMethodRef],
}

#[derive(Clone, Copy, Debug)]
struct AwsSdkMethodMapping {
    sdk_package: &'static str,
    sdk_name: &'static str,
    sdk_method: &'static str,
    api_service: &'static str,
    api_name: &'static str,
}

mod generated {
    include!(concat!(env!("OUT_DIR"), "/actions.rs"));
}

fn operation_exists(service: &str, name: &str) -> bool {
    generated::OPERATIONS
        .binary_search_by(|operation| (operation.service, operation.name).cmp(&(service, name)))
        .is_ok()
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
        ]
    }

    fn sdk_method_mappings(&self) -> Vec<SdkMethodMapping> {
        let mut mappings = Vec::new();

        mappings.extend(generated::SDK_METHOD_MAPPINGS.iter().map(|row| {
            SdkMethodMapping::new(
                Sdk::new(row.sdk_package, Language::Python),
                MethodReference::Python(PythonMethodReference::new(
                    "boto3",
                    Some(row.sdk_name.to_owned()),
                    row.sdk_method,
                )),
                vec![ApiMethod::new(row.api_service, row.api_name)],
            )
        }));

        mappings.extend(
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
                }),
        );

        mappings.sort();
        mappings.dedup();
        mappings
    }

    fn permissions_policy(&self, methods: &[ApiMethod]) -> Result<serde_json::Value, Self::Error> {
        let mut statements = BTreeMap::<
            (
                &'static str,
                &'static [&'static str],
                &'static [&'static str],
                bool,
            ),
            BTreeSet<&'static str>,
        >::new();

        for method in methods {
            let operation_index = generated::OPERATIONS
                .binary_search_by(|operation| {
                    (operation.service, operation.name).cmp(&(method.service(), method.name()))
                })
                .map_err(|_| AwsError::UnknownApiMethod {
                    service: method.service().to_owned(),
                    name: method.name().to_owned(),
                })?;
            let operation = &generated::OPERATIONS[operation_index];

            for authorized_action in operation.authorized_actions {
                let action_index = generated::ACTIONS
                    .binary_search_by(|action| {
                        (action.service, action.name)
                            .cmp(&(authorized_action.service, authorized_action.name))
                    })
                    .map_err(|_| AwsError::UnknownApiMethod {
                        service: method.service().to_owned(),
                        name: method.name().to_owned(),
                    })?;
                let action = &generated::ACTIONS[action_index];
                statements
                    .entry((
                        action.service,
                        action.resource_types,
                        action.resource_templates,
                        action.has_complete_resource_templates,
                    ))
                    .or_default()
                    .insert(action.permission);
            }
        }

        if statements.is_empty() {
            return Ok(json!({"Version":"2012-10-17","Statement":[]}));
        }

        let statements = statements
            .into_iter()
            .map(
                |(
                    (_service, resource_types, resource_templates, has_complete_resource_templates),
                    actions,
                )| {
                    let resource = if resource_types.is_empty() || !has_complete_resource_templates
                    {
                        json!("*")
                    } else {
                        match resource_templates {
                            [template] => json!(template),
                            _ => json!(resource_templates),
                        }
                    };

                    json!({
                        "Effect": "Allow",
                        "Action": actions.into_iter().collect::<Vec<_>>(),
                        "Resource": resource,
                    })
                },
            )
            .collect::<Vec<_>>();

        Ok(json!({
            "Version": "2012-10-17",
            "Statement": statements,
        }))
    }
}
