use std::error::Error;

use cloudcover_aws::{AwsError, AwsProvider};
use cloudcover_core::{ApiMethod, CloudProvider};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[test]
fn known_role_transitions_block_complete_policies_without_hiding_evidence()
-> Result<(), Box<dyn Error>> {
    let review: Value = serde_json::from_str(include_str!("../data/passrole-preservation.json"))?;
    let contents = include_bytes!("../data/sar/catalog.json");
    let digest = format!("{:x}", Sha256::digest(contents));
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
    let mut present = 0;
    for key in review["operations"].as_object().ok_or("operations")?.keys() {
        let (service, name) = key.split_once(':').ok_or("operation key")?;
        let method = ApiMethod::new(service, name);
        let indexed = methods.contains(&method);
        present += usize::from(indexed);
        if baseline {
            assert!(provider.authorization_disposition(&method).is_none());
        }
        if !candidate && !(baseline && !indexed) {
            continue;
        }
        if candidate {
            let disposition = provider
                .authorization_disposition(&method)
                .ok_or("missing role diagnostic")?;
            assert!(disposition.blocks_policy());
            assert_eq!(disposition.kind, "manual_review");
            assert!(provider.iam_permissions(&method)?.contains(&"iam:PassRole"));
            provider.authorization_coverage(&method)?;
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
                    let error = result
                        .err()
                        .ok_or("role transition returned a complete or partial policy")?;
                    if candidate {
                        assert!(
                            matches!(&error,AwsError::UnresolvedAuthorization{service:s,name:n} if s==service && n==name)
                        );
                        let text = error.to_string();
                        assert!(text.contains("iam:PassRole"));
                        assert!(text.contains("concrete requested role ARN"));
                        assert!(text.contains("iam:PassedToService"));
                        assert!(text.contains("entire request"));
                    } else {
                        assert!(matches!(error, AwsError::UnknownApiMethod { .. }));
                    }
                }
            }
        }
    }
    if baseline {
        assert_eq!(present, 12);
    }
    if candidate {
        assert_eq!(present, 22);
    }
    // Unrelated unknowns keep the ordinary unknown-operation error.
    assert!(matches!(
        provider.permissions_policy(&[ApiMethod::new("unknown", "Unknown")]),
        Err(AwsError::UnknownApiMethod { .. })
    ));
    Ok(())
}
