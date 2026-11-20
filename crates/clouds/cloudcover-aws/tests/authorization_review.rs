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
fn appstream_defaults_keep_base_and_role_grants_remain_manual() -> Result<(), Box<dyn Error>> {
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
                .filter(|permission| *permission != "iam:PassRole")
                .map(str::to_owned)
                .collect()
        );
        let hcl = provider.review_permissions_policy_hcl(std::slice::from_ref(&method), true)?;
        assert_eq!(
            hcl.conditional_permissions,
            advanced.conditional_permissions
        );
        assert!(!actions(&advanced.policy)?.contains("iam:PassRole"));
        assert!(!hcl.policy.contains("iam:PassRole"));
        for diagnostic in &advanced.conditional_permissions {
            if diagnostic.permission == "iam:PassRole" {
                assert_eq!(
                    diagnostic.passed_to_service,
                    Some("appstream.amazonaws.com")
                );
                let step = diagnostic.manual_step.ok_or("missing manual role step")?;
                assert!(step.contains("concrete, validated requested role ARN"));
                assert!(step.contains("iam:PassedToService = appstream.amazonaws.com"));
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
fn reviewed_dispositions_block_every_policy_path_without_permissionless_success()
-> Result<(), Box<dyn Error>> {
    let review: serde_json::Value =
        serde_json::from_str(include_str!("../data/authorization-dispositions.json"))?;
    let provider = AwsProvider::new();
    let entries = review["entries"].as_array().ok_or("missing dispositions")?;
    assert_eq!(entries.len(), 22);
    for row in entries {
        let service = row["service"].as_str().ok_or("service")?;
        let operation = row["operation"].as_str().ok_or("operation")?;
        let method = ApiMethod::new(service, operation);
        let diagnostic = provider
            .authorization_disposition(&method)
            .ok_or("missing diagnostic")?;
        assert_eq!(diagnostic.kind, row["kind"].as_str().ok_or("kind")?);
        assert_ne!(diagnostic.source, "");
        for include in [false, true] {
            for methods in [
                vec![method.clone()],
                vec![ApiMethod::new("s3", "GetObject"), method.clone()],
            ] {
                for result in [
                    provider
                        .review_permissions_policy(&methods, include)
                        .map(|_| ()),
                    provider
                        .review_permissions_policy_hcl(&methods, include)
                        .map(|_| ()),
                ] {
                    assert!(
                        matches!(result, Err(AwsError::AuthorizationDisposition { diagnostic: d })
                        if d == diagnostic)
                    );
                }
            }
        }
        assert!(matches!(
            provider.permissions_policy(std::slice::from_ref(&method)),
            Err(AwsError::AuthorizationDisposition { .. })
        ));
        assert!(matches!(
            provider.permissions_policy_hcl(&[method]),
            Err(AwsError::AuthorizationDisposition { .. })
        ));
        if service == "network-firewall" {
            assert!(
                diagnostic
                    .message
                    .contains("not evidence of endpoint shutdown")
            );
            assert!(diagnostic.message.contains("v1.80.0"));
        }
        if service == "cloudwatch" {
            assert!(diagnostic.message.contains("HasAccessGrant=true"));
            assert!(diagnostic.message.contains("space/profile"));
        }
    }
    Ok(())
}

#[test]
fn odb_bridge_is_single_action_and_scoped_in_both_formats() -> Result<(), Box<dyn Error>> {
    use cloudcover_aws::AuthorizationCoverage;
    let provider = AwsProvider::new();
    let method = ApiMethod::new("odb", "DisassociateVirtualMachinesFromExadbVmCluster");
    let action = "odb:DisassociateVirtualMachinesFromExadbVmCluster";
    let arn = "arn:${Partition}:odb:${Region}:${Account}:exadb-vm-cluster/${ExadbVmClusterId}";
    assert_eq!(
        provider.authorization_coverage(&method)?,
        AuthorizationCoverage::Mapped
    );
    assert!(provider.authorization_disposition(&method).is_none());
    assert_eq!(provider.iam_permissions(&method)?, [action]);
    for include in [false, true] {
        let result = provider.review_permissions_policy(std::slice::from_ref(&method), include)?;
        assert_eq!(result.conditional_permissions, []);
        assert_eq!(
            result.policy["Statement"],
            serde_json::json!([
                {"Effect":"Allow", "Action":[action], "Resource":arn}
            ])
        );
        let hcl = provider.review_permissions_policy_hcl(std::slice::from_ref(&method), include)?;
        assert_eq!(hcl.conditional_permissions, []);
        assert!(hcl.policy.contains(action));
        assert!(hcl.policy.contains(&arn.replace("${", "$${")));
        assert!(!hcl.policy.contains("\"*\""));
        assert!(!hcl.policy.contains("PassRole"));
    }
    assert_eq!(
        provider.permissions_policy(std::slice::from_ref(&method))?["Statement"][0]["Resource"],
        arn
    );
    assert!(
        provider
            .permissions_policy_hcl(&[method])?
            .contains(&arn.replace("${", "$${"))
    );
    Ok(())
}

#[test]
fn retained_proxy_apis_and_unknown_operations_are_not_legacy_exceptions()
-> Result<(), Box<dyn Error>> {
    let provider = AwsProvider::new();
    for name in ["DeleteProxy", "DescribeProxy"] {
        let method = ApiMethod::new("network-firewall", name);
        assert!(provider.authorization_disposition(&method).is_none());
        let json = provider.permissions_policy(std::slice::from_ref(&method))?;
        assert!(actions(&json)?.contains(&format!("network-firewall:{name}")));
        assert!(
            provider
                .permissions_policy_hcl(&[method])?
                .contains(&format!("network-firewall:{name}"))
        );
    }
    // Retained in both models, but neither raw snapshot establishes its IAM
    // mapping. Do not mislabel it as deprecated or invent a same-name grant.
    let retained_unresolved = ApiMethod::new("network-firewall", "UpdateProxySettings");
    assert!(
        provider
            .authorization_disposition(&retained_unresolved)
            .is_none()
    );
    assert!(matches!(
        provider.permissions_policy(std::slice::from_ref(&retained_unresolved)),
        Err(AwsError::UnresolvedAuthorization { .. })
    ));
    assert!(matches!(
        provider.permissions_policy_hcl(&[retained_unresolved]),
        Err(AwsError::UnresolvedAuthorization { .. })
    ));
    for (service, name) in [
        ("network-firewall", "UnreviewedProxyOperation"),
        ("other", "CreateAlert"),
        ("inspector2", "RegisterCrossAccountAccessRole"),
    ] {
        let method = ApiMethod::new(service, name);
        assert!(provider.authorization_disposition(&method).is_none());
        assert!(matches!(
            provider.permissions_policy(std::slice::from_ref(&method)),
            Err(AwsError::UnknownApiMethod { .. })
        ));
        assert!(matches!(
            provider.permissions_policy_hcl(&[method]),
            Err(AwsError::UnknownApiMethod { .. })
        ));
    }
    Ok(())
}
