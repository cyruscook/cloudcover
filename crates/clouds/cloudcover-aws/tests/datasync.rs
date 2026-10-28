use std::{collections::BTreeMap, error::Error};

use cloudcover_aws::AwsProvider;
use cloudcover_core::{ApiMethod, CloudProvider};
use serde_json::{Value, json};

type Scopes = BTreeMap<String, Vec<String>>;

fn hcl_attribute<T: serde::de::DeserializeOwned>(
    block: &hcl::Block,
    name: &str,
) -> Result<T, Box<dyn Error>> {
    let attribute = block
        .body
        .attributes()
        .find(|attribute| attribute.key.as_str() == name)
        .ok_or("missing HCL attribute")?;
    Ok(serde_json::from_value(serde_json::to_value(
        hcl::Value::from(attribute.expr.clone()),
    )?)?)
}

fn policy_scopes(methods: &[ApiMethod]) -> Result<Scopes, Box<dyn Error>> {
    let provider = AwsProvider::new();
    let policy = provider.permissions_policy(methods)?;
    let mut json_scopes = Scopes::new();
    for statement in policy["Statement"].as_array().ok_or("missing statements")? {
        assert_eq!(statement["Effect"], "Allow");
        let resources = match &statement["Resource"] {
            Value::String(value) => vec![value.clone()],
            value => serde_json::from_value(value.clone())?,
        };
        let actions: Vec<String> = serde_json::from_value(statement["Action"].clone())?;
        for action in actions {
            assert!(json_scopes.insert(action, resources.clone()).is_none());
        }
    }

    let body: hcl::Body = hcl::from_str(&provider.permissions_policy_hcl(methods)?)?;
    let data = body.blocks().next().ok_or("missing data block")?;
    let mut hcl_scopes = Scopes::new();
    for statement in data.body.blocks() {
        assert_eq!(hcl_attribute::<String>(statement, "effect")?, "Allow");
        let resources: Vec<String> = hcl_attribute(statement, "resources")?;
        let actions: Vec<String> = hcl_attribute(statement, "actions")?;
        for action in actions {
            assert!(hcl_scopes.insert(action, resources.clone()).is_none());
        }
    }
    assert_eq!(json_scopes, hcl_scopes);
    Ok(json_scopes)
}

#[test]
fn agentless_location_creation_has_wildcard_scope_in_json_and_hcl() -> Result<(), Box<dyn Error>> {
    let provider = AwsProvider::new();
    for operation in [
        "CreateLocationS3",
        "CreateLocationAzureBlob",
        "CreateLocationObjectStorage",
    ] {
        let method = ApiMethod::new("datasync", operation);
        let action = format!("datasync:{operation}");
        assert_eq!(
            provider.iam_permissions(&method)?,
            [action.as_str(), "datasync:TagResource", "iam:PassRole"]
        );
        let scopes = policy_scopes(&[method])?;
        assert_eq!(scopes.len(), 3);
        assert_eq!(scopes[&action], ["*"]);
        assert_eq!(
            scopes["iam:PassRole"],
            ["arn:${Partition}:iam::${Account}:role/${RoleNameWithPath}"]
        );
        assert!(scopes["datasync:TagResource"].iter().all(|r| r != "*"));
    }
    Ok(())
}

#[test]
fn mixed_creation_and_agent_operations_keep_separate_scopes_and_deduplicate()
-> Result<(), Box<dyn Error>> {
    let methods = [
        ApiMethod::new("datasync", "CreateLocationS3"),
        ApiMethod::new("datasync", "CreateLocationAzureBlob"),
        ApiMethod::new("datasync", "CreateLocationObjectStorage"),
        ApiMethod::new("datasync", "DescribeAgent"),
        ApiMethod::new("datasync", "UpdateLocationNfs"),
    ];
    let scopes = policy_scopes(&methods)?;
    assert_eq!(
        scopes["datasync:DescribeAgent"],
        ["arn:${Partition}:datasync:${Region}:${AccountId}:agent/${AgentId}"]
    );
    assert!(scopes["datasync:UpdateLocationNfs"].contains(
        &"arn:${Partition}:datasync:${Region}:${AccountId}:location/${LocationId}".to_owned()
    ));
    assert!(
        scopes["datasync:UpdateLocationNfs"]
            .iter()
            .all(|r| r != "*")
    );

    let provider = AwsProvider::new();
    let policy = provider.permissions_policy(&methods)?;
    let wildcard = policy["Statement"]
        .as_array()
        .ok_or("missing statements")?
        .iter()
        .find(|statement| statement["Resource"] == "*")
        .ok_or("missing agentless creation grant")?;
    assert_eq!(
        wildcard["Action"],
        json!([
            "datasync:CreateLocationAzureBlob",
            "datasync:CreateLocationObjectStorage",
            "datasync:CreateLocationS3"
        ])
    );
    let mut reversed = methods.to_vec();
    reversed.reverse();
    reversed.push(methods[0].clone());
    assert_eq!(policy, provider.permissions_policy(&reversed)?);
    assert_eq!(scopes, policy_scopes(&reversed)?);
    assert_eq!(
        provider.permissions_policy_hcl(&methods)?,
        provider.permissions_policy_hcl(&reversed)?
    );
    Ok(())
}
