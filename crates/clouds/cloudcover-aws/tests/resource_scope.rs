use std::{collections::BTreeMap, error::Error};

use cloudcover_aws::{AwsError, AwsProvider};
use cloudcover_core::{ApiMethod, CloudProvider};
use serde_json::Value;

fn incomplete_actions(catalog: &Value) -> Result<BTreeMap<String, Vec<String>>, Box<dyn Error>> {
    let mut result = BTreeMap::new();
    for service in catalog["services"].as_array().ok_or("services")? {
        let prefix = service["service"].as_str().ok_or("service")?;
        let document = &service["document"];
        let resources = document["Resources"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for action in document["Actions"].as_array().ok_or("actions")? {
            let mut missing = Vec::new();
            for reference in action["Resources"].as_array().cloned().unwrap_or_default() {
                let name = reference["Name"].as_str().ok_or("resource name")?;
                let definition = resources.iter().find(|resource| resource["Name"] == name);
                if definition
                    .and_then(|resource| resource["ARNFormats"].as_array())
                    .is_none_or(Vec::is_empty)
                {
                    missing.push(name.to_owned());
                }
            }
            if !missing.is_empty() {
                result.insert(
                    format!("{prefix}:{}", action["Name"].as_str().ok_or("action")?),
                    missing,
                );
            }
        }
    }
    Ok(result)
}

fn assert_scope_error(
    result: Result<(), AwsError>,
    incomplete: &BTreeMap<String, Vec<String>>,
) -> Result<(), Box<dyn Error>> {
    let error = result.err().ok_or("incomplete scope produced a policy")?;
    let AwsError::UnresolvedAuthorization { service, name } = &error else {
        return Err(format!("unexpected scope error: {error}").into());
    };
    let permission = format!("{service}:{name}");
    let missing = incomplete
        .get(&permission)
        .ok_or("scope error omitted affected action")?;
    let message = error.to_string();
    assert!(message.contains(&permission));
    assert!(message.contains("incomplete resource scope"));
    assert!(message.contains("Review the pinned SAR resource definitions"));
    assert!(message.contains("without wildcard or partial grants"));
    for name in missing {
        assert!(message.contains(name));
    }
    Ok(())
}

#[test]
fn pinned_incomplete_direct_carriers_fail_closed_with_actionable_diagnostics()
-> Result<(), Box<dyn Error>> {
    let catalog: Value = serde_json::from_str(include_str!("../data/sar/catalog.json"))?;
    let incomplete = incomplete_actions(&catalog)?;
    let provider = AwsProvider::new();
    for service in catalog["services"].as_array().ok_or("services")? {
        let prefix = service["service"].as_str().ok_or("service")?;
        for operation in service["document"]["Operations"]
            .as_array()
            .cloned()
            .unwrap_or_default()
        {
            let carriers = operation["AuthorizedActions"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if !carriers.iter().any(|action| {
                incomplete.contains_key(&format!(
                    "{}:{}",
                    action["Service"].as_str().unwrap_or_default(),
                    action["Name"].as_str().unwrap_or_default()
                ))
            }) {
                continue;
            }
            let method = ApiMethod::new(prefix, operation["Name"].as_str().ok_or("operation")?);
            // The raw incomplete carriers in these paired snapshots are direct
            // ordinary grants. Conditional exclusion is covered separately by
            // the synthetic conditional-scope test; it never renders that link.
            for include in [false, true] {
                for methods in [
                    vec![method.clone()],
                    vec![ApiMethod::new("s3", "GetObject"), method.clone()],
                ] {
                    for result in [
                        provider.permissions_policy(&methods).map(|_| ()),
                        provider.permissions_policy_hcl(&methods).map(|_| ()),
                        provider
                            .review_permissions_policy(&methods, include)
                            .map(|_| ()),
                        provider
                            .review_permissions_policy_hcl(&methods, include)
                            .map(|_| ()),
                    ] {
                        assert_scope_error(result, &incomplete)?;
                    }
                }
            }
        }
    }
    Ok(())
}
