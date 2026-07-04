use std::{error::Error, io};

use cloudcover_aws::{AwsError, AwsProvider};
use cloudcover_core::{
    ApiMethod, CloudProvider, GoMethodReference, Language, MethodReference, PythonMethodReference,
    Sdk, SdkMethodMapping, TerraformMethodReference,
};
use serde_json::json;

#[test]
fn lists_known_aws_methods() {
    let methods = AwsProvider::new().list_api_methods();

    assert!(methods.contains(&ApiMethod::new("s3", "GetObject")));
    assert!(methods.contains(&ApiMethod::new("ec2", "DescribeInstances")));

    let mut sorted = methods.clone();
    sorted.sort();
    assert_eq!(methods, sorted);
    assert!(methods.windows(2).all(|window| window[0] != window[1]));
}

#[test]
fn lists_supported_aws_sdks() {
    assert_eq!(
        AwsProvider::new().list_sdks(),
        vec![
            Sdk::new("aws-sdk-go-v2", Language::Go),
            Sdk::new("boto3", Language::Python),
            Sdk::new("terraform-provider-aws", Language::Terraform),
        ]
    );
}

#[test]
fn maps_sdk_methods_to_api_methods() {
    let provider = AwsProvider::new();
    let mappings = provider.sdk_method_mappings();

    assert!(mappings.contains(&SdkMethodMapping::new(
        Sdk::new("boto3", Language::Python),
        MethodReference::Python(PythonMethodReference::new(
            "boto3",
            Some("s3".to_owned()),
            "get_object",
        )),
        vec![ApiMethod::new("s3", "GetObject")],
    )));
    assert!(mappings.contains(&SdkMethodMapping::new(
        Sdk::new("boto3", Language::Python),
        MethodReference::Python(PythonMethodReference::new(
            "boto3",
            Some("ec2".to_owned()),
            "describe_instances",
        )),
        vec![ApiMethod::new("ec2", "DescribeInstances")],
    )));
    assert!(mappings.contains(&SdkMethodMapping::new(
        Sdk::new("aws-sdk-go-v2", Language::Go),
        MethodReference::Go(GoMethodReference::new(
            "github.com/aws/aws-sdk-go-v2/service/s3",
            Some("Client".to_owned()),
            "GetObject",
        )),
        vec![ApiMethod::new("s3", "GetObject")],
    )));
    assert!(mappings.contains(&SdkMethodMapping::new(
        Sdk::new("aws-sdk-go-v2", Language::Go),
        MethodReference::Go(GoMethodReference::new(
            "github.com/aws/aws-sdk-go-v2/service/ec2",
            Some("Client".to_owned()),
            "DescribeInstances",
        )),
        vec![ApiMethod::new("ec2", "DescribeInstances")],
    )));
    let terraform_bucket_create_matches: Vec<_> = mappings
        .iter()
        .filter(|mapping| {
            mapping.sdk() == &Sdk::new("terraform-provider-aws", Language::Terraform)
                && mapping.method()
                    == &MethodReference::Terraform(TerraformMethodReference::new(
                        "resource",
                        "aws_s3_bucket",
                        "create",
                    ))
        })
        .collect();
    assert_eq!(
        terraform_bucket_create_matches.len(),
        1,
        "missing terraform-provider-aws aws_s3_bucket create mapping"
    );
    let terraform_bucket_create = terraform_bucket_create_matches[0];
    assert!(terraform_bucket_create
        .api_methods()
        .contains(&ApiMethod::new("s3", "CreateBucket")));


    let api_methods = provider.list_api_methods();
    for mapping in mappings.iter().filter(|mapping| {
        matches!(
            mapping.sdk().name(),
            "aws-sdk-go-v2" | "terraform-provider-aws"
        )
    }) {
        for api_method in mapping.api_methods() {
            assert!(api_methods.contains(api_method));
        }
    }

    let mut sorted = mappings.clone();
    sorted.sort();
    assert_eq!(mappings, sorted);
    assert!(mappings.windows(2).all(|window| window[0] != window[1]));
}

