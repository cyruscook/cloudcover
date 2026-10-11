use std::error::Error;

use cloudcover_aws::{AwsProvider, fas::FasCoverage};
use cloudcover_core::{ApiMethod, CloudProvider};
use serde_json::{Value, json};

fn method(service: &str, operation: &str) -> ApiMethod {
    ApiMethod::new(service, operation)
}

#[test]
fn separates_direct_kms_from_fas_and_keeps_copy_keys_distinct() -> Result<(), Box<dyn Error>> {
    let provider = AwsProvider::new();
    let methods = [method("s3", "CopyObject"), method("kms", "Decrypt")];
    let policy = provider.permissions_policy(&methods)?;
    let statements = policy["Statement"].as_array().ok_or("missing statements")?;
    let direct = statements
        .iter()
        .find(|s| s["Resource"] == "arn:${Partition}:kms:${Region}:${Account}:key/${KeyId}")
        .ok_or("missing direct KMS grant")?;
    assert_eq!(direct["Action"], json!(["kms:Decrypt"]));
    assert!(direct.get("Condition").is_none());
    let source = statements
        .iter()
        .find(|s| s["Resource"] == "arn:${Partition}:kms:${Region}:${Account}:key/${SourceKeyId}")
        .ok_or("missing source key")?;
    assert_eq!(source["Action"], json!(["kms:Decrypt"]));
    assert_eq!(
        source["Condition"]["StringEquals"]["kms:ViaService"],
        "s3.${Region}.amazonaws.com"
    );
    assert!(
        statements.iter().any(|s| s["Resource"]
            == "arn:${Partition}:kms:${Region}:${Account}:key/${DestinationKeyId}")
    );
    let mut reversed = methods.to_vec();
    reversed.reverse();
    reversed.push(method("s3", "CopyObject"));
    assert_eq!(policy, provider.permissions_policy(&reversed)?);
    Ok(())
}

#[test]
fn preserves_s3_option_predicates_and_multipart_stages() -> Result<(), Box<dyn Error>> {
    let provider = AwsProvider::new();
    let head = provider.fas_dependencies(&method("s3", "HeadObject"))?;
    assert!(
        head.iter()
            .all(|d| d.rule.applicability.contains("ChecksumMode=ENABLED"))
    );
    let general = head
        .iter()
        .find(|d| d.rule.id == "s3-headobject")
        .ok_or("missing general HEAD")?;
    assert_eq!(general.iam_permissions, ["kms:Decrypt"]);
    let directory = head
        .iter()
        .find(|d| d.rule.id == "s3-directory-headobject")
        .ok_or("missing directory HEAD")?;
    assert_eq!(
        directory.iam_permissions,
        ["kms:Decrypt", "kms:GenerateDataKey"]
    );
    let part = provider.fas_dependencies(&method("s3", "UploadPart"))?;
    assert_eq!(
        part.iter()
            .find(|d| d.rule.id == "s3-uploadpart")
            .ok_or("missing part")?
            .iam_permissions,
        ["kms:Decrypt"]
    );
    let initiation = provider.fas_dependencies(&method("s3", "CreateMultipartUpload"))?;
    assert_eq!(
        initiation
            .iter()
            .find(|d| d.rule.id == "s3-createmultipartupload")
            .ok_or("missing initiation")?
            .iam_permissions,
        ["kms:GenerateDataKey"]
    );
    Ok(())
}

