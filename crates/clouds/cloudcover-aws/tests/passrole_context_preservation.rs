use std::error::Error;

use cloudcover_aws::{AwsError, AwsProvider};
use cloudcover_core::{ApiMethod, CloudProvider};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn outcomes(provider: AwsProvider, methods: &[ApiMethod]) -> [Result<(), AwsError>; 6] {
    [
        provider.permissions_policy(methods).map(|_| ()),
        provider.permissions_policy_hcl(methods).map(|_| ()),
        provider
            .review_permissions_policy(methods, false)
            .map(|_| ()),
        provider
            .review_permissions_policy_hcl(methods, false)
            .map(|_| ()),
        provider
            .review_permissions_policy(methods, true)
            .map(|_| ()),
        provider
            .review_permissions_policy_hcl(methods, true)
            .map(|_| ()),
    ]
}

#[test]
fn four_context_changes_reject_all_72_whole_requests_and_preserve_shared_fas()
-> Result<(), Box<dyn Error>> {
    let review: Value =
        serde_json::from_str(include_str!("../data/passrole-context-preservation.json"))?;
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
    let mut cases = 0;
    for key in review["operations"].as_object().ok_or("operations")?.keys() {
        let (service, name) = key.split_once(':').ok_or("operation key")?;
        let target = ApiMethod::new(service, name);
        if baseline || candidate {
            assert!(methods.contains(&target));
            assert!(provider.iam_permissions(&target)?.contains(&"iam:PassRole"));
            provider.authorization_coverage(&target)?;
        }
        if baseline {
            assert!(provider.authorization_disposition(&target).is_none());
        }
        if candidate {
            let disposition = provider
                .authorization_disposition(&target)
                .ok_or("context diagnostic")?;
            assert!(disposition.blocks_policy());
            assert_eq!(disposition.kind, "manual_review");
        }
        if !(baseline || candidate) {
            continue;
        }
        // GetObject and CopyObject share downstream KMS authorization. A
        // blocked operation cannot turn those source/FAS grants into a partial
        // policy, regardless of where it occurs in the request.
        for request in [
            vec![target.clone()],
            vec![
                ApiMethod::new("s3", "GetObject"),
                ApiMethod::new("s3", "CopyObject"),
                target.clone(),
            ],
            vec![
                target.clone(),
                ApiMethod::new("s3", "CopyObject"),
                ApiMethod::new("s3", "GetObject"),
            ],
        ] {
            for result in outcomes(provider, &request) {
                cases += 1;
                if baseline {
                    result?;
                } else {
                    let error = result
                        .err()
                        .ok_or("context drift returned a policy or partial grant")?;
                    assert!(
                        matches!(&error, AwsError::UnresolvedAuthorization {service:s,name:n} if s==service && n==name)
                    );
                    let diagnostic = error.to_string();
                    for text in [
                        "iam:PassRole",
                        "concrete requested role ARN",
                        "iam:PassedToService",
                        "entire request",
                        "conditional opt-in cannot authorize",
                    ] {
                        assert!(diagnostic.contains(text));
                    }
                }
            }
        }
    }
    if baseline || candidate {
        assert_eq!(cases, 72);
    }
    let shared = [
        ApiMethod::new("s3", "GetObject"),
        ApiMethod::new("s3", "CopyObject"),
    ];
    for result in outcomes(provider, &shared) {
        result?;
    }
    assert!(matches!(
        provider.permissions_policy(&[ApiMethod::new("unknown", "Unknown")]),
        Err(AwsError::UnknownApiMethod { .. })
    ));
    Ok(())
}
