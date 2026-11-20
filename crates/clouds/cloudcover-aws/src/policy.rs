use std::collections::{BTreeMap, BTreeSet};

use cloudcover_core::ApiMethod;
use serde_json::json;

use crate::{AwsError, ReviewedPolicy, conditional, fas, generated, model::AwsAction};

pub(crate) fn build_permissions_policy(
    methods: &[ApiMethod],
) -> Result<serde_json::Value, AwsError> {
    require_review(review_permissions_policy(methods, false)?)
}

fn require_review<T>(review: ReviewedPolicy<T>) -> Result<T, AwsError> {
    if let Some(first) = review.conditional_permissions.first() {
        return Err(AwsError::ConditionalAuthorization {
            service: first.service.to_owned(),
            name: first.operation.to_owned(),
            permission: first.permission.to_owned(),
            source_url: first.source,
        });
    }
    Ok(review.policy)
}

pub(crate) fn review_permissions_policy(
    methods: &[ApiMethod],
    include_conditional: bool,
) -> Result<ReviewedPolicy<serde_json::Value>, AwsError> {
    let (statements, conditional_permissions) =
        build_policy_statements(methods, include_conditional)?;
    let policy = render_json(statements);
    Ok(ReviewedPolicy {
        policy,
        conditional_permissions,
    })
}

fn render_json(statements: Vec<PolicyStatement>) -> serde_json::Value {
    json!({
        "Version": "2012-10-17",
        "Statement": statements.into_iter().map(|statement| {
            let resource = match statement.scope.resources.as_slice() {
                [resource] => json!(resource),
                _ => json!(statement.scope.resources),
            };
            let mut value = json!({"Effect": "Allow", "Action": statement.actions, "Resource": resource});
            if let Some(via_service) = statement.scope.via_service {
                value["Condition"] = json!({condition_test(&via_service): {"kms:ViaService": via_service}});
            }
            value
        }).collect::<Vec<_>>(),
    })
}

pub(crate) fn build_permissions_policy_hcl(methods: &[ApiMethod]) -> Result<String, AwsError> {
    require_review(review_permissions_policy_hcl(methods, false)?)
}

pub(crate) fn review_permissions_policy_hcl(
    methods: &[ApiMethod],
    include_conditional: bool,
) -> Result<ReviewedPolicy<String>, AwsError> {
    let (statements, conditional_permissions) =
        build_policy_statements(methods, include_conditional)?;
    let policy = render_hcl(statements)?;
    Ok(ReviewedPolicy {
        policy,
        conditional_permissions,
    })
}

