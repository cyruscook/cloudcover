use std::{error::Error, io};

use cloudcover_aws::{AwsError, AwsProvider};
use cloudcover_core::{
    ApiMethod, CloudProvider, GoMethodReference, Language, MethodReference, PythonMethodReference,
    ResolvedSdk, Sdk, SdkMethodMapping, SdkModule, SdkModuleReplacement, TerraformMethodReference,
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
    let sdks = AwsProvider::new().list_sdks();
    assert_eq!(sdks.first(), Some(&Sdk::new("aws-sdk-go-v2", Language::Go)));
    assert_eq!(sdks[1], Sdk::new("aws-sdk-go-v1", Language::Go));
    assert_eq!(sdks[2], Sdk::new("boto3", Language::Python));
    let terraform_sdks = &sdks[3..];
    assert_eq!(terraform_sdks.len(), 516);
    for version in ["0.1.0", "6.63.0", "6.64.0"] {
        assert!(terraform_sdks.contains(
            &Sdk::new("terraform-provider-aws", Language::Terraform).with_version(version)
        ));
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn maps_sdk_methods_to_api_methods() -> Result<(), Box<dyn Error>> {
    let provider = AwsProvider::new();
    let python_sdk = Sdk::new("boto3", Language::Python);
    let go_sdk = Sdk::new("aws-sdk-go-v2", Language::Go);
    let go_v1_sdk = Sdk::new("aws-sdk-go-v1", Language::Go);
    let terraform_sdk =
        Sdk::new("terraform-provider-aws", Language::Terraform).with_version("6.64.0");
    let python_mappings = provider.sdk_method_mappings(&ResolvedSdk::new(python_sdk.clone()))?;
    let go_mappings =
        provider.sdk_method_mappings(&ResolvedSdk::new(go_sdk.clone()).with_modules([
            SdkModule::new("github.com/aws/aws-sdk-go-v2/service/s3", "v1.104.0"),
        ]))?;
    let go_v1_mappings = provider.sdk_method_mappings(
        &ResolvedSdk::new(go_v1_sdk.clone())
            .with_modules([SdkModule::new("github.com/aws/aws-sdk-go", "v1.55.8")]),
    )?;
    let go_paginator_mappings =
        provider.sdk_method_mappings(&ResolvedSdk::new(go_sdk.clone()).with_modules([
            SdkModule::new("github.com/aws/aws-sdk-go-v2/service/ec2", "v1.335.0"),
        ]))?;
    let terraform_mappings =
        provider.sdk_method_mappings(&ResolvedSdk::new(terraform_sdk.clone()))?;

    assert!(python_mappings.contains(&SdkMethodMapping::new(
        python_sdk.clone(),
        MethodReference::Python(PythonMethodReference::new(
            "boto3",
            Some("s3".to_owned()),
            "get_object",
        )),
        vec![ApiMethod::new("s3", "GetObject")],
    )));
    assert!(python_mappings.contains(&SdkMethodMapping::new(
        python_sdk,
        MethodReference::Python(PythonMethodReference::new(
            "boto3",
            Some("ec2".to_owned()),
            "describe_instances",
        )),
        vec![ApiMethod::new("ec2", "DescribeInstances")],
    )));
    assert!(go_mappings.contains(&SdkMethodMapping::new(
        go_sdk.clone(),
        MethodReference::Go(GoMethodReference::new(
            "github.com/aws/aws-sdk-go-v2/service/s3",
            Some("Client".to_owned()),
            "GetObject",
        )),
        vec![ApiMethod::new("s3", "GetObject")],
    )));
    assert!(go_paginator_mappings.contains(&SdkMethodMapping::new(
        go_sdk,
        MethodReference::Go(GoMethodReference::new(
            "github.com/aws/aws-sdk-go-v2/service/ec2",
            Some("DescribeInstancesPaginator".to_owned()),
            "NextPage",
        )),
        vec![ApiMethod::new("ec2", "DescribeInstances")],
    )));
    assert!(go_v1_mappings.contains(&SdkMethodMapping::new(
        go_v1_sdk,
        MethodReference::Go(GoMethodReference::new(
            "github.com/aws/aws-sdk-go/service/s3",
            Some("S3".to_owned()),
            "GetObject",
        )),
        vec![ApiMethod::new("s3", "GetObject")],
    )));
    let terraform_bucket_create = terraform_mappings
        .iter()
        .find(|mapping| {
            mapping.method()
                == &MethodReference::Terraform(TerraformMethodReference::new(
                    "resource",
                    "aws_s3_bucket",
                    "create",
                ))
        })
        .ok_or_else(|| {
            io::Error::other("missing terraform-provider-aws aws_s3_bucket create mapping")
        })?;
    assert_eq!(terraform_bucket_create.sdk(), &terraform_sdk);
    assert!(
        terraform_bucket_create
            .api_methods()
            .contains(&ApiMethod::new("s3", "CreateBucket"))
    );

    let api_methods = provider.list_api_methods();
    for mapping in go_mappings
        .iter()
        .chain(go_v1_mappings.iter())
        .chain(go_paginator_mappings.iter())
        .chain(terraform_mappings.iter())
    {
        for api_method in mapping.api_methods() {
            assert!(api_methods.contains(api_method));
        }
    }

    for mappings in [
        &python_mappings,
        &go_mappings,
        &go_v1_mappings,
        &go_paginator_mappings,
        &terraform_mappings,
    ] {
        let mut sorted = mappings.clone();
        sorted.sort();
        assert_eq!(*mappings, sorted);
        assert!(mappings.windows(2).all(|window| window[0] != window[1]));
    }
    Ok(())
}

#[test]
fn rejects_unsupported_sdk_mappings() {
    let provider = AwsProvider::new();

    let result = provider.sdk_method_mappings(&ResolvedSdk::new(Sdk::new(
        "terraform-provider-aws",
        Language::Terraform,
    )));
    assert!(matches!(
        result,
        Err(AwsError::MissingSdkVersion { name }) if name == "terraform-provider-aws"
    ));

    for version in ["7.0.0", "6.64", "v6.64.0", "6.64.0+local"] {
        let result = provider.sdk_method_mappings(&ResolvedSdk::new(
            Sdk::new("terraform-provider-aws", Language::Terraform).with_version(version),
        ));
        assert!(matches!(
            result,
            Err(AwsError::UnsupportedSdkVersion { name, version: actual })
                if name == "terraform-provider-aws" && actual == version
        ));
    }

    for version in ["1.48.0", "1.42", "v1.42.0", "1.42.0+local"] {
        let result = provider.sdk_method_mappings(&ResolvedSdk::new(
            Sdk::new("aws-sdk-go-v2", Language::Go).with_version(version),
        ));
        assert!(matches!(
            result,
            Err(AwsError::UnsupportedSdkVersion { name, version: actual })
                if name == "aws-sdk-go-v2" && actual == version
        ));
    }

    let service_path = "github.com/aws/aws-sdk-go-v2/service/s3";
    let pseudo_version = "v1.104.1-0.20260617000000-deadbeefdead";
    let result = provider.sdk_method_mappings(
        &ResolvedSdk::new(Sdk::new("aws-sdk-go-v2", Language::Go))
            .with_modules([SdkModule::new(service_path, pseudo_version)]),
    );
    assert!(matches!(
        result,
        Err(AwsError::UnsupportedSdkModuleVersion { path, version })
            if path == service_path && version == pseudo_version
    ));

    let result =
        provider.sdk_method_mappings(
            &ResolvedSdk::new(Sdk::new("aws-sdk-go-v2", Language::Go))
                .with_modules([SdkModule::new(service_path, "v1.104.0")
                    .with_replacement(SdkModuleReplacement::new("../local-s3", None))]),
        );
    assert!(matches!(
        result,
        Err(AwsError::UnsupportedSdkModuleReplacement {
            path,
            replacement_path,
            replacement_version: None,
        }) if path == service_path && replacement_path == "../local-s3"
    ));

    let result = provider.sdk_method_mappings(&ResolvedSdk::new(Sdk::new(
        "other-terraform-provider",
        Language::Terraform,
    )));
    assert!(matches!(
        result,
        Err(AwsError::UnsupportedSdk { name, language })
            if name == "other-terraform-provider" && language == Language::Terraform
    ));
}

#[test]
fn distinguishes_unsupported_terraform_provider_aws_versions_from_unknown_versions() {
    let provider = AwsProvider::new();

    let legacy_result = provider.sdk_method_mappings(&ResolvedSdk::new(
        Sdk::new("terraform-provider-aws", Language::Terraform).with_version("1.56.0"),
    ));
    assert!(matches!(
        legacy_result,
        Err(AwsError::UnsupportedTerraformProviderAwsVersion { version, reason })
            if version == "1.56.0" && reason.as_str() == "aws_sdk_go_v1"
    ));

    let supported_result = provider.sdk_method_mappings(&ResolvedSdk::new(
        Sdk::new("terraform-provider-aws", Language::Terraform).with_version("1.57.0"),
    ));
    assert!(matches!(supported_result, Ok(mappings) if !mappings.is_empty()));

    for version in ["1.56", "v1.56.0", "1.56.0+local"] {
        let result = provider.sdk_method_mappings(&ResolvedSdk::new(
            Sdk::new("terraform-provider-aws", Language::Terraform).with_version(version),
        ));
        assert!(matches!(
            result,
            Err(AwsError::UnsupportedSdkVersion { name, version: actual })
                if name == "terraform-provider-aws" && actual == version
        ));
    }
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
fn excludes_get_caller_identity_from_mixed_policy() -> Result<(), Box<dyn Error>> {
    let policy = AwsProvider::new().permissions_policy(&[
        ApiMethod::new("sts", "GetCallerIdentity"),
        ApiMethod::new("ec2", "DescribeInstances"),
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
                }
            ]
        })
    );

    Ok(())
}

