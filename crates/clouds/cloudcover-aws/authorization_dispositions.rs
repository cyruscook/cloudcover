//! Exact reviewed losses of authorization metadata. These never create grants.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use serde_json::{Value, json};

use super::BuildResult;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Review {
    schema_version: u8,
    reviewed: String,
    baseline_commit: String,
    candidate_commit: String,
    entries: Vec<Entry>,
    action_scopes: BTreeMap<String, Value>,
    resource_scopes: BTreeMap<String, Value>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    service: String,
    operation: String,
    kind: String,
    candidate_state: String,
    authorized_actions: Vec<Value>,
    message: String,
    source: String,
}

fn load() -> BuildResult<Review> {
    let review: Review =
        serde_json::from_str(include_str!("data/authorization-dispositions.json"))?;
    if review.schema_version != 1
        || review.reviewed.len() != 10
        || [&review.baseline_commit, &review.candidate_commit]
            .iter()
            .any(|s| s.len() != 40 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("invalid authorization disposition provenance".into());
    }
    let mut seen = BTreeSet::new();
    for entry in &review.entries {
        if !seen.insert((&entry.service, &entry.operation))
            || entry.authorized_actions.is_empty()
            || entry.message.is_empty()
            || !(entry.source.starts_with("https://docs.aws.amazon.com/")
                || entry
                    .source
                    .starts_with("https://raw.githubusercontent.com/aws/aws-sdk-go-v2/"))
            || !matches!(
                (entry.kind.as_str(), entry.candidate_state.as_str()),
                ("manual_review" | "retired", "empty") | ("deprecated_sdk_api", "removed")
            )
        {
            return Err("invalid authorization disposition entry".into());
        }
        let mut links = BTreeSet::new();
        for link in &entry.authorized_actions {
            let service = link["Service"]
                .as_str()
                .ok_or("invalid reviewed action service")?;
            let name = link["Name"]
                .as_str()
                .ok_or("invalid reviewed action name")?;
            let key = format!("{service}:{name}");
            if !links.insert(key.clone()) || !review.action_scopes.contains_key(&key) {
                return Err("missing scope or duplicate reviewed disposition action".into());
            }
        }
    }
    for (key, scope) in &review.action_scopes {
        let (service, _) = key.split_once(':').ok_or("invalid reviewed action key")?;
        for resource in scope["Resources"]
            .as_array()
            .ok_or("invalid reviewed resource scope")?
        {
            let name = resource["Name"]
                .as_str()
                .ok_or("invalid reviewed resource name")?;
            if !review
                .resource_scopes
                .contains_key(&format!("{service}:{name}"))
            {
                return Err("missing reviewed disposition resource scope".into());
            }
        }
    }
    Ok(review)
}

fn definition<'a>(
    catalog: &'a Value,
    service: &str,
    category: &str,
    name: &str,
) -> BuildResult<Option<&'a Value>> {
    let services = catalog["services"].as_array().ok_or("missing services")?;
    let service = services
        .iter()
        .find(|s| s["service"] == service)
        .ok_or("missing reviewed disposition service")?;
    let rows = service["document"][category]
        .as_array()
        .ok_or("missing definition table")?;
    let mut found = rows.iter().filter(|row| row["Name"] == name);
    let row = found.next();
    if found.next().is_some() {
        return Err("duplicate reviewed definition".into());
    }
    Ok(row)
}

fn links_shape(links: &[Value]) -> Vec<String> {
    let mut shape: Vec<_> = links.iter().map(Value::to_string).collect();
    shape.sort_unstable();
    shape
}

pub(super) fn validate(contents: &[u8]) -> BuildResult<()> {
    validate_document(&serde_json::from_slice(contents)?, &load()?)
}

fn validate_document(catalog: &Value, review: &Review) -> BuildResult<()> {
    for entry in &review.entries {
        match definition(catalog, &entry.service, "Operations", &entry.operation)? {
            None if entry.candidate_state == "removed" => {}
            Some(row) => {
                let links = match row.get("AuthorizedActions") {
                    None => &[][..],
                    Some(value) => value
                        .as_array()
                        .ok_or("invalid reviewed authorized actions")?,
                };
                if !(links_shape(links) == links_shape(&entry.authorized_actions)
                    || links.is_empty() && entry.candidate_state == "empty")
                {
                    return Err(format!(
                        "reviewed disposition action/context changed for {}:{}",
                        entry.service, entry.operation
                    )
                    .into());
                }
            }
            None => {
                return Err(format!(
                    "reviewed disposition operation removed: {}:{}",
                    entry.service, entry.operation
                )
                .into());
            }
        }
    }
    // Retained action and resource evidence must keep the reviewed scope, even
    // though lifecycle/manual dispositions do not automatically render grants.
    for (key, expected) in &review.action_scopes {
        let (service, name) = key.split_once(':').ok_or("invalid reviewed action key")?;
        let action = definition(catalog, service, "Actions", name)?
            .ok_or("missing reviewed disposition action")?;
        let actual = json!({"Resources": action.get("Resources").cloned().unwrap_or(json!([])),
            "ActionConditionKeys": action.get("ActionConditionKeys").cloned().unwrap_or(json!([]))});
        if &actual != expected {
            return Err(format!("reviewed disposition action scope changed: {key}").into());
        }
    }
    for (key, expected) in &review.resource_scopes {
        let (service, name) = key.split_once(':').ok_or("invalid reviewed resource key")?;
        let resource = definition(catalog, service, "Resources", name)?
            .ok_or("missing reviewed disposition resource")?;
        let actual = json!({"ARNFormats": resource.get("ARNFormats").cloned().unwrap_or(json!([])),
            "ConditionKeys": resource.get("ConditionKeys").cloned().unwrap_or(json!([]))});
        if &actual != expected {
            return Err(format!("reviewed disposition resource scope changed: {key}").into());
        }
    }
    Ok(())
}

