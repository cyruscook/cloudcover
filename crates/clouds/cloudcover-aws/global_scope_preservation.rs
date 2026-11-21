//! Fail closed on the reviewed frontier of newly unscoped default grants.
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
struct Entry {
    baseline_fingerprint: String,
    // Describes the reviewed risk; this list never authorizes any permission.
    affected_permissions: Vec<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Review {
    schema_version: u8,
    purpose: String,
    baseline: Pair,
    candidate: Pair,
    operations: BTreeMap<String, Entry>,
}

fn qualified(value: &str) -> bool {
    value
        .split_once(':')
        .is_some_and(|(service, name)| !service.is_empty() && !name.is_empty())
}

fn load() -> BuildResult<Review> {
    let review: Review = serde_json::from_str(include_str!("data/global-scope-preservation.json"))?;
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
        || review.operations.iter().any(|(key, entry)| {
            !qualified(key)
                || !valid_hash(&entry.baseline_fingerprint, 64)
                || entry.affected_permissions.is_empty()
                || entry
                    .affected_permissions
                    .iter()
                    .any(|permission| !qualified(permission))
        })
    {
        return Err("invalid global-scope preservation provenance or entry".into());
    }
    Ok(review)
}

fn blocked<'a>(catalog: &Value, review: &'a Review) -> Vec<&'a str> {
    review
        .operations
        .iter()
        .filter_map(|(key, entry)| {
            let (service, name) = key.split_once(':')?;
            // Equality preserves only the exact historical closure. Changed scope,
            // an added carrier, absent/null context or ambiguity never adds a grant.
            (fingerprint(catalog, service, name).ok().as_ref() != Some(&entry.baseline_fingerprint))
                .then_some(key.as_str())
        })
        .collect()
}