#[test]
fn excludes_get_caller_identity_from_sole_permission_policy() -> Result<(), Box<dyn Error>> {
    let policy =
        AwsProvider::new().permissions_policy(&[ApiMethod::new("sts", "GetCallerIdentity")])?;

    assert_eq!(policy, json!({"Version":"2012-10-17","Statement":[]}));

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
fn lists_iam_permissions_for_api_operations() -> Result<(), Box<dyn Error>> {
    let permissions = AwsProvider::new().iam_permissions(&ApiMethod::new("s3", "CopyObject"))?;

    assert_eq!(
        permissions,
        vec![
            "s3-object-lambda:PutObject",
            "s3:GetObject",
            "s3:GetObjectVersion",
            "s3:PutObject",
            "s3:PutObjectAcl",
            "s3:PutObjectLegalHold",
            "s3:PutObjectRetention",
            "s3:PutObjectTagging",
        ]
    );

    Ok(())
}

#[test]
fn reports_empty_iam_permissions_for_authorized_actionless_operation() -> Result<(), Box<dyn Error>>
{
    assert_eq!(
        AwsProvider::new().iam_permissions(&ApiMethod::new("s3", "CreateSession"))?,
        Vec::<&'static str>::new()
    );
    Ok(())
}

#[test]
fn rejects_unknown_api_method_for_iam_permissions() {
    assert!(matches!(
        AwsProvider::new().iam_permissions(&ApiMethod::new("not-a-service", "Nope")),
        Err(AwsError::UnknownApiMethod { service, name })
            if service == "not-a-service" && name == "Nope"
    ));
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
