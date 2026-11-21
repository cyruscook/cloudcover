use std::error::Error;

use cloudcover_aws::{AwsError, AwsProvider};
use cloudcover_core::{ApiMethod, CloudProvider};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[test]
fn global_scope_transitions_reject_complete_json_and_hcl_without_losing_evidence()
-> Result<(), Box<dyn Error>> {
    let review: Value =
        serde_json::from_str(include_str!("../data/global-scope-preservation.json"))?;
    let digest = format!(
        "{:x}",
        Sha256::digest(include_bytes!("../data/sar/catalog.json"))
    );
    let baseline = digest
        == review["baseline"]["catalog_sha256"]
            .as_str()
            .ok_or("baseline hash")?;
    let candidate = digest
        == review["candidate"]["catalog_sha256"]
            .as_str()
            .ok_or("candidate hash")?;
    let provider = AwsProvider::new();
    let methods = provider.list_api_methods();
    for (key, entry) in review["operations"].as_object().ok_or("operations")? {
        let (service, name) = key.split_once(':').ok_or("method")?;
        let method = ApiMethod::new(service, name);
        assert!(methods.contains(&method));
        if baseline {
            assert!(provider.authorization_disposition(&method).is_none());
        }
        if candidate {
            let disposition = provider
                .authorization_disposition(&method)
                .ok_or("missing scope disposition")?;
            assert_eq!(disposition.kind, "manual_review");
            assert!(disposition.blocks_policy());
            let evidence = provider.iam_permissions(&method)?;
            for permission in entry["affected_permissions"]
                .as_array()
                .ok_or("permissions")?
            {
                assert!(evidence.contains(&permission.as_str().ok_or("permission")?));
            }
        }
        if !(baseline || candidate) {
            continue;
        }
        for include in [false, true] {
            for request in [
                vec![method.clone()],
                vec![ApiMethod::new("s3", "GetObject"), method.clone()],
                vec![method.clone(), ApiMethod::new("s3", "GetObject")],
            ] {
                for result in [
                    provider.permissions_policy(&request).map(|_| ()),
                    provider.permissions_policy_hcl(&request).map(|_| ()),
                    provider
                        .review_permissions_policy(&request, include)
                        .map(|_| ()),
                    provider
                        .review_permissions_policy_hcl(&request, include)
                        .map(|_| ()),
                ] {
                    if baseline {
                        result?;
                    } else {
                        let error = result
                            .err()
                            .ok_or("global scope returned a policy or partial grant")?;
                        assert!(
                            matches!(&error,AwsError::UnresolvedAuthorization{service:s,name:n} if s==service && n==name)
                        );
                        let message = error.to_string();
                        for expected in [
                            "entire request",
                            "upstream global declaration",
                            "resource or condition constraints",
                            "conditional opt-in cannot bypass",
                        ] {
                            assert!(message.contains(expected));
                        }
                        for permission in entry["affected_permissions"]
                            .as_array()
                            .ok_or("permissions")?
                        {
                            assert!(message.contains(permission.as_str().ok_or("permission")?));
                        }
                    }
                }
            }
        }
    }
    // Legitimate globals outside this finite reviewed frontier retain policy
    // support. Unknown operations retain ordinary fail-closed behavior.
    let historical = ApiMethod::new("ec2", "DescribeRegions");
    assert!(provider.authorization_disposition(&historical).is_none());
    assert!(
        provider
            .permissions_policy(&[historical])?
            .to_string()
            .contains('*')
    );
    assert!(matches!(
        provider.permissions_policy(&[ApiMethod::new("unknown", "Unknown")]),
        Err(AwsError::UnknownApiMethod { .. })
    ));
    Ok(())
}