fn render_hcl(statements: Vec<PolicyStatement>) -> Result<String, AwsError> {
    let statement_blocks = statements.into_iter().map(|statement| {
        let mut builder = hcl::Block::builder("statement")
            .add_attribute(("effect", "Allow"))
            .add_attribute(("actions", statement.actions))
            .add_attribute(("resources", statement.scope.resources));
        if let Some(via_service) = statement.scope.via_service {
            builder = builder.add_block(
                hcl::Block::builder("condition")
                    .add_attribute(("test", condition_test(&via_service)))
                    .add_attribute(("variable", "kms:ViaService"))
                    .add_attribute(("values", vec![via_service]))
                    .build(),
            );
        }
        builder.build()
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

fn condition_test(via_service: &str) -> &'static str {
    if via_service.contains('*') {
        "StringLike"
    } else {
        "StringEquals"
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct PolicyScope {
    service: &'static str,
    resources: Vec<String>,
    via_service: Option<String>,
    // Keep independently replaceable source/destination/replica keys separate.
    resource_role: String,
}

struct PolicyStatement {
    scope: PolicyScope,
    actions: Vec<&'static str>,
}

fn checked_resources(action: &AwsAction) -> Result<Vec<String>, AwsError> {
    // A declared but unresolved scope is not evidence for a global grant or
    // for dropping the unknown member of a partially resolved resource union.
    // Check before any reviewed optional-agent wildcard exception.
    if !action.has_complete_resource_templates
        || (!action.resource_types.is_empty() && action.resource_templates.is_empty())
    {
        return Err(AwsError::UnresolvedAuthorization {
            service: action.service.to_owned(),
            name: action.name.to_owned(),
        });
    }
    // This reviewed bridge is valid only with the exact retained cluster scope.
    // Recheck at rendering too: incomplete metadata must never produce "*".
    if action.service == "odb"
        && action.name == "DisassociateVirtualMachinesFromExadbVmCluster"
        && (!action.has_complete_resource_templates
            || action.resource_types != ["exadb-vm-cluster"]
            || action.resource_templates
                != [
                    "arn:${Partition}:odb:${Region}:${Account}:exadb-vm-cluster/${ExadbVmClusterId}",
                ])
    {
        return Err(AwsError::UnresolvedAuthorization {
            service: action.service.to_owned(),
            name: action.name.to_owned(),
        });
    }
    Ok(resources(action))
}

fn resources(action: &AwsAction) -> Vec<String> {
    // AgentArns is optional for these APIs (S3 uses it only on Outposts).
    // SAR can list an agent resource even though agentless requests need "*".
    // Source analysis does not resolve request arguments, so support both paths.
    // https://docs.aws.amazon.com/datasync/latest/apireference/API_CreateLocationS3.html
    // https://docs.aws.amazon.com/datasync/latest/apireference/API_CreateLocationAzureBlob.html
    // https://docs.aws.amazon.com/datasync/latest/apireference/API_CreateLocationObjectStorage.html
    let optional_agent = action.service == "datasync"
        && matches!(
            action.name,
            "CreateLocationS3" | "CreateLocationAzureBlob" | "CreateLocationObjectStorage"
        );
    if optional_agent || action.resource_types.is_empty() || !action.has_complete_resource_templates
    {
        vec!["*".to_owned()]
    } else {
        action
            .resource_templates
            .iter()
            .map(|value| (*value).to_owned())
            .collect()
    }
}

fn build_policy_statements(
    methods: &[ApiMethod],
    include_conditional: bool,
) -> Result<(Vec<PolicyStatement>, Vec<crate::ConditionalPermission>), AwsError> {
    build_policy_statements_with(methods, include_conditional, resolve_direct_actions)
}

fn build_policy_statements_with(
    methods: &[ApiMethod],
    include_conditional: bool,
    resolve: impl FnMut(&ApiMethod) -> Result<Vec<&'static AwsAction>, AwsError>,
) -> Result<(Vec<PolicyStatement>, Vec<crate::ConditionalPermission>), AwsError> {
    build_policy_statements_with_resolvers(
        methods,
        include_conditional,
        resolve,
        resolve_direct_actions,
    )
}

fn build_policy_statements_with_resolvers(
    methods: &[ApiMethod],
    include_conditional: bool,
    resolve: impl FnMut(&ApiMethod) -> Result<Vec<&'static AwsAction>, AwsError>,
    resolve_downstream: impl FnMut(&ApiMethod) -> Result<Vec<&'static AwsAction>, AwsError>,
) -> Result<(Vec<PolicyStatement>, Vec<crate::ConditionalPermission>), AwsError> {
    build_policy_statements_with_guard(
        methods,
        include_conditional,
        resolve,
        resolve_downstream,
        crate::disposition::ensure_policy_supported,
    )
}

fn build_policy_statements_with_guard(
    methods: &[ApiMethod],
    include_conditional: bool,
    mut resolve: impl FnMut(&ApiMethod) -> Result<Vec<&'static AwsAction>, AwsError>,
    mut resolve_downstream: impl FnMut(&ApiMethod) -> Result<Vec<&'static AwsAction>, AwsError>,
    mut ensure_supported: impl FnMut(&ApiMethod) -> Result<(), AwsError>,
) -> Result<(Vec<PolicyStatement>, Vec<crate::ConditionalPermission>), AwsError> {
    let mut statements = BTreeMap::<PolicyScope, BTreeSet<&'static str>>::new();
    let mut diagnostics = Vec::new();
    for method in methods {
        ensure_supported(method)?;
        let direct = resolve(method)?;
        ensure_ordinary_authorization(method, &direct)?;
        let (ordinary, optional): (Vec<&AwsAction>, Vec<&AwsAction>) = direct
            .into_iter()
            .partition(|action| conditional::classify(method, action).is_none());
        for action in ordinary {
            statements
                .entry(PolicyScope {
                    service: action.service,
                    resources: checked_resources(action)?,
                    via_service: None,
                    resource_role: String::new(),
                })
                .or_default()
                .insert(action.permission);
        }
        for action in optional {
            if let Some(diagnostic) = conditional::classify(method, action) {
                if include_conditional && diagnostic.manual_step.is_none() {
                    statements
                        .entry(PolicyScope {
                            service: action.service,
                            resources: checked_resources(action)?,
                            via_service: None,
                            resource_role: String::new(),
                        })
                        .or_default()
                        .insert(action.permission);
                }
                diagnostics.push(diagnostic);
            }
        }
        for dependency in fas::dependencies(method)? {
            let rule = dependency.rule;
            for operation in &rule.downstream_operations {
                ensure_supported(&ApiMethod::new(&rule.downstream_service, operation))?;
                for action in
                    resolve_downstream(&ApiMethod::new(&rule.downstream_service, operation))?
                {
                    let mut resources = checked_resources(action)?;
                    if let Some(key) = &rule.key_placeholder {
                        for resource in &mut resources {
                            *resource = resource
                                .replace("${KeyId}", &format!("${{{key}}}"))
                                .replace("${Region}", &format!("${{{}}}", rule.region_placeholder));
                        }
                    }
                    statements
                        .entry(PolicyScope {
                            service: action.service,
                            resources,
                            via_service: rule.via_service.clone(),
                            resource_role: rule.resource_role.clone(),
                        })
                        .or_default()
                        .insert(action.permission);
                }
            }
        }
    }
    diagnostics.sort_unstable_by_key(|d| (d.service, d.operation, d.permission));
    diagnostics.dedup();
    Ok((
        statements
            .into_iter()
            .map(|(scope, actions)| PolicyStatement {
                scope,
                actions: actions.into_iter().collect(),
            })
            .collect(),
        diagnostics,
    ))
}

fn ensure_ordinary_authorization(
    method: &ApiMethod,
    actions: &[&AwsAction],
) -> Result<(), AwsError> {
    if !actions.is_empty()
        && actions
            .iter()
            .all(|action| conditional::classify(method, action).is_some())
    {
        return Err(AwsError::UnresolvedAuthorization {
            service: method.service().to_owned(),
            name: method.name().to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn resolve_actions(method: &ApiMethod) -> Result<Vec<&'static AwsAction>, AwsError> {
    let mut actions = resolve_direct_actions(method)?;
    for dependency in fas::dependencies(method)? {
        for operation in &dependency.rule.downstream_operations {
            actions.extend(resolve_direct_actions(&ApiMethod::new(
                &dependency.rule.downstream_service,
                operation,
            ))?);
        }
    }
    actions.sort_unstable_by_key(|action| action.permission);
    actions.dedup_by_key(|action| action.permission);
    Ok(actions)
}

pub(crate) fn resolve_direct_actions(
    method: &ApiMethod,
) -> Result<Vec<&'static crate::model::AwsAction>, AwsError> {
    let operation = find_operation(method)?;

    resolve_operation_actions(method, operation)
}

fn resolve_operation_actions(
    method: &ApiMethod,
    operation: &crate::model::AwsOperation,
) -> Result<Vec<&'static crate::model::AwsAction>, AwsError> {
    match operation.authorization_coverage() {
        crate::AuthorizationCoverage::NotRequired => return Ok(Vec::new()),
        crate::AuthorizationCoverage::Unresolved => {
            return Err(AwsError::UnresolvedAuthorization {
                service: method.service().to_owned(),
                name: method.name().to_owned(),
            });
        }
        crate::AuthorizationCoverage::Mapped => {}
    }

    operation
        .authorized_actions
        .iter()
        .map(|authorized_action| find_action(authorized_action, method))
        .collect()
}

pub(crate) fn find_operation(
    method: &ApiMethod,
) -> Result<&'static crate::model::AwsOperation, AwsError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disposition_guard_blocks_fas_before_rendering_partial_source_grants() -> Result<(), AwsError>
    {
        for include in [false, true] {
            for hcl in [false, true] {
                let mut downstream_seen = false;
                let result = build_policy_statements_with_guard(
                    &[ApiMethod::new("s3", "GetObject")],
                    include,
                    resolve_direct_actions,
                    resolve_direct_actions,
                    |method| {
                        if method.service() == "kms" && method.name() == "Decrypt" {
                            downstream_seen = true;
                            Err(AwsError::UnresolvedAuthorization {
                                service: method.service().to_owned(),
                                name: method.name().to_owned(),
                            })
                        } else {
                            crate::disposition::ensure_policy_supported(method)
                        }
                    },
                )
                .and_then(|(statements, _)| {
                    if hcl {
                        render_hcl(statements)
                    } else {
                        Ok(render_json(statements).to_string())
                    }
                });
                assert!(downstream_seen);
                assert!(
                    matches!(result, Err(AwsError::UnresolvedAuthorization {service, name}) if service=="kms" && name=="Decrypt")
                );
            }
        }
        Ok(())
    }

    #[test]
    fn incomplete_fas_leaf_blocks_both_renderers_without_partial_policy() -> Result<(), AwsError> {
        let leaf = resolve_direct_actions(&ApiMethod::new("kms", "Decrypt"))?[0];
        let broken = Box::leak(Box::new(AwsAction {
            resource_templates: &[],
            has_complete_resource_templates: false,
            ..*leaf
        }));
        for include in [false, true] {
            for hcl in [false, true] {
                let result = build_policy_statements_with_resolvers(
                    &[ApiMethod::new("s3", "GetObject")],
                    include,
                    resolve_direct_actions,
                    |method| {
                        if method.service() == "kms" && method.name() == "Decrypt" {
                            Ok(vec![broken])
                        } else {
                            resolve_direct_actions(method)
                        }
                    },
                )
                .and_then(|(statements, _)| {
                    if hcl {
                        render_hcl(statements)
                    } else {
                        Ok(render_json(statements).to_string())
                    }
                });
                assert!(
                    matches!(result, Err(AwsError::UnresolvedAuthorization { service, name })
                    if service == "kms" && name == "Decrypt")
                );
            }
        }
        Ok(())
    }

    #[test]
    fn incomplete_scope_never_renders_global_or_partial_permissions() -> Result<(), AwsError> {
        let valid = resolve_direct_actions(&ApiMethod::new("s3", "GetObject"))?[0];
        for action in [
            AwsAction {
                resource_types: &["missing"],
                resource_templates: &[],
                has_complete_resource_templates: false,
                ..*valid
            },
            AwsAction {
                resource_types: &["empty"],
                resource_templates: &[],
                has_complete_resource_templates: true,
                ..*valid
            },
            AwsAction {
                resource_types: &["known", "missing"],
                resource_templates: &["arn:test:known/${Id}"],
                has_complete_resource_templates: false,
                ..*valid
            },
            AwsAction {
                has_complete_resource_templates: false,
                ..agent_action("CreateLocationS3")
            },
        ] {
            let broken = Box::leak(Box::new(action));
            assert!(checked_resources(broken).is_err());
            for include in [false, true] {
                for mixed in [false, true] {
                    let methods = if mixed {
                        vec![
                            ApiMethod::new("s3", "GetObject"),
                            ApiMethod::new("s3", "PutObject"),
                        ]
                    } else {
                        vec![ApiMethod::new("s3", "PutObject")]
                    };
                    for hcl in [false, true] {
                        let result = build_policy_statements_with(&methods, include, |method| {
                            Ok(if method.name() == "PutObject" {
                                vec![valid, broken]
                            } else {
                                vec![valid]
                            })
                        })
                        .and_then(|(statements, _)| {
                            if hcl {
                                render_hcl(statements)
                            } else {
                                Ok(render_json(statements).to_string())
                            }
                        });
                        assert!(matches!(
                            result,
                            Err(AwsError::UnresolvedAuthorization { .. })
                        ));
                    }
                }
            }
        }
        let global = AwsAction {
            resource_types: &[],
            resource_templates: &[],
            has_complete_resource_templates: true,
            ..*valid
        };
        assert_eq!(checked_resources(&global)?, ["*"]);
        assert_eq!(checked_resources(&agent_action("CreateLocationS3"))?, ["*"]);
        Ok(())
    }

    #[test]
    fn incomplete_conditional_scope_blocks_opt_in_and_keeps_ordinary_diagnostics()
    -> Result<(), AwsError> {
        static OPTIONAL: AwsAction = AwsAction {
            service: "dynamodb",
            name: "ReadDataForReplication",
            permission: "dynamodb:ReadDataForReplication",
            resource_types: &["table"],
            resource_templates: &[],
            has_complete_resource_templates: false,
        };
        let method = ApiMethod::new("dynamodb", "Scan");
        let base = resolve_direct_actions(&method)?
            .into_iter()
            .find(|action| action.permission == "dynamodb:Scan")
            .ok_or_else(|| AwsError::UnresolvedAuthorization {
                service: method.service().to_owned(),
                name: method.name().to_owned(),
            })?;
        for hcl in [false, true] {
            for include in [false, true] {
                let result =
                    build_policy_statements_with(std::slice::from_ref(&method), include, |_| {
                        Ok(vec![base, &OPTIONAL])
                    });
                if include {
                    assert!(matches!(
                        result,
                        Err(AwsError::UnresolvedAuthorization { .. })
                    ));
                } else {
                    let (statements, diagnostics) = result?;
                    assert_eq!(diagnostics.len(), 1);
                    assert_eq!(diagnostics[0].permission, OPTIONAL.permission);
                    let output = if hcl {
                        render_hcl(statements)?
                    } else {
                        render_json(statements).to_string()
                    };
                    assert!(!output.contains(OPTIONAL.permission));
                    assert!(output.contains("dynamodb:Scan"));
                }
            }
        }
        Ok(())
    }

    #[test]
    fn odb_renderers_reject_scope_fallback_even_when_injected_after_compilation()
    -> Result<(), AwsError> {
        let original = resolve_direct_actions(&ApiMethod::new(
            "odb",
            "DisassociateVirtualMachinesFromExadbVmCluster",
        ))?[0];
        for action in [
            AwsAction {
                resource_types: &[],
                resource_templates: &[],
                ..*original
            },
            AwsAction {
                resource_templates: &["*"],
                ..*original
            },
            AwsAction {
                resource_templates: &[],
                ..*original
            },
            AwsAction {
                has_complete_resource_templates: false,
                ..*original
            },
            AwsAction {
                resource_types: &["other"],
                ..*original
            },
        ] {
            let action = Box::leak(Box::new(action));
            for include in [false, true] {
                for hcl in [false, true] {
                    let result = build_policy_statements_with(
                        &[ApiMethod::new(
                            "odb",
                            "DisassociateVirtualMachinesFromExadbVmCluster",
                        )],
                        include,
                        |_| Ok(vec![action]),
                    )
                    .and_then(|(statements, _)| {
                        if hcl {
                            render_hcl(statements)
                        } else {
                            Ok(render_json(statements).to_string())
                        }
                    });
                    assert!(matches!(
                        result,
                        Err(AwsError::UnresolvedAuthorization { .. })
                    ));
                }
            }
        }
        Ok(())
    }

    #[test]
    fn incomplete_passrole_metadata_is_diagnostic_only_in_both_renderers() -> Result<(), AwsError> {
        static BASE: AwsAction = AwsAction {
            service: "appstream",
            name: "CreateFleet",
            permission: "appstream:CreateFleet",
            resource_types: &[],
            resource_templates: &[],
            // An explicit global action has no declared resource references;
            // the compiler marks that scope complete. Only ROLE is incomplete.
            has_complete_resource_templates: true,
        };
        static ROLE: AwsAction = AwsAction {
            service: "iam",
            name: "PassRole",
            permission: "iam:PassRole",
            resource_types: &["role"],
            resource_templates: &[],
            has_complete_resource_templates: false,
        };
        assert_eq!(resources(&ROLE), ["*"]);
        for include in [false, true] {
            for hcl in [false, true] {
                let (statements, diagnostics) = build_policy_statements_with(
                    &[ApiMethod::new("appstream", "CreateFleet")],
                    include,
                    |_| Ok(vec![&BASE, &ROLE]),
                )?;
                assert_eq!(diagnostics.len(), 1);
                assert!(diagnostics[0].manual_step.is_some());
                assert_eq!(
                    diagnostics[0].passed_to_service,
                    Some("appstream.amazonaws.com")
                );
                let rendered = if hcl {
                    render_hcl(statements)?
                } else {
                    render_json(statements).to_string()
                };
                assert!(rendered.contains("appstream:CreateFleet"));
                assert!(!rendered.contains("iam:PassRole"));
            }
        }
        Ok(())
    }

    #[test]
    fn classification_is_tuple_specific_and_cannot_remove_all_direct_authorization()
    -> Result<(), AwsError> {
        let method = ApiMethod::new("appstream", "CreateFleet");
        let actions = resolve_direct_actions(&method)?;
        let optional: Vec<_> = actions
            .iter()
            .copied()
            .filter(|a| conditional::classify(&method, a).is_some())
            .collect();
        assert_ne!(optional.len(), 0);
        assert!(matches!(
            ensure_ordinary_authorization(&method, &optional),
            Err(AwsError::UnresolvedAuthorization { .. })
        ));
        ensure_ordinary_authorization(&method, &actions)?;
        for action in optional {
            assert!(
                conditional::classify(&ApiMethod::new("appstream", "TagResource"), action)
                    .is_none()
            );
            assert!(
                conditional::classify(&ApiMethod::new("other", "CreateFleet"), action).is_none()
            );
        }
        // Preserve existing optional Lambda PassRole/TagResource links and the
        // same runtime action on unrelated operations.
        for (service, operation, action_service, action_name) in [
            ("lambda", "CreateFunction", "iam", "PassRole"),
            ("lambda", "CreateFunction", "lambda", "TagResource"),
            (
                "lambda",
                "PutRuntimeManagementConfig",
                "lambda",
                "AssociateRuntimeVersion",
            ),
        ] {
            let action = AwsAction {
                service: action_service,
                name: action_name,
                permission: "test",
                resource_types: &[],
                resource_templates: &[],
                has_complete_resource_templates: true,
            };
            assert!(conditional::classify(&ApiMethod::new(service, operation), &action).is_none());
        }
        Ok(())
    }

    const AGENT: &str = "arn:${Partition}:datasync:${Region}:${AccountId}:agent/${AgentId}";
    const LOCATION: &str =
        "arn:${Partition}:datasync:${Region}:${AccountId}:location/${LocationId}";

    #[test]
    fn pending_catalog_operations_with_missing_authorization_are_rejected() {
        // Exact unresolved operations in SAR refresh #57. Exercise the policy
        // resolver without making the test depend on merging that data update.
        for (service, name) in [
            ("s3", "CreateSession"), // FAS alone cannot resolve a missing direct link.
            ("ec2", "GetCapacityManagerSupportabilityScores"),
            ("neptune-db", "CancelIndexRebuild"),
            ("neptune-db", "GetIndexRebuildStatus"),
            ("neptune-db", "GetStorageInfo"),
            ("neptune-db", "ListIndexRebuilds"),
            ("neptune-db", "StartIndexRebuild"),
            ("opensearch", "FailoverReplication"),
        ] {
            let method = ApiMethod::new(service, name);
            let operation = crate::model::AwsOperation {
                service,
                name,
                authorized_actions: &[],
            };
            assert!(matches!(
                resolve_operation_actions(&method, &operation),
                Err(AwsError::UnresolvedAuthorization { service: s, name: n })
                    if s == service && n == name
            ));
        }
    }

    fn agent_action(name: &'static str) -> AwsAction {
        // Model the agent metadata introduced by the pending SAR refresh, even
        // while main's pinned catalog still has no resources for these actions.
        AwsAction {
            service: "datasync",
            name,
            permission: "unused",
            resource_types: &["agent"],
            resource_templates: &[AGENT],
            has_complete_resource_templates: true,
        }
    }

    #[test]
    fn optional_agent_creation_keeps_wildcard_with_agent_metadata() {
        for name in [
            "CreateLocationS3",
            "CreateLocationAzureBlob",
            "CreateLocationObjectStorage",
        ] {
            assert_eq!(resources(&agent_action(name)), ["*"], "{name}");
        }
    }

    #[test]
    fn required_agents_and_location_updates_retain_their_resources() {
        for name in [
            "CreateLocationNfs",
            "CreateLocationSmb",
            "CreateLocationHdfs",
        ] {
            assert_eq!(resources(&agent_action(name)), [AGENT], "{name}");
        }
        for name in [
            "UpdateLocationS3",
            "UpdateLocationAzureBlob",
            "UpdateLocationObjectStorage",
            "UpdateLocationNfs",
            "UpdateLocationSmb",
            "UpdateLocationHdfs",
        ] {
            let action = AwsAction {
                resource_types: &["agent", "location"],
                resource_templates: &[AGENT, LOCATION],
                ..agent_action(name)
            };
            assert_eq!(resources(&action), [AGENT, LOCATION], "{name}");
        }
    }

    #[test]
    fn optional_agent_override_is_service_specific_and_keeps_existing_fallbacks() {
        let action = AwsAction {
            service: "another-service",
            ..agent_action("CreateLocationS3")
        };
        assert_eq!(resources(&action), [AGENT]);
        let action = AwsAction {
            has_complete_resource_templates: false,
            ..agent_action("CreateLocationNfs")
        };
        assert_eq!(resources(&action), ["*"]);
        let action = AwsAction {
            resource_types: &[],
            resource_templates: &[],
            ..agent_action("CreateLocationNfs")
        };
        assert_eq!(resources(&action), ["*"]);
    }
}
