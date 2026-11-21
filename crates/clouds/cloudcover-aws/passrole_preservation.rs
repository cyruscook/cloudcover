//! A preservation boundary, not an authorization supplement. No grants are added.
use std::{collections::BTreeMap, fmt::Write};

use serde_json::Value;

use super::{BuildResult, authorization_evidence::fingerprint};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Pair {
    commit: String,
    catalog_sha256: String,
    manifest_sha256: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Review {
    schema_version: u8,
    purpose: String,
    baseline: Pair,
    candidate: Pair,
    operations: BTreeMap<String, String>,
}

fn load() -> BuildResult<Review> {
    let review: Review = serde_json::from_str(include_str!("data/passrole-preservation.json"))?;
    let valid_hash = |value: &str, size| {
        value.len() == size && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    };
    if review.schema_version != 1
        || review.purpose.is_empty()
        || review.operations.is_empty()
        || [&review.baseline, &review.candidate].iter().any(|pair| {
            !valid_hash(&pair.commit, 40)
                || !valid_hash(&pair.catalog_sha256, 64)
                || !valid_hash(&pair.manifest_sha256, 64)
        })
        || review.operations.iter().any(|(key, hash)| {
            !key.split_once(':')
                .is_some_and(|(service, name)| !service.is_empty() && !name.is_empty())
                || !valid_hash(hash, 64)
        })
    {
        return Err("invalid PassRole preservation provenance or fingerprint".into());
    }
    Ok(review)
}

fn blocked(catalog: &Value, review: &Review) -> Vec<(String, String)> {
    review
        .operations
        .iter()
        .filter_map(|(key, expected)| {
            let (service, name) = key.split_once(':')?;
            // Any malformed, ambiguous or changed shape blocks output; a digest
            // match only preserves historical evidence and never adds a link.
            (fingerprint(catalog, service, name).ok().as_ref() != Some(expected))
                .then(|| (service.to_owned(), name.to_owned()))
        })
        .collect()
}

pub(super) fn generate(contents: &[u8]) -> BuildResult<String> {
    let review = load()?;
    let catalog: Value = serde_json::from_slice(contents)?;
    let mut generated = String::from(
        "pub(super) const PRIVILEGED_AUTHORIZATION_DISPOSITIONS: &[crate::disposition::OperationDisposition] = &[\n",
    );
    for (service, operation) in blocked(&catalog, &review) {
        let message = "New or changed iam:PassRole authorization evidence is outside the exact historical preservation boundary. Automatic policy generation is blocked for the entire request. Review the API's concrete requested role ARN and exact iam:PassedToService constraints and supply a separately reviewed scoped policy; a role ARN template, raw Context or conditional opt-in cannot authorize a grant.";
        let source = format!(
            "https://github.com/cyruscook/cloudcover/blob/{}/crates/clouds/cloudcover-aws/data/sar/catalog.json",
            review.candidate.commit
        );
        writeln!(
            generated,
            "crate::disposition::OperationDisposition {{ service: {service:?}, operation: {operation:?}, kind: \"manual_review\", message: {message:?}, source: {source:?} }},"
        )?;
    }
    generated.push_str("];\n");
    Ok(generated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sha2::{Digest, Sha256};

    fn fixture() -> Value {
        json!({"services":[{"service":"test","document":{
            "Operations":[{"Name":"Create","SDK":[],"AuthorizedActions":[
                {"Service":"test","Name":"Create","Context":{"test:Flag":["old"]}},
                {"Service":"test","Name":"Tag"}
            ]}],
            "Actions":[
                {"Name":"Create","Resources":[{"Name":"thing"}],"ActionConditionKeys":["test:Flag"]},
                {"Name":"Tag","Resources":[{"Name":"thing"}]}
            ],
            "Resources":[{"Name":"thing","ARNFormats":["arn:test:thing/${Id}"]}],
            "ConditionKeys":[{"Name":"test:Flag","Types":["String"]}]
        }},{"service":"iam","document":{
            "Operations":[],"Actions":[{"Name":"PassRole","Resources":[{"Name":"role"}]}],
            "Resources":[{"Name":"role","ARNFormats":["arn:${Partition}:iam::${Account}:role/${RoleNameWithPath}"]}],
            "ConditionKeys":[{"Name":"iam:PassedToService","Types":["String"]}]
        }}]})
    }

    fn review(catalog: &Value, name: &str) -> BuildResult<Review> {
        let pair = || Pair {
            commit: "0".repeat(40),
            catalog_sha256: "0".repeat(64),
            manifest_sha256: "0".repeat(64),
        };
        Ok(Review {
            schema_version: 1,
            purpose: "test".to_owned(),
            baseline: pair(),
            candidate: pair(),
            operations: BTreeMap::from([(
                format!("test:{name}"),
                fingerprint(catalog, "test", name)?,
            )]),
        })
    }

    #[test]
    fn scope_context_and_ambiguous_changes_cannot_reuse_historical_preservation() -> BuildResult<()>
    {
        let baseline = fixture();
        let review = review(&baseline, "Create")?;
        assert_eq!(blocked(&baseline, &review), Vec::<(String, String)>::new());
        for mutation in 0..15 {
            let mut changed = baseline.clone();
            let doc = &mut changed["services"][0]["document"];
            match mutation {
                0 => doc["Operations"][0]["AuthorizedActions"].as_array_mut().ok_or("links")?.push(json!({"Service":"iam","Name":"PassRole","Context":{"iam:PassedToService":["test.amazonaws.com"]}})),
                1 => doc["Operations"][0]["AuthorizedActions"][0]["Context"]["test:Flag"] = json!(["stricter"]),
                2 => doc["Operations"][0]["AuthorizedActions"][0]["Context"] = Value::Null,
                3 => {doc["Operations"][0]["AuthorizedActions"][0].as_object_mut().ok_or("link")?.remove("Context");},
                4 => doc["Resources"][0]["ARNFormats"] = json!(["*"]),
                5 => doc["Resources"][0]["ARNFormats"] = json!([]),
                6 => doc["Resources"] = json!([]),
                7 => doc["Actions"][0]["Resources"] = json!([]),
                8 => doc["ConditionKeys"][0]["Types"] = json!(["Boolean"]),
                9 => doc["Operations"] = json!([]),
                10 => {let duplicate=doc["Operations"][0].clone();doc["Operations"].as_array_mut().ok_or("operations")?.push(duplicate);},
                11 => {let duplicate=doc["Actions"][0].clone();doc["Actions"].as_array_mut().ok_or("actions")?.push(duplicate);},
                12 => {let duplicate=doc["Resources"][0].clone();doc["Resources"].as_array_mut().ok_or("resources")?.push(duplicate);},
                13 => doc["Actions"][0]["UnknownScope"] = json!(true),
                14 => {let duplicate=changed["services"][0].clone();changed["services"].as_array_mut().ok_or("services")?.push(duplicate);},
                _ => unreachable!(),
            }
            assert_eq!(
                blocked(&changed, &review),
                [("test".to_owned(), "Create".to_owned())],
                "mutation {mutation}"
            );
        }
        // SDK-only and unrelated definitions cannot add authorization.
        let mut benign = baseline.clone();
        benign["services"][0]["document"]["Operations"][0]["SDK"] =
            json!([{"Name":"new SDK projection"}]);
        benign["services"][0]["document"]["Operations"][0]["AuthorizedActions"]
            .as_array_mut()
            .ok_or("links")?
            .reverse();
        benign["services"][0]["document"]["Resources"]
            .as_array_mut()
            .ok_or("resources")?
            .push(json!({"Name":"unrelated","ARNFormats":["*"]}));
        assert_eq!(blocked(&benign, &review), Vec::<(String, String)>::new());
        Ok(())
    }

    #[test]
    fn role_scope_or_context_cannot_turn_new_evidence_into_an_automatic_grant() -> BuildResult<()> {
        let baseline = fixture();
        let review = review(&baseline, "Create")?;
        for mutation in 0..8 {
            let mut changed = baseline.clone();
            changed["services"][0]["document"]["Operations"][0]["AuthorizedActions"]
                .as_array_mut().ok_or("links")?.push(json!({
                    "Service":"iam", "Name":"PassRole", "Context":{"iam:PassedToService":["test.amazonaws.com"]}
                }));
            match mutation {
                0 => {}
                1 => {
                    changed["services"][1]["document"]["Resources"][0]["ARNFormats"] = json!(["*"]);
                }
                2 => changed["services"][1]["document"]["Resources"][0]["ARNFormats"] = json!([]),
                3 => changed["services"][1]["document"]["Actions"][0]["Resources"] = json!([]),
                4 => {
                    changed["services"][1]["document"]["Resources"][0]["ARNFormats"] =
                        json!(["arn:aws:iam::123456789012:role/concrete-but-not-request-bound"]);
                }
                5 => {
                    changed["services"][0]["document"]["Operations"][0]["AuthorizedActions"][2]["Context"] =
                        Value::Null;
                }
                6 => {
                    changed["services"][0]["document"]["Operations"][0]["AuthorizedActions"][2]["Context"] =
                        json!({"iam:PassedToService":["other.amazonaws.com"]});
                }
                7 => changed["services"][1]["document"]["Resources"] = json!([]),
                _ => unreachable!(),
            }
            assert_eq!(blocked(&changed, &review).len(), 1, "mutation {mutation}");
        }
        Ok(())
    }

    #[test]
    fn absent_operation_is_not_a_permanent_permissionless_or_fallback_exception() -> BuildResult<()>
    {
        let baseline = fixture();
        let review = review(&baseline, "New")?;
        assert_eq!(blocked(&baseline, &review), Vec::<(String, String)>::new());
        let mut empty = baseline.clone();
        empty["services"][0]["document"]["Operations"]
            .as_array_mut()
            .ok_or("operations")?
            .push(json!({"Name":"New","AuthorizedActions":[]}));
        assert_eq!(blocked(&empty, &review).len(), 1);
        let mut fallback = baseline.clone();
        fallback["services"][0]["document"]["Operations"] = json!([]);
        assert_eq!(blocked(&fallback, &review).len(), 1);
        Ok(())
    }

    #[test]
    fn paired_pins_match_all_twenty_two_reviewed_boundaries() -> BuildResult<()> {
        let contents = std::fs::read("data/sar/catalog.json")?;
        let catalog: Value = serde_json::from_slice(&contents)?;
        let review = load()?;
        assert_eq!(review.operations.len(), 22);
        let digest = format!("{:x}", Sha256::digest(&contents));
        if digest == review.baseline.catalog_sha256 {
            assert_eq!(blocked(&catalog, &review), Vec::<(String, String)>::new());
        }
        if digest == review.candidate.catalog_sha256 {
            assert_eq!(blocked(&catalog, &review).len(), 22);
        }
        Ok(())
    }
}