pub(super) fn generate(contents: &[u8]) -> BuildResult<String> {
    let review = load()?;
    let catalog: Value = serde_json::from_slice(contents)?;
    let mut generated = String::from(
        "pub(super) const GLOBAL_SCOPE_AUTHORIZATION_DISPOSITIONS: &[crate::disposition::OperationDisposition] = &[\n",
    );
    for key in blocked(&catalog, &review) {
        let (service, operation) = key.split_once(':').ok_or("invalid scope operation")?;
        let permissions = review.operations[key].affected_permissions.join(", ");
        let message = format!(
            "Changed authorization scope requires application review before granting {permissions}. Policy generation is blocked for the entire request. An upstream global declaration does not approve an unrestricted grant. Review request-dependent applicability and resource or condition constraints, and supply a separately reviewed policy; conditional opt-in cannot bypass this review."
        );
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
            "Operations":[{"Name":"Update","SDK":[],"AuthorizedActions":[{"Service":"test","Name":"Update","Context":{"test:Flag":["reviewed"]}}]}],
            "Actions":[{"Name":"Update","Resources":[{"Name":"item"}],"ActionConditionKeys":["test:Flag"]},{"Name":"Create","Resources":[]}],
            "Resources":[{"Name":"item","ARNFormats":["arn:test:item/${Id}"]}],
            "ConditionKeys":[{"Name":"test:Flag","Types":["String"]}]
        }}]})
    }
    fn review(catalog: &Value) -> BuildResult<Review> {
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
                "test:Update".to_owned(),
                Entry {
                    baseline_fingerprint: fingerprint(catalog, "test", "Update")?,
                    affected_permissions: vec!["test:Create".to_owned()],
                },
            )]),
        })
    }

    #[test]
    fn added_global_carrier_is_not_an_existing_action_scope_widening() -> BuildResult<()> {
        let baseline = fixture();
        let review = review(&baseline)?;
        assert_eq!(blocked(&baseline, &review), Vec::<&str>::new());
        let mut added = baseline.clone();
        added["services"][0]["document"]["Operations"][0]["AuthorizedActions"]
            .as_array_mut()
            .ok_or("links")?
            .push(json!({"Service":"test","Name":"Create"}));
        // The base action stays scoped; the global action's definition also
        // stays identical. Comparing action scopes alone misses the new link.
        assert_eq!(
            baseline["services"][0]["document"]["Actions"],
            added["services"][0]["document"]["Actions"]
        );
        assert_eq!(blocked(&added, &review), ["test:Update"]);
        let mut widened = baseline.clone();
        widened["services"][0]["document"]["Actions"][0]["Resources"] = json!([]);
        assert_eq!(blocked(&widened, &review), ["test:Update"]);
        Ok(())
    }

    #[test]
    fn partial_missing_context_and_ambiguous_evidence_cannot_preserve_scope() -> BuildResult<()> {
        let baseline = fixture();
        let review = review(&baseline)?;
        for mutation in 0..10 {
            let mut changed = baseline.clone();
            let doc = &mut changed["services"][0]["document"];
            match mutation {
                0 => doc["Resources"] = json!([]),
                1 => doc["Resources"][0]["ARNFormats"] = json!([]),
                2 => {
                    doc["Actions"][0]["Resources"]
                        .as_array_mut()
                        .ok_or("resources")?
                        .push(json!({"Name":"missing"}));
                }
                3 => doc["Operations"][0]["AuthorizedActions"][0]["Context"] = Value::Null,
                4 => {
                    doc["Operations"][0]["AuthorizedActions"][0]
                        .as_object_mut()
                        .ok_or("link")?
                        .remove("Context");
                }
                5 => {
                    doc["Operations"][0]["AuthorizedActions"][0]["Context"]["test:Flag"] =
                        json!(["stricter"])
                }
                6 => {
                    let duplicate = doc["Resources"][0].clone();
                    doc["Resources"]
                        .as_array_mut()
                        .ok_or("resources")?
                        .push(duplicate);
                }
                7 => doc["Operations"][0]["AuthorizedActions"] = json!([]),
                8 => doc["Operations"] = json!([]),
                9 => doc["ConditionKeys"][0]["Types"] = json!(["Boolean"]),
                _ => unreachable!(),
            }
            assert_eq!(
                blocked(&changed, &review),
                ["test:Update"],
                "mutation {mutation}"
            );
        }
        Ok(())
    }

    #[test]
    fn historical_globals_and_unrelated_sdk_metadata_keep_their_behavior() -> BuildResult<()> {
        let mut baseline = fixture();
        baseline["services"][0]["document"]["Actions"][0]["Resources"] = json!([]);
        let review = review(&baseline)?;
        let mut benign = baseline.clone();
        benign["services"][0]["document"]["Operations"][0]["SDK"] =
            json!([{"Name":"new SDK projection"}]);
        benign["services"][0]["document"]["Actions"]
            .as_array_mut()
            .ok_or("actions")?
            .push(json!({"Name":"UnrelatedGlobal","Resources":[]}));
        assert_eq!(blocked(&benign, &review), Vec::<&str>::new());
        Ok(())
    }

    #[test]
    fn exact_paired_inputs_keep_zero_old_and_six_candidate_blockers() -> BuildResult<()> {
        let contents = std::fs::read("data/sar/catalog.json")?;
        let catalog: Value = serde_json::from_slice(&contents)?;
        let review = load()?;
        assert_eq!(review.operations.len(), 6);
        assert_eq!(
            review
                .operations
                .values()
                .map(|e| e.affected_permissions.len())
                .sum::<usize>(),
            7
        );
        let digest = format!("{:x}", Sha256::digest(&contents));
        if digest == review.baseline.catalog_sha256 {
            assert_eq!(blocked(&catalog, &review), Vec::<&str>::new());
        }
        if digest == review.candidate.catalog_sha256 {
            assert_eq!(blocked(&catalog, &review).len(), 6);
        }
        Ok(())
    }
}
