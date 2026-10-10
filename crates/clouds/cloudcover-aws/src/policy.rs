use std::collections::{BTreeMap, BTreeSet};

use cloudcover_core::ApiMethod;
use serde_json::json;

use crate::{AwsError, generated};

pub(crate) fn build_permissions_policy(
    methods: &[ApiMethod],
) -> Result<serde_json::Value, AwsError> {
    let statements = build_policy_statements(methods)?;

    Ok(json!({
        "Version": "2012-10-17",
        "Statement": statements
            .into_iter()
            .map(|statement| {
                let resource = match statement.resources.as_slice() {
                    [resource] => json!(resource),
                    _ => json!(statement.resources),
                };
                json!({
                    "Effect": "Allow",
                    "Action": statement.actions,
                    "Resource": resource,
                })
            })
            .collect::<Vec<_>>(),
    }))
}

pub(crate) fn build_permissions_policy_hcl(methods: &[ApiMethod]) -> Result<String, AwsError> {
    let statements = build_policy_statements(methods)?;
    let statement_blocks = statements.into_iter().map(|statement| {
        hcl::Block::builder("statement")
            .add_attribute(("effect", "Allow"))
            .add_attribute(("actions", statement.actions))
            .add_attribute(("resources", statement.resources))
            .build()
    });
    let body = hcl::Body::builder()
        .add_block(
            hcl::Block::builder("data")
                .add_label("aws_iam_policy_document")
                .add_label("cloudcover")
                .add_blocks(statement_blocks)
                .build(),
        )
        .build();

    hcl::format::to_string(&body).map_err(|source| AwsError::HclSerialization { source })
}

fn build_policy_statements(methods: &[ApiMethod]) -> Result<Vec<PolicyStatement>, AwsError> {
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

    Ok(statements
        .into_iter()
        .map(
            |(
                (_service, resource_types, resource_templates, has_complete_resource_templates),
                actions,
            )| {
                let resources = if resource_types.is_empty() || !has_complete_resource_templates {
                    vec!["*"]
                } else {
                    resource_templates.to_vec()
                };

                PolicyStatement {
                    actions: actions.into_iter().collect(),
                    resources,
                }
            },
        )
        .collect())
}

struct PolicyStatement {
    actions: Vec<&'static str>,
    resources: Vec<&'static str>,
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
