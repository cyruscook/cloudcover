use cloudcover_aws::{AwsError, AwsProvider};
use cloudcover_core::{ApiMethod, CloudProvider};
use std::{collections::BTreeSet, error::Error};

fn actions(policy: &serde_json::Value) -> Result<BTreeSet<String>, Box<dyn Error>> {
    policy["Statement"]
        .as_array()
        .ok_or("missing statements")?
        .iter()
        .flat_map(|statement| statement["Action"].as_array().into_iter().flatten())
        .map(|value| Ok(value.as_str().ok_or("invalid action")?.to_owned()))
        .collect()
}

#[test]
fn appstream_defaults_keep_only_reviewed_base_and_opt_in_scopes_role() -> Result<(), Box<dyn Error>>
{
    let provider = AwsProvider::new();
    for name in [
        "CreateFleet",
        "UpdateFleet",
        "CreateImageBuilder",
        "CreateStack",
        "CreateUpdatedImage",
    ] {
        let method = ApiMethod::new("appstream", name);
        let ordinary = provider.review_permissions_policy(std::slice::from_ref(&method), false)?;
        assert_eq!(
            actions(&ordinary.policy)?,
            BTreeSet::from([format!("appstream:{name}")])
        );
        assert_eq!(
            ordinary.conditional_permissions.len(),
            if matches!(name, "CreateFleet" | "CreateImageBuilder") {
                2
            } else {
                1
            }
        );
        assert!(matches!(
            provider.permissions_policy(std::slice::from_ref(&method)),
            Err(AwsError::ConditionalAuthorization { .. })
        ));
        assert!(matches!(
            provider.permissions_policy_hcl(std::slice::from_ref(&method)),
            Err(AwsError::ConditionalAuthorization { .. })
        ));
        let advanced = provider.review_permissions_policy(std::slice::from_ref(&method), true)?;
        assert_eq!(
            advanced.conditional_permissions,
            ordinary.conditional_permissions
        );
        assert_eq!(
            actions(&advanced.policy)?,
            provider
                .iam_permissions(&method)?
                .into_iter()
                .map(str::to_owned)
                .collect()
        );
        let hcl = provider.review_permissions_policy_hcl(std::slice::from_ref(&method), true)?;
        assert_eq!(
            hcl.conditional_permissions,
            advanced.conditional_permissions
        );
        for statement in advanced.policy["Statement"]
            .as_array()
            .ok_or("missing statements")?
        {
            if statement["Action"]
                .as_array()
                .ok_or("missing actions")?
                .iter()
                .any(|a| a == "iam:PassRole")
            {
                assert_eq!(
                    statement["Condition"]["StringEquals"]["iam:PassedToService"],
                    "appstream.amazonaws.com"
                );
                assert!(hcl.policy.contains("iam:PassedToService"));
                assert!(hcl.policy.contains("appstream.amazonaws.com"));
            }
        }
        let ordinary_hcl = provider.review_permissions_policy_hcl(&[method], false)?;
        assert!(!ordinary_hcl.policy.contains("iam:PassRole"));
        assert!(!ordinary_hcl.policy.contains("appstream:TagResource"));
    }
    Ok(())
}

