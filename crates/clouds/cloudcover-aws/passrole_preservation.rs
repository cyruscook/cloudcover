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

fn load_review(source: &str) -> BuildResult<Review> {
    let review: Review = serde_json::from_str(source)?;
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

fn load() -> BuildResult<Review> {
    load_review(include_str!("data/passrole-preservation.json"))
}

fn context_review() -> BuildResult<Review> {
    load_review(include_str!("data/passrole-context-preservation.json"))
}

// A Value parser normally discards duplicate object keys. That must not let
// ambiguous raw Context JSON impersonate an exact historical closure. This
// additional check is used only by the four-operation context boundary.
fn context_catalog(contents: &[u8]) -> BuildResult<Value> {
    struct Unique(Value);
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = Unique;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("JSON without duplicate object keys")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<Unique, M::Error> {
            let mut object = serde_json::Map::new();
            while let Some((key, value)) = map.next_entry::<String, Unique>()? {
                if object.insert(key, value.0).is_some() {
                    return Err(serde::de::Error::custom("duplicate authorization JSON key"));
                }
            }
            Ok(Unique(Value::Object(object)))
        }
        fn visit_seq<S: serde::de::SeqAccess<'de>>(self, mut seq: S) -> Result<Unique, S::Error> {
            let mut array = Vec::new();
            while let Some(Unique(value)) = seq.next_element::<Unique>()? {
                array.push(value);
            }
            Ok(Unique(Value::Array(array)))
        }
        fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Unique, E> {
            Ok(Unique(Value::String(value.to_owned())))
        }
        fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Unique, E> {
            Ok(Unique(Value::String(value)))
        }
        fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Unique, E> {
            Ok(Unique(Value::Bool(value)))
        }
        fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Unique, E> {
            Ok(Unique(Value::from(value)))
        }
        fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Unique, E> {
            Ok(Unique(Value::from(value)))
        }
        fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Unique, E> {
            serde_json::Number::from_f64(value)
                .map(|number| Unique(Value::Number(number)))
                .ok_or_else(|| E::custom("invalid authorization JSON number"))
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<Unique, E> {
            Ok(Unique(Value::Null))
        }
    }
    impl<'de> serde::Deserialize<'de> for Unique {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            deserializer.deserialize_any(Visitor)
        }
    }
    let mut deserializer = serde_json::Deserializer::from_slice(contents);
    let Unique(catalog) = <Unique as serde::Deserialize>::deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(catalog)
}

