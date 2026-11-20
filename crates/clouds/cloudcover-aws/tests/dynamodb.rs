use std::error::Error;

use cloudcover_aws::{AwsError, AwsProvider};
use cloudcover_core::{
    ApiMethod, CloudProvider, JavaScriptMethodReference, Language, MethodReference, ResolvedSdk,
    Sdk, SdkModule,
};

const DOCUMENT: &str = "@aws-sdk/lib-dynamodb";
const LOW_LEVEL: &str = "@aws-sdk/client-dynamodb";

#[test]
fn batch_transaction_and_partiql_actions_use_dynamodb_authorization_names()
-> Result<(), Box<dyn Error>> {
    let provider = AwsProvider::new();
    for (operation, expected) in [
        ("BatchGetItem", vec!["dynamodb:BatchGetItem"]),
        ("BatchWriteItem", vec!["dynamodb:BatchWriteItem"]),
        ("TransactGetItems", vec!["dynamodb:GetItem"]),
        (
            "TransactWriteItems",
            vec![
                "dynamodb:ConditionCheckItem",
                "dynamodb:DeleteItem",
                "dynamodb:PutItem",
                "dynamodb:UpdateItem",
            ],
        ),
        (
            "ExecuteStatement",
            vec![
                "dynamodb:PartiQLDelete",
                "dynamodb:PartiQLInsert",
                "dynamodb:PartiQLSelect",
                "dynamodb:PartiQLUpdate",
            ],
        ),
        (
            "BatchExecuteStatement",
            vec![
                "dynamodb:PartiQLDelete",
                "dynamodb:PartiQLInsert",
                "dynamodb:PartiQLSelect",
                "dynamodb:PartiQLUpdate",
            ],
        ),
        (
            "ExecuteTransaction",
            vec![
                "dynamodb:PartiQLDelete",
                "dynamodb:PartiQLInsert",
                "dynamodb:PartiQLSelect",
                "dynamodb:PartiQLUpdate",
            ],
        ),
    ] {
        let actions = provider.iam_permissions(&ApiMethod::new("dynamodb", operation))?;
        assert_eq!(
            actions
                .into_iter()
                .filter(|action| action.starts_with("dynamodb:"))
                .collect::<Vec<_>>(),
            expected
        );
    }
    Ok(())
}

#[test]
fn document_operations_reuse_low_level_dynamodb_iam_semantics() -> Result<(), Box<dyn Error>> {
    let provider = AwsProvider::new();
    let sdk = ResolvedSdk::new(Sdk::new("aws-sdk-js-v3", Language::TypeScript)).with_modules([
        SdkModule::new(DOCUMENT, "3.1137.0"),
        SdkModule::new(LOW_LEVEL, "3.1137.0"),
    ]);
    let mappings = provider.sdk_method_mappings(&sdk)?;
    for (wrapper, operation) in [
        ("Get", "GetItem"),
        ("Put", "PutItem"),
        ("Delete", "DeleteItem"),
        ("Update", "UpdateItem"),
        ("Query", "Query"),
        ("Scan", "Scan"),
        ("BatchGet", "BatchGetItem"),
        ("BatchWrite", "BatchWriteItem"),
        ("TransactGet", "TransactGetItems"),
        ("TransactWrite", "TransactWriteItems"),
        ("ExecuteStatement", "ExecuteStatement"),
        ("BatchExecuteStatement", "BatchExecuteStatement"),
        ("ExecuteTransaction", "ExecuteTransaction"),
    ] {
        let method = format!("{wrapper}Command");
        let aggregate = format!(
            "{}{rest}",
            wrapper[..1].to_lowercase(),
            rest = &wrapper[1..]
        );
        let low_level = format!("{operation}Command");
        for (package, receiver, name) in [
            (DOCUMENT, None, method.as_str()),
            (
                DOCUMENT,
                Some("DynamoDBDocument".to_owned()),
                aggregate.as_str(),
            ),
            (LOW_LEVEL, None, low_level.as_str()),
        ] {
            let reference = MethodReference::JavaScript(JavaScriptMethodReference::new(
                package, receiver, name,
            ));
            let mapping = mappings
                .iter()
                .find(|row| row.method() == &reference)
                .ok_or_else(|| format!("missing {reference:?}"))?;
            assert_eq!(
                mapping.api_methods(),
                &[ApiMethod::new("dynamodb", operation)]
            );
            assert_eq!(mapping.unresolved_api_methods(), []);
            // Match both JSON and HCL output, including shared transaction and
            // PartiQL authorization, conditional direct links and possible FAS.
            let expected = [ApiMethod::new("dynamodb", operation)];
            for include in [false, true] {
                let actual_json =
                    provider.review_permissions_policy(mapping.api_methods(), include)?;
                let expected_json = provider.review_permissions_policy(&expected, include)?;
                assert_eq!(actual_json.policy, expected_json.policy);
                assert_eq!(
                    actual_json.conditional_permissions,
                    expected_json.conditional_permissions
                );
                let actual_hcl =
                    provider.review_permissions_policy_hcl(mapping.api_methods(), include)?;
                let expected_hcl = provider.review_permissions_policy_hcl(&expected, include)?;
                assert_eq!(actual_hcl.policy, expected_hcl.policy);
                assert_eq!(
                    actual_hcl.conditional_permissions,
                    expected_hcl.conditional_permissions
                );
                if actual_json.conditional_permissions.is_empty() {
                    assert_eq!(
                        provider.permissions_policy(mapping.api_methods())?,
                        actual_json.policy
                    );
                    assert_eq!(
                        provider.permissions_policy_hcl(mapping.api_methods())?,
                        actual_hcl.policy
                    );
                } else {
                    // Newer SAR Scan evidence requires review. Document wrappers
                    // must preserve the same strict bare-API behavior and report.
                    assert!(matches!(
                        provider.permissions_policy(mapping.api_methods()),
                        Err(AwsError::ConditionalAuthorization { .. })
                    ));
                    assert!(matches!(
                        provider.permissions_policy_hcl(mapping.api_methods()),
                        Err(AwsError::ConditionalAuthorization { .. })
                    ));
                }
            }
        }
    }
    Ok(())
}

#[test]
fn document_versions_and_symbols_remain_exact_and_fail_closed() -> Result<(), Box<dyn Error>> {
    let provider = AwsProvider::new();
    for version in [
        "",
        "3.0.0",
        "3.1137.1",
        "3.1137.0+local",
        "3.1137.0-beta",
        "03.1137.0",
        "999.0.0",
    ] {
        let sdk = ResolvedSdk::new(Sdk::new("aws-sdk-js-v3", Language::JavaScript))
            .with_modules([SdkModule::new(DOCUMENT, version)]);
        assert!(matches!(
            provider.sdk_method_mappings(&sdk),
            Err(AwsError::MissingJsSdkPackageVersion { .. }
                | AwsError::UnsupportedJsSdkPackageVersion { .. })
        ));
    }
    let sdk = ResolvedSdk::new(Sdk::new("aws-sdk-js-v3", Language::JavaScript))
        .with_modules([SdkModule::new(DOCUMENT, "3.8.0")]);
    let rows = provider.sdk_method_mappings(&sdk)?;
    for name in [
        "UnknownCommand",
        "GetItemCommand",
        "paginateQuery",
        "paginateScan",
    ] {
        assert!(!rows.iter().any(|row| row.method()
            == &MethodReference::JavaScript(JavaScriptMethodReference::new(DOCUMENT, None, name))));
    }
    Ok(())
}