pub(super) fn generate() -> BuildResult<String> {
    let review = load()?;
    let mut code = String::from(
        "pub(crate) static AUTHORIZATION_DISPOSITIONS: &[crate::OperationDisposition] = &[\n",
    );
    for entry in review.entries {
        writeln!(
            code,
            "crate::OperationDisposition {{ service: {:?}, operation: {:?}, kind: {:?}, message: {:?}, source: {:?} }},",
            entry.service, entry.operation, entry.kind, entry.message, entry.source
        )?;
    }
    code.push_str("];\n");
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(review: &Review, candidate: bool) -> Value {
        let mut services = BTreeMap::<String, Value>::new();
        for entry in &review.entries {
            let service = services.entry(entry.service.clone()).or_insert_with(|| json!({
                "service":entry.service, "document":{"Operations":[],"Actions":[],"Resources":[]}
            }));
            if candidate && entry.candidate_state == "removed" {
                continue;
            }
            let links = if candidate {
                vec![]
            } else {
                entry.authorized_actions.clone()
            };
            if let Some(rows) = service["document"]["Operations"].as_array_mut() {
                rows.push(json!({"Name":entry.operation,"AuthorizedActions":links}));
            }
        }
        for (category, scopes) in [
            ("Actions", &review.action_scopes),
            ("Resources", &review.resource_scopes),
        ] {
            for (key, shape) in scopes {
                if let Some((service, name)) = key.split_once(':') {
                    let service = services.entry(service.to_owned()).or_insert_with(|| json!({
                        "service":service,"document":{"Operations":[],"Actions":[],"Resources":[]}
                    }));
                    let mut row = shape.clone();
                    row["Name"] = json!(name);
                    if let Some(rows) = service["document"][category].as_array_mut() {
                        rows.push(row);
                    }
                }
            }
        }
        json!({"services":services.into_values().collect::<Vec<_>>()})
    }

    #[test]
    fn dispositions_reject_context_scope_and_transition_drift() -> BuildResult<()> {
        let review = load()?;
        let catalog = fixture(&review, false);
        validate_document(&catalog, &review)?;
        validate_document(&fixture(&review, true), &review)?;
        for entry in &review.entries {
            let mut changed = catalog.clone();
            let service = changed["services"]
                .as_array_mut()
                .ok_or("services")?
                .iter_mut()
                .find(|s| s["service"] == entry.service)
                .ok_or("service")?;
            let rows = service["document"]["Operations"]
                .as_array_mut()
                .ok_or("operations")?;
            rows.retain(|r| r["Name"] != entry.operation);
            let mut links = entry.authorized_actions.clone();
            links[0]["Context"] = json!({"unreviewed:Scope": ["strict"]});
            rows.push(json!({"Name": entry.operation, "AuthorizedActions": links}));
            assert!(validate_document(&changed, &review).is_err());
            let rows = changed["services"]
                .as_array_mut()
                .ok_or("services")?
                .iter_mut()
                .find(|s| s["service"] == entry.service)
                .ok_or("service")?["document"]["Operations"]
                .as_array_mut()
                .ok_or("operations")?;
            rows.retain(|r| r["Name"] != entry.operation);
            if entry.candidate_state == "removed" {
                rows.push(json!({"Name":entry.operation,"AuthorizedActions":[]}));
            }
            assert!(validate_document(&changed, &review).is_err());
        }
        for (category, scopes, field) in [
            ("Actions", &review.action_scopes, "Resources"),
            ("Resources", &review.resource_scopes, "ARNFormats"),
        ] {
            for key in scopes.keys() {
                let (service, name) = key.split_once(':').ok_or("key")?;
                let mut changed = catalog.clone();
                let service = changed["services"]
                    .as_array_mut()
                    .ok_or("services")?
                    .iter_mut()
                    .find(|s| s["service"] == service)
                    .ok_or("service")?;
                let row = service["document"][category]
                    .as_array_mut()
                    .ok_or("definitions")?
                    .iter_mut()
                    .find(|r| r["Name"] == name)
                    .ok_or("definition")?;
                row[field] = json!(["*"]);
                assert!(validate_document(&changed, &review).is_err(), "{key}");
            }
        }
        Ok(())
    }
}
