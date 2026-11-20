//! Reviewed operation-to-action links absent from the upstream SAR operation table.
//! This never changes raw SAR documents or substitutes FAS for direct authorization.
use std::collections::BTreeSet;

use super::{BuildResult, OperationRow};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Supplement {
    schema_version: u8,
    mappings: Vec<Mapping>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Mapping {
    service: String,
    operation: String,
    authorized_actions: Vec<Action>,
    sources: Vec<Source>,
    reviewed: String,
    required_scope: Option<RequiredScope>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RequiredScope {
    resource_type: String,
    arn_format: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Action {
    service: String,
    name: String,
    #[serde(default)]
    context: Option<serde_json::Value>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    url: String,
    evidence: String,
}

pub(super) fn apply(
    operations: &mut [OperationRow],
    actions: &BTreeSet<(String, String)>,
) -> BuildResult<()> {
    apply_document(operations, actions, include_str!("data/authorization.json"))
}

pub(super) fn validate_upstream_contexts(
    service: &str,
    operations: &[super::ServiceOperation],
) -> BuildResult<()> {
    let supplement: Supplement = serde_json::from_str(include_str!("data/authorization.json"))?;
    for operation in operations {
        let Some(expected) = supplement
            .mappings
            .iter()
            .find(|mapping| mapping.service == service && mapping.operation == operation.name)
        else {
            continue;
        };
        if operation.authorized_actions.is_empty() {
            continue;
        }
        let mut seen = BTreeSet::new();
        if operation.authorized_actions.len() != expected.authorized_actions.len()
            || operation.authorized_actions.iter().any(|action| {
                !seen.insert((&action.service, &action.name))
                    || !expected.authorized_actions.iter().any(|reviewed| {
                        reviewed.service == action.service
                            && reviewed.name == action.name
                            && reviewed.context == action.context
                    })
            })
        {
            return Err(format!(
                "authorization supplement action/context conflicts with SAR for {service}:{}",
                operation.name
            )
            .into());
        }
    }
    Ok(())
}

pub(super) fn validate_scopes(service: &str, document: &super::ServiceDocument) -> BuildResult<()> {
    let supplement: Supplement = serde_json::from_str(include_str!("data/authorization.json"))?;
    for mapping in supplement.mappings.iter().filter(|m| m.service == service) {
        let Some(scope) = &mapping.required_scope else {
            continue;
        };
        let action = document
            .actions
            .iter()
            .find(|a| a.name == mapping.operation)
            .ok_or("missing scoped supplemental action")?;
        let resource = document
            .resources
            .iter()
            .find(|r| r.name == scope.resource_type)
            .ok_or("missing scoped supplemental resource")?;
        if mapping.authorized_actions.len() != 1
            || mapping.authorized_actions[0].service != service
            || mapping.authorized_actions[0].name != action.name
            || !action.condition_keys.is_empty()
            || action.resources.len() != 1
            || action.resources[0].name != scope.resource_type
            || !action.resources[0].condition_keys.is_empty()
            || resource.arn_formats != [scope.arn_format.clone()]
            || resource.condition_keys != ["aws:ResourceTag/${TagKey}"]
        {
            return Err(format!(
                "reviewed authorization scope changed for {service}:{}",
                mapping.operation
            )
            .into());
        }
    }
    Ok(())
}

#[cfg(test)]
fn validate_upstream_context(
    service: &str,
    operation: &super::ServiceOperation,
) -> BuildResult<()> {
    validate_upstream_contexts(service, std::slice::from_ref(operation))
}

fn apply_document(
    operations: &mut [OperationRow],
    actions: &BTreeSet<(String, String)>,
    document: &str,
) -> BuildResult<()> {
    let supplement: Supplement = serde_json::from_str(document)?;
    if supplement.schema_version != 1 {
        return Err("unsupported authorization supplement schema".into());
    }
    let mut seen = BTreeSet::new();
    for mapping in supplement.mappings {
        let identity = (&mapping.service, &mapping.operation);
        if !seen.insert((mapping.service.clone(), mapping.operation.clone()))
            || mapping.authorized_actions.is_empty()
            || mapping.sources.is_empty()
            || mapping.reviewed.len() != 10
            || mapping.sources.iter().any(|source| {
                !source.url.starts_with("https://docs.aws.amazon.com/")
                    || source.evidence.trim().is_empty()
            })
        {
            return Err(format!("invalid authorization supplement for {identity:?}").into());
        }
        let mut expected = mapping
            .authorized_actions
            .into_iter()
            .map(|action| (action.service, action.name))
            .collect::<Vec<_>>();
        expected.sort_unstable();
        if expected.windows(2).any(|pair| pair[0] == pair[1])
            || expected.iter().any(|action| !actions.contains(action))
        {
            return Err(format!("missing or duplicate authorized action for {identity:?}").into());
        }
        let row = operations
            .iter_mut()
            .find(|row| (&row.service, &row.name) == identity)
            .ok_or_else(|| format!("missing supplemented operation {identity:?}"))?;
        // An identical upstream fix is harmless. A changed upstream mapping must
        // be reviewed, never silently overwritten or unioned with a stale rule.
        if !row.authorized_actions.is_empty() && row.authorized_actions != expected {
            return Err(
                format!("authorization supplement conflicts with SAR for {identity:?}").into(),
            );
        }
        row.authorized_actions = expected;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odb_bridge_rejects_missing_wildcard_or_changed_scope() -> BuildResult<()> {
        let original = serde_json::json!({"Name":"odb", "Actions":[{
            "Name":"DisassociateVirtualMachinesFromExadbVmCluster",
            "Resources":[{"Name":"exadb-vm-cluster"}]}], "Resources":[{
            "Name":"exadb-vm-cluster", "ARNFormats":["arn:${Partition}:odb:${Region}:${Account}:exadb-vm-cluster/${ExadbVmClusterId}"],
            "ConditionKeys":["aws:ResourceTag/${TagKey}"]}]});
        validate_scopes("odb", &serde_json::from_value(original.clone())?)?;
        for (pointer, value) in [
            ("/Actions/0/Resources", serde_json::json!([])),
            (
                "/Actions/0/Resources",
                serde_json::json!([{"Name":"other"}]),
            ),
            (
                "/Actions/0/Resources",
                serde_json::json!([{"Name":"exadb-vm-cluster", "ConditionKeys":["new:Restriction"]}]),
            ),
            (
                "/Actions/0/ActionConditionKeys",
                serde_json::json!(["new:Restriction"]),
            ),
            ("/Resources", serde_json::json!([])),
            ("/Resources/0/ARNFormats", serde_json::json!([])),
            ("/Resources/0/ARNFormats", serde_json::json!(["*"])),
            (
                "/Resources/0/ARNFormats",
                serde_json::json!(["arn:${Partition}:odb:${Region}:${Account}:other/${Id}"]),
            ),
            (
                "/Resources/0/ConditionKeys",
                serde_json::json!(["new:Restriction"]),
            ),
        ] {
            let mut changed = original.clone();
            if pointer == "/Actions/0/ActionConditionKeys" {
                changed["Actions"][0]["ActionConditionKeys"] = value;
            } else {
                *changed
                    .pointer_mut(pointer)
                    .ok_or("missing fixture field")? = value;
            }
            assert!(
                validate_scopes("odb", &serde_json::from_value(changed)?).is_err(),
                "{pointer}"
            );
        }
        Ok(())
    }

    #[test]
    fn reviewed_context_shapes_accept_published_rows_and_reject_stricter_changes() -> BuildResult<()>
    {
        let supplement: Supplement = serde_json::from_str(include_str!("data/authorization.json"))?;
        for mapping in supplement.mappings {
            let actions: Vec<_> = mapping
                .authorized_actions
                .iter()
                .map(|a| {
                    let mut row = serde_json::json!({"Service": a.service, "Name": a.name});
                    if let Some(context) = &a.context {
                        row["Context"] = context.clone();
                    }
                    row
                })
                .collect();
            let row = serde_json::json!({"Name": mapping.operation, "AuthorizedActions": actions});
            validate_upstream_context(&mapping.service, &serde_json::from_value(row.clone())?)?;
            validate_upstream_context(
                &mapping.service,
                &serde_json::from_value(serde_json::json!({"Name": mapping.operation}))?,
            )?;
            for (index, action) in actions.iter().enumerate() {
                let mut changed = row.clone();
                changed["AuthorizedActions"][index]["Context"] =
                    serde_json::json!({"unreviewed:StricterCondition": ["required"]});
                assert!(
                    validate_upstream_context(&mapping.service, &serde_json::from_value(changed)?)
                        .is_err()
                );
                let mut explicit_null = row.clone();
                explicit_null["AuthorizedActions"][index]["Context"] = serde_json::Value::Null;
                assert!(
                    validate_upstream_context(
                        &mapping.service,
                        &serde_json::from_value(explicit_null)?
                    )
                    .is_err()
                );
                if action.get("Context").is_some() {
                    for context in [
                        serde_json::json!({}),
                        serde_json::json!({"iam:PassedToService": ["other.amazonaws.com"]}),
                        serde_json::Value::Null,
                    ] {
                        let mut changed = row.clone();
                        changed["AuthorizedActions"][index]["Context"] = context;
                        assert!(
                            validate_upstream_context(
                                &mapping.service,
                                &serde_json::from_value(changed)?
                            )
                            .is_err()
                        );
                    }
                    let mut changed = row.clone();
                    changed["AuthorizedActions"][index]
                        .as_object_mut()
                        .ok_or("action object missing")?
                        .remove("Context");
                    assert!(
                        validate_upstream_context(
                            &mapping.service,
                            &serde_json::from_value(changed)?
                        )
                        .is_err()
                    );
                }
            }
            let mut duplicate = row;
            duplicate["AuthorizedActions"]
                .as_array_mut()
                .ok_or("missing actions")?
                .push(actions[0].clone());
            assert!(
                validate_upstream_context(&mapping.service, &serde_json::from_value(duplicate)?)
                    .is_err()
            );
        }
        Ok(())
    }

    fn rows() -> Vec<OperationRow> {
        [
            ("s3", "CreateSession"),
            ("appstream", "CreateFleet"),
            ("appstream", "UpdateFleet"),
            ("appstream", "CreateImageBuilder"),
            ("appstream", "CreateStack"),
            ("appstream", "CreateUpdatedImage"),
            ("odb", "DisassociateVirtualMachinesFromExadbVmCluster"),
        ]
        .into_iter()
        .map(|(service, name)| OperationRow {
            service: service.into(),
            name: name.into(),
            authorized_actions: vec![],
        })
        .collect()
    }

    fn actions() -> BTreeSet<(String, String)> {
        [
            ("s3express", "CreateSession"),
            ("iam", "PassRole"),
            ("appstream", "CreateFleet"),
            ("appstream", "UpdateFleet"),
            ("appstream", "CreateImageBuilder"),
            ("appstream", "CreateStack"),
            ("appstream", "CreateUpdatedImage"),
            ("appstream", "TagResource"),
            ("odb", "DisassociateVirtualMachinesFromExadbVmCluster"),
        ]
        .into_iter()
        .map(|(s, n)| (s.into(), n.into()))
        .collect()
    }

    #[test]
    fn applies_documented_bridge_and_accepts_identical_upstream_fix() -> BuildResult<()> {
        let mut rows = rows();
        apply(&mut rows, &actions())?;
        assert_eq!(
            rows[0].authorized_actions,
            [("s3express".into(), "CreateSession".into())]
        );
        for row in rows.iter().filter(|row| row.service == "appstream") {
            assert!(
                row.authorized_actions
                    .contains(&("appstream".into(), row.name.clone()))
            );
        }
        apply(&mut rows, &actions())?;
        Ok(())
    }

    #[test]
    fn rejects_missing_references_and_conflicting_upstream_authorization() {
        assert!(apply(&mut [], &actions()).is_err());
        assert!(apply(&mut rows(), &BTreeSet::new()).is_err());
        let mut rows = rows();
        rows[0].authorized_actions = vec![("s3".into(), "GetObject".into())];
        assert!(apply(&mut rows, &actions()).is_err());
    }

    #[test]
    fn every_bridge_rejects_future_disagreement_and_missing_action_references() {
        for index in 0..rows().len() {
            let mut conflicting = rows();
            conflicting[index].authorized_actions = vec![("unreviewed".into(), "Action".into())];
            assert!(apply(&mut conflicting, &actions()).is_err());
        }
        for missing in actions() {
            let mut keys = actions();
            keys.remove(&missing);
            assert!(apply(&mut rows(), &keys).is_err(), "{missing:?}");
        }
    }

    #[test]
    fn rejects_invalid_schema_evidence_and_duplicate_mappings() -> BuildResult<()> {
        let document: serde_json::Value =
            serde_json::from_str(include_str!("data/authorization.json"))?;
        for field in ["sources", "authorized_actions"] {
            let mut invalid = document.clone();
            invalid["mappings"][0][field] = serde_json::json!([]);
            assert!(apply_document(&mut rows(), &actions(), &invalid.to_string()).is_err());
        }
        let mut invalid = document.clone();
        invalid["schema_version"] = serde_json::json!(2);
        assert!(apply_document(&mut rows(), &actions(), &invalid.to_string()).is_err());
        let mut invalid = document.clone();
        invalid["mappings"] = serde_json::json!([document["mappings"][0], document["mappings"][0]]);
        assert!(apply_document(&mut rows(), &actions(), &invalid.to_string()).is_err());
        Ok(())
    }
}