#[test]
fn builds_deterministic_iam_allow_policy() -> Result<(), Box<dyn Error>> {
    let policy = AwsProvider::new().permissions_policy(&[
        ApiMethod::new("s3", "PutObject"),
        ApiMethod::new("s3", "GetObject"),
        ApiMethod::new("ec2", "DescribeInstances"),
        ApiMethod::new("s3", "GetObject"),
    ])?;

    assert_eq!(
        policy,
        json!({
            "Version":"2012-10-17",
            "Statement":[
                {
                    "Effect":"Allow",
                    "Action":["ec2:DescribeInstances"],
                    "Resource":"*"
                },
                {
                    "Effect":"Allow",
                    "Action":[
                        "s3:GetObject",
                        "s3:GetObjectLegalHold",
                        "s3:GetObjectRetention",
                        "s3:GetObjectTagging",
                        "s3:GetObjectVersion",
                        "s3:PutObject",
                        "s3:PutObjectAcl",
                        "s3:PutObjectLegalHold",
                        "s3:PutObjectRetention",
                        "s3:PutObjectTagging"
                    ],
                    "Resource":[
                        "arn:${Partition}:s3:${Region}:${Account}:accesspoint/${AccessPointName}/object/${ObjectName}",
                        "arn:${Partition}:s3:::${BucketName}/${ObjectName}"
                    ]
                },
                {
                    "Effect":"Allow",
                    "Action":["s3-object-lambda:GetObject","s3-object-lambda:PutObject"],
                    "Resource":"arn:${Partition}:s3-object-lambda:${Region}:${Account}:accesspoint/${AccessPointName}"
                }
            ]
        })
    );

    Ok(())
}

#[test]
fn splits_actions_by_service_and_resource_scope() -> Result<(), Box<dyn Error>> {
    let policy = AwsProvider::new().permissions_policy(&[
        ApiMethod::new("iam", "AttachUserPolicy"),
        ApiMethod::new("iam", "CreateAccountAlias"),
        ApiMethod::new("iam", "AttachRolePolicy"),
    ])?;

    assert_eq!(
        policy,
        json!({
            "Version":"2012-10-17",
            "Statement":[
                {
                    "Effect":"Allow",
                    "Action":["iam:CreateAccountAlias"],
                    "Resource":"*"
                },
                {
                    "Effect":"Allow",
                    "Action":["iam:AttachRolePolicy"],
                    "Resource":"arn:${Partition}:iam::${Account}:role/${RoleNameWithPath}"
                },
                {
                    "Effect":"Allow",
                    "Action":["iam:AttachUserPolicy"],
                    "Resource":"arn:${Partition}:iam::${Account}:user/${UserNameWithPath}"
                }
            ]
        })
    );

    Ok(())
}

#[test]
fn operation_permissions_use_authorized_actions() -> Result<(), Box<dyn Error>> {
    let policy = AwsProvider::new().permissions_policy(&[ApiMethod::new("s3", "CopyObject")])?;
    let statements = policy
        .get("Statement")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| io::Error::other("missing policy statements"))?;
    let mut actions = statements
        .iter()
        .filter_map(|statement| statement.get("Action"))
        .flat_map(|action| action.as_array().into_iter().flatten())
        .filter_map(serde_json::Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();

    actions.sort();
    actions.dedup();

    assert_eq!(
        actions,
        vec![
            "s3-object-lambda:PutObject".to_owned(),
            "s3:GetObject".to_owned(),
            "s3:GetObjectVersion".to_owned(),
            "s3:PutObject".to_owned(),
            "s3:PutObjectAcl".to_owned(),
            "s3:PutObjectLegalHold".to_owned(),
            "s3:PutObjectRetention".to_owned(),
            "s3:PutObjectTagging".to_owned(),
        ]
    );

    Ok(())
}

#[test]
fn operation_with_no_authorized_actions_builds_no_policy_statement() -> Result<(), Box<dyn Error>> {
    let policy = AwsProvider::new().permissions_policy(&[ApiMethod::new("s3", "CreateSession")])?;

    assert_eq!(policy, json!({"Version":"2012-10-17","Statement":[]}));

    Ok(())
}

#[test]
fn empty_method_set_builds_empty_policy() -> Result<(), Box<dyn Error>> {
    let policy = AwsProvider::new().permissions_policy(&[])?;

    assert_eq!(policy, json!({"Version":"2012-10-17","Statement":[]}));

    Ok(())
}

#[test]
fn rejects_unknown_api_method() -> Result<(), Box<dyn Error>> {
    let result = AwsProvider::new().permissions_policy(&[ApiMethod::new("not-a-service", "Nope")]);

    let Err(AwsError::UnknownApiMethod { service, name }) = result else {
        return Err(io::Error::other("expected unknown API method error").into());
    };

    assert_eq!(service, "not-a-service");
    assert_eq!(name, "Nope");

    Ok(())
}
