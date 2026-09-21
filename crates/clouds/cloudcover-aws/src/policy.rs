use std::collections::{BTreeMap, BTreeSet};

use cloudcover_core::ApiMethod;
use serde_json::json;

use crate::{AwsError, generated};

pub(crate) fn build_permissions_policy(
    methods: &[ApiMethod],
) -> Result<serde_json::Value, AwsError> {
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
        for action in resolve_actions(method)? {
            if action.permission == "sts:GetCallerIdentity" {
                continue;
            }
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
                let resource = if resource_types.is_empty() || !has_complete_resource_templates {
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

pub(crate) fn resolve_actions(
    method: &ApiMethod,
) -> Result<Vec<&'static crate::model::AwsAction>, AwsError> {
    let operation = find_operation(method)?;

    operation
        .authorized_actions
        .iter()
        .map(|authorized_action| find_action(authorized_action, method))
        .collect()
}

fn find_operation(method: &ApiMethod) -> Result<&'static crate::model::AwsOperation, AwsError> {
    let operation_index = generated::OPERATIONS
        .binary_search_by(|operation| {
            (operation.service, operation.name).cmp(&(method.service(), method.name()))
        })
        .map_err(|_| AwsError::UnknownApiMethod {
            service: method.service().to_owned(),
            name: method.name().to_owned(),
        })?;

    Ok(&generated::OPERATIONS[operation_index])
}

fn find_action(
    authorized_action: &crate::model::AwsApiMethodRef,
    method: &ApiMethod,
) -> Result<&'static crate::model::AwsAction, AwsError> {
    let action_index = generated::ACTIONS
        .binary_search_by(|action| {
            (action.service, action.name).cmp(&(authorized_action.service, authorized_action.name))
        })
        .map_err(|_| AwsError::UnknownApiMethod {
            service: method.service().to_owned(),
            name: method.name().to_owned(),
        })?;

    Ok(&generated::ACTIONS[action_index])
}