#[test]
fn exact_conditional_links_keep_evidence_without_changing_other_grants_or_fas()
-> Result<(), Box<dyn Error>> {
    let provider = AwsProvider::new();
    for (service, name, optional) in [
        ("dynamodb", "Scan", vec!["dynamodb:ReadDataForReplication"]),
        (
            "lambda",
            "CreateFunction",
            vec![
                "lambda:AssociateRuntimeAlias",
                "lambda:AssociateRuntimeVersion",
            ],
        ),
        (
            "lambda",
            "UpdateFunctionConfiguration",
            vec![
                "lambda:AssociateRuntimeAlias",
                "lambda:AssociateRuntimeVersion",
            ],
        ),
        (
            "secretsmanager",
            "CreateSecret",
            vec!["secretsmanager:PutResourcePolicy"],
        ),
    ] {
        let method = ApiMethod::new(service, name);
        let evidence: BTreeSet<_> = provider
            .iam_permissions(&method)?
            .into_iter()
            .map(str::to_owned)
            .collect();
        let ordinary = provider.review_permissions_policy(std::slice::from_ref(&method), false)?;
        let advanced = provider.review_permissions_policy(std::slice::from_ref(&method), true)?;
        let actual_optional: BTreeSet<_> = optional
            .into_iter()
            .filter(|a| evidence.contains(*a))
            .map(str::to_owned)
            .collect();
        assert_eq!(actions(&advanced.policy)?, evidence);
        assert_eq!(
            actions(&ordinary.policy)?,
            evidence.difference(&actual_optional).cloned().collect()
        );
        assert_eq!(
            ordinary
                .conditional_permissions
                .iter()
                .map(|d| d.permission.to_owned())
                .collect::<BTreeSet<_>>(),
            actual_optional
        );
        for diagnostic in &ordinary.conditional_permissions {
            assert_ne!(diagnostic.applicability, "");
            assert!(
                diagnostic
                    .source
                    .starts_with("https://docs.aws.amazon.com/")
            );
        }
        let hcl = provider.review_permissions_policy_hcl(&[method], false)?;
        for permission in actual_optional {
            assert!(!hcl.policy.contains(&permission));
        }
    }
    for (service, name, permission) in [
        ("appstream", "TagResource", "appstream:TagResource"),
        (
            "secretsmanager",
            "PutResourcePolicy",
            "secretsmanager:PutResourcePolicy",
        ),
    ] {
        let method = ApiMethod::new(service, name);
        assert!(actions(&provider.permissions_policy(&[method])?)?.contains(permission));
    }
    let mixed = provider.review_permissions_policy(
        &[
            ApiMethod::new("appstream", "CreateFleet"),
            ApiMethod::new("appstream", "TagResource"),
        ],
        false,
    )?;
    assert!(actions(&mixed.policy)?.contains("appstream:TagResource"));
    assert!(matches!(
        provider.review_permissions_policy(&[ApiMethod::new("unknown", "NoSuchOperation")], false),
        Err(AwsError::UnknownApiMethod { .. })
    ));
    assert!(matches!(
        provider.review_permissions_policy(&[ApiMethod::new("s3", "RenameObject")], false),
        Err(AwsError::UnresolvedAuthorization { .. })
    ));
    Ok(())
}

#[test]
fn unrelated_mapped_to_empty_rows_still_fail_closed() -> Result<(), Box<dyn Error>> {
    use cloudcover_aws::AuthorizationCoverage;
    let catalog: serde_json::Value = serde_json::from_slice(&std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/sar/catalog.json"),
    )?)?;
    let provider = AwsProvider::new();
    for (service, operation) in [
        ("cloudwatch", "CreateAlert"),
        ("inspector", "RegisterCrossAccountAccessRole"),
        ("odb", "DisassociateVirtualMachinesFromExadbVmCluster"),
    ] {
        let document = catalog["services"]
            .as_array()
            .ok_or("missing services")?
            .iter()
            .find(|entry| entry["service"] == service)
            .ok_or("missing service")?;
        let row = document["document"]["Operations"]
            .as_array()
            .ok_or("missing operations")?
            .iter()
            .find(|row| row["Name"] == operation)
            .ok_or("missing operation")?;
        let method = ApiMethod::new(service, operation);
        if row["AuthorizedActions"]
            .as_array()
            .is_none_or(Vec::is_empty)
        {
            assert_eq!(
                provider.authorization_coverage(&method)?,
                AuthorizationCoverage::Unresolved
            );
            assert!(matches!(
                provider.review_permissions_policy(std::slice::from_ref(&method), false),
                Err(AwsError::UnresolvedAuthorization { .. })
            ));
            assert!(matches!(
                provider.review_permissions_policy_hcl(&[method], true),
                Err(AwsError::UnresolvedAuthorization { .. })
            ));
        } else {
            assert_eq!(
                provider.authorization_coverage(&method)?,
                AuthorizationCoverage::Mapped
            );
            assert!(!actions(&provider.permissions_policy(&[method])?)?.is_empty());
        }
    }
    Ok(())
}