#[test]
fn secrets_manager_validation_key_changes_and_replication_preserve_roles()
-> Result<(), Box<dyn Error>> {
    let provider = AwsProvider::new();
    let create = provider.fas_dependencies(&method("secretsmanager", "CreateSecret"))?;
    assert!(
        create
            .iter()
            .any(|d| d.rule.applicability.contains("without a secret value"))
    );
    let update = provider.permissions_policy(&[method("secretsmanager", "UpdateSecret")])?;
    let statements = update["Statement"].as_array().ok_or("missing statements")?;
    assert!(statements.iter().any(|s| s["Resource"]
        == "arn:${Partition}:kms:${Region}:${Account}:key/${OldKeyId}"
        && s["Action"] == json!(["kms:Decrypt"])));
    assert!(statements.iter().any(|s| {
        s["Resource"] == "arn:${Partition}:kms:${Region}:${Account}:key/${NewKeyId}"
            && s["Action"]
                .as_array()
                .is_some_and(|a| a.contains(&json!("kms:Encrypt")))
    }));
    let replication =
        provider.permissions_policy(&[method("secretsmanager", "ReplicateSecretToRegions")])?;
    let statements = replication["Statement"]
        .as_array()
        .ok_or("missing statements")?;
    let replica = statements
        .iter()
        .find(|s| {
            s["Resource"] == "arn:${Partition}:kms:${ReplicaRegion}:${Account}:key/${ReplicaKeyId}"
        })
        .ok_or("missing replica key")?;
    assert_eq!(
        replica["Condition"]["StringEquals"]["kms:ViaService"],
        "secretsmanager.${ReplicaRegion}.amazonaws.com"
    );
    assert!(
        replica["Action"]
            .as_array()
            .ok_or("missing replica actions")?
            .contains(&json!("kms:Encrypt"))
    );
    assert!(
        !provider
            .iam_permissions(&method("secretsmanager", "GetSecretValue"))?
            .contains(&"kms:DescribeKey")
    );
    Ok(())
}

#[test]
fn follows_secret_reference_chain_with_parent_provenance() -> Result<(), Box<dyn Error>> {
    let provider = AwsProvider::new();
    let dependencies = provider.fas_dependencies(&method("ssm", "GetParameter"))?;
    let chain = dependencies
        .iter()
        .find(|d| d.rule.service == "secretsmanager")
        .ok_or("missing transitive secret decryption")?;
    assert_eq!(chain.path, ["ssm-secrets-reference", "secrets-read"]);
    assert_eq!(chain.iam_permissions, ["kms:Decrypt"]);
    assert_eq!(
        chain.rule.via_service.as_deref(),
        Some("secretsmanager.${Region}.amazonaws.com")
    );
    assert!(
        provider
            .iam_permissions(&method("ssm", "GetParameters"))?
            .contains(&"secretsmanager:GetSecretValue")
    );
    assert!(
        !provider
            .iam_permissions(&method("ssm", "GetParametersByPath"))?
            .contains(&"secretsmanager:GetSecretValue")
    );
    Ok(())
}

#[test]
fn keeps_non_kms_authentication_and_grant_dependencies() -> Result<(), Box<dyn Error>> {
    let provider = AwsProvider::new();
    let permissions = provider.iam_permissions(&method("redshift-data", "ExecuteStatement"))?;
    for action in [
        "redshift:GetClusterCredentials",
        "redshift:GetClusterCredentialsWithIAM",
        "redshift-serverless:GetCredentials",
        "secretsmanager:GetSecretValue",
        "kms:Decrypt",
        "kms:CreateGrant",
    ] {
        assert!(permissions.contains(&action), "missing {action}");
    }
    assert!(
        provider
            .iam_permissions(&method("ecr", "CreateRepository"))?
            .contains(&"kms:CreateGrant")
    );
    assert!(
        provider
            .iam_permissions(&method("ecr", "DeleteRepository"))?
            .contains(&"kms:RetireGrant")
    );
    Ok(())
}