fn context_blocked(contents: &[u8], review: &Review) -> Vec<(String, String)> {
    match context_catalog(contents) {
        Ok(catalog) => blocked(&catalog, review),
        Err(_) => blocked(&Value::Null, review),
    }
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
    let context_boundary = context_review()?;
    if context_boundary
        .operations
        .keys()
        .any(|key| review.operations.contains_key(key))
    {
        return Err("overlapping PassRole preservation boundaries".into());
    }
    let catalog: Value = serde_json::from_slice(contents)?;
    let mut generated = String::from(
        "pub(super) const PRIVILEGED_AUTHORIZATION_DISPOSITIONS: &[crate::disposition::OperationDisposition] = &[\n",
    );
    let blocked = blocked(&catalog, &review)
        .into_iter()
        .map(|(service, operation)| (service, operation, review.candidate.commit.as_str()))
        .chain(
            context_blocked(contents, &context_boundary)
                .into_iter()
                .map(|(service, operation)| {
                    (
                        service,
                        operation,
                        context_boundary.candidate.commit.as_str(),
                    )
                }),
        );
    for (service, operation, source_commit) in blocked {
        let message = "New or changed iam:PassRole authorization evidence is outside the exact historical preservation boundary. Automatic policy generation is blocked for the entire request. Review the API's concrete requested role ARN and exact iam:PassedToService constraints and supply a separately reviewed scoped policy; a role ARN template, raw Context or conditional opt-in cannot authorize a grant.";
        let source = format!(
            "https://github.com/cyruscook/cloudcover/blob/{source_commit}/crates/clouds/cloudcover-aws/data/sar/catalog.json"
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

    fn historical_role_fixture() -> BuildResult<Value> {
        let mut catalog = fixture();
        catalog["services"][0]["document"]["Operations"][0]["AuthorizedActions"]
            .as_array_mut()
            .ok_or("links")?
            .push(json!({"Service":"iam","Name":"PassRole",
                "Context":{"iam:PassedToService":["test.amazonaws.com"]}}));
        Ok(catalog)
    }

    #[test]
    #[allow(clippy::too_many_lines)] // Keep the related 24-case mutation matrix together.
    fn retained_context_carrier_and_arn_mutations_fail_closed() -> BuildResult<()> {
        let baseline = historical_role_fixture()?;
        let review = review(&baseline, "Create")?;
        assert_eq!(
            context_blocked(&serde_json::to_vec(&baseline)?, &review),
            Vec::<(String, String)>::new()
        );
        for mutation in 0..24 {
            let mut changed = baseline.clone();
            match mutation {
                0 => {
                    changed["services"][0]["document"]["Operations"][0]["AuthorizedActions"][2]["Context"]
                        ["iam:PassedToService"] = json!(["other.amazonaws.com"]);
                }
                1 => {
                    changed["services"][0]["document"]["Operations"][0]["AuthorizedActions"][2]["Context"] =
                        Value::Null;
                }
                2 => {
                    changed["services"][0]["document"]["Operations"][0]["AuthorizedActions"][2]
                        .as_object_mut()
                        .ok_or("link")?
                        .remove("Context");
                }
                3 => {
                    changed["services"][0]["document"]["Operations"][0]["AuthorizedActions"][2]["Context"] =
                        json!({"unrecognized":{"scope":"changed"}});
                }
                4 => {
                    changed["services"][0]["document"]["Operations"][0]["AuthorizedActions"]
                        .as_array_mut()
                        .ok_or("links")?
                        .pop();
                }
                5 => {
                    let link =
                        changed["services"][0]["document"]["Operations"][0]["AuthorizedActions"][2]
                            .clone();
                    changed["services"][0]["document"]["Operations"][0]["AuthorizedActions"]
                        .as_array_mut()
                        .ok_or("links")?
                        .push(link);
                }
                6 => {
                    changed["services"][0]["document"]["Operations"][0]["AuthorizedActions"][2]["Service"] =
                        json!("unknown");
                }
                7 => changed["services"][1]["document"]["Actions"] = json!([]),
                8 => {
                    let action = changed["services"][1]["document"]["Actions"][0].clone();
                    changed["services"][1]["document"]["Actions"]
                        .as_array_mut()
                        .ok_or("actions")?
                        .push(action);
                }
                9 => changed["services"][1]["document"]["Resources"] = json!([]),
                10 => changed["services"][1]["document"]["Resources"][0]["ARNFormats"] = json!([]),
                11 => {
                    changed["services"][1]["document"]["Resources"][0]["ARNFormats"] = json!(["*"]);
                }
                12 => {
                    changed["services"][1]["document"]["Resources"][0]["ARNFormats"] = json!([""]);
                }
                13 => {
                    changed["services"][1]["document"]["Resources"][0]["ARNFormats"] =
                        json!(["arn:aws:iam::123456789012:role/concrete-but-unreviewed"]);
                }
                14 => {
                    changed["services"][1]["document"]["ConditionKeys"][0]["Types"] =
                        json!(["Boolean"]);
                }
                15 => {
                    let key = changed["services"][1]["document"]["ConditionKeys"][0].clone();
                    changed["services"][1]["document"]["ConditionKeys"]
                        .as_array_mut()
                        .ok_or("keys")?
                        .push(key);
                }
                16 => changed["services"][0]["document"]["Operations"] = json!([]),
                17 => {
                    let op = changed["services"][0]["document"]["Operations"][0].clone();
                    changed["services"][0]["document"]["Operations"]
                        .as_array_mut()
                        .ok_or("operations")?
                        .push(op);
                }
                18 => {
                    changed["services"][0]["document"]["Operations"][0]["UnknownAuthorization"] =
                        json!(true);
                }
                19 => changed["services"][1]["document"]["Actions"][0]["Resources"] = Value::Null,
                20 => {
                    changed["services"][1]["document"]["Actions"][0]["Resources"] =
                        json!([{"Name":"unrecognized"}]);
                }
                21 => {
                    let service = changed["services"][1].clone();
                    changed["services"]
                        .as_array_mut()
                        .ok_or("services")?
                        .push(service);
                }
                22 => {
                    changed["services"][0]["document"]["Operations"][0]["AuthorizedActions"][2]["Name"] =
                        json!("Unknown");
                }
                23 => {
                    changed["services"][0]["document"]["Operations"][0]["AuthorizedActions"][2]["Context"] =
                        json!(7);
                }
                _ => unreachable!(),
            }
            assert_eq!(
                context_blocked(&serde_json::to_vec(&changed)?, &review).len(),
                1,
                "mutation {mutation}"
            );
        }
        Ok(())
    }

    #[test]
    fn duplicate_raw_context_cannot_impersonate_a_preserved_value() -> BuildResult<()> {
        let baseline = historical_role_fixture()?;
        let review = review(&baseline, "Create")?;
        let original = serde_json::to_string(&baseline)?;
        let ambiguous = original.replace(
            "\"iam:PassedToService\":[\"test.amazonaws.com\"]",
            "\"iam:PassedToService\":[\"other.amazonaws.com\"],\"iam:PassedToService\":[\"test.amazonaws.com\"]",
        );
        assert_ne!(ambiguous, original);
        // A partial guard operating on an ordinary Value would accept this
        // ambiguous input after the first key has silently disappeared.
        assert_eq!(
            blocked(&serde_json::from_str(&ambiguous)?, &review),
            Vec::<(String, String)>::new()
        );
        assert_eq!(context_blocked(ambiguous.as_bytes(), &review).len(), 1);
        for invalid in [b"{".as_slice(), b"[] trailing".as_slice()] {
            assert_eq!(context_blocked(invalid, &review).len(), 1);
        }
        assert_eq!(context_catalog(original.as_bytes())?, baseline);
        Ok(())
    }

    #[test]
    fn context_preservation_keeps_sdk_and_unrelated_evidence_changes() -> BuildResult<()> {
        let baseline = historical_role_fixture()?;
        let review = review(&baseline, "Create")?;
        let mut benign = baseline.clone();
        benign["services"][0]["document"]["Operations"][0]["SDK"] =
            json!([{"Name":"new SDK projection"}]);
        benign["services"][0]["document"]["Operations"][0]["AuthorizedActions"]
            .as_array_mut()
            .ok_or("links")?
            .reverse();
        benign["services"][1]["document"]["Resources"]
            .as_array_mut()
            .ok_or("resources")?
            .push(json!({"Name":"unrelated","ARNFormats":["*"]}));
        assert_eq!(
            context_blocked(&serde_json::to_vec_pretty(&benign)?, &review),
            Vec::<(String, String)>::new()
        );
        Ok(())
    }

    #[test]
    fn paired_context_boundary_is_exactly_four_operations() -> BuildResult<()> {
        let review = context_review()?;
        assert_eq!(
            review
                .operations
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "cognito-identity:SetIdentityPoolRoles",
                "elasticmapreduce:StartSession",
                "observabilityadmin:CreateS3TableIntegration",
                "sagemaker:CreateTrainingSession",
            ]
        );
        let prior = load()?;
        assert!(
            review
                .operations
                .keys()
                .all(|key| !prior.operations.contains_key(key))
        );
        let contents = std::fs::read("data/sar/catalog.json")?;
        let manifest = std::fs::read("data/sar/manifest.json")?;
        let digest = format!("{:x}", Sha256::digest(&contents));
        if digest == review.baseline.catalog_sha256 {
            assert_eq!(
                format!("{:x}", Sha256::digest(&manifest)),
                review.baseline.manifest_sha256
            );
            assert_eq!(
                context_blocked(&contents, &review),
                Vec::<(String, String)>::new()
            );
        }
        if digest == review.candidate.catalog_sha256 {
            assert_eq!(
                format!("{:x}", Sha256::digest(&manifest)),
                review.candidate.manifest_sha256
            );
            assert_eq!(context_blocked(&contents, &review).len(), 4);
        }
        Ok(())
    }
}