#[test]
fn coverage_distinguishes_unknown_and_reviewed_exclusions() -> Result<(), Box<dyn Error>> {
    let provider = AwsProvider::new();
    for (service, operation) in [
        ("s3", "DeleteObject"),
        ("secretsmanager", "DescribeSecret"),
        ("ecr", "PutImage"),
        ("states", "StartExecution"),
    ] {
        assert_eq!(
            provider.fas_coverage(&method(service, operation))?,
            FasCoverage::ReviewedNoDependencies
        );
        assert!(
            provider
                .fas_dependencies(&method(service, operation))?
                .is_empty()
        );
        assert!(
            provider
                .iam_permissions(&method(service, operation))?
                .iter()
                .all(|a| !a.starts_with("kms:"))
        );
    }
    assert_eq!(
        provider.fas_coverage(&method("ec2", "DescribeInstances"))?,
        FasCoverage::Unknown
    );
    assert_eq!(
        provider.fas_coverage(&method("s3", "GetObject"))?,
        FasCoverage::DocumentedDependencies
    );
    assert!(provider.fas_coverage(&method("invalid", "Nope")).is_err());
    Ok(())
}

fn hcl_value<T: serde::de::DeserializeOwned>(
    attributes: &[hcl::Attribute],
    name: &str,
) -> Result<T, Box<dyn Error>> {
    let attribute = attributes
        .iter()
        .find(|a| a.key.as_str() == name)
        .ok_or("missing HCL attribute")?;
    Ok(serde_json::from_value(serde_json::to_value(
        hcl::Value::from(attribute.expr.clone()),
    )?)?)
}

#[test]
fn json_and_hcl_keep_equivalent_fas_grants() -> Result<(), Box<dyn Error>> {
    let provider = AwsProvider::new();
    let methods = [
        method("s3", "CopyObject"),
        method("kms", "Decrypt"),
        method("dynamodb", "GetItem"),
        method("secretsmanager", "ReplicateSecretToRegions"),
    ];
    let json_policy = provider.permissions_policy(&methods)?;
    let body: hcl::Body = hcl::from_str(&provider.permissions_policy_hcl(&methods)?)?;
    let data = body.blocks().next().ok_or("missing data block")?;
    let mut converted = Vec::new();
    for block in data.body.blocks() {
        let attributes = block.body.attributes().cloned().collect::<Vec<_>>();
        let actions: Vec<String> = hcl_value(&attributes, "actions")?;
        let resources: Vec<String> = hcl_value(&attributes, "resources")?;
        let resource = match resources.as_slice() {
            [r] => json!(r),
            _ => json!(resources),
        };
        let mut statement = json!({"Effect":"Allow", "Action":actions, "Resource":resource});
        for condition in block.body.blocks() {
            let attrs = condition.body.attributes().cloned().collect::<Vec<_>>();
            let test: String = hcl_value(&attrs, "test")?;
            let variable: String = hcl_value(&attrs, "variable")?;
            let values: Vec<String> = hcl_value(&attrs, "values")?;
            statement["Condition"] =
                json!({test: {variable: values.first().ok_or("empty condition")?}});
        }
        converted.push(statement);
    }
    assert_eq!(json_policy["Statement"], Value::Array(converted));
    Ok(())
}

#[test]
fn sqs_redrive_separates_old_message_and_destination_keys() -> Result<(), Box<dyn Error>> {
    let provider = AwsProvider::new();
    let policy = provider.permissions_policy(&[method("sqs", "StartMessageMoveTask")])?;
    let statements = policy["Statement"].as_array().ok_or("missing statements")?;
    let destination = statements
        .iter()
        .find(|s| {
            s["Resource"]
                == "arn:${Partition}:kms:${Region}:${Account}:key/${DestinationQueueKeyId}"
        })
        .ok_or("missing destination key")?;
    assert_eq!(
        destination["Action"],
        json!(["kms:Decrypt", "kms:GenerateDataKey"])
    );
    let original = statements
        .iter()
        .find(|s| {
            s["Resource"] == "arn:${Partition}:kms:${Region}:${Account}:key/${OriginalQueueKeyId}"
        })
        .ok_or("missing original message key")?;
    assert_eq!(original["Action"], json!(["kms:Decrypt"]));
    let cancel = provider.iam_permissions(&method("sqs", "CancelMessageMoveTask"))?;
    assert!(cancel.contains(&"kms:Decrypt"));
    assert!(!cancel.contains(&"kms:GenerateDataKey"));
    Ok(())
}
