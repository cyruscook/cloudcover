//! Canonical raw authorization closure shared by preservation boundaries.
use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::BuildResult;

fn document<'a>(catalog: &'a Value, service: &str) -> BuildResult<Option<&'a Value>> {
    let mut found = catalog["services"]
        .as_array()
        .ok_or("missing services")?
        .iter()
        .filter(|entry| entry["service"] == service);
    let document = found.next().map(|entry| &entry["document"]);
    if found.next().is_some() {
        return Err("duplicate preservation service".into());
    }
    Ok(document)
}

fn definition<'a>(
    catalog: &'a Value,
    service: &str,
    category: &str,
    name: &str,
) -> BuildResult<Option<&'a Value>> {
    let Some(doc) = document(catalog, service)? else {
        return Ok(None);
    };
    let rows = match doc.get(category) {
        None => return Ok(None),
        Some(value) => value
            .as_array()
            .ok_or("invalid preservation definition table")?,
    };
    let mut found = rows.iter().filter(|row| row["Name"] == name);
    let value = found.next();
    if found.next().is_some() {
        return Err("duplicate preservation definition".into());
    }
    Ok(value)
}

// Explicitly canonicalize objects even if a consumer enables serde_json's
// preserve_order feature. Unknown fields and Context values remain evidence;
// absent and null are different. Only the SDK projection is excluded.
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), canonical(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
        value => value.clone(),
    }
}

pub(super) fn fingerprint(catalog: &Value, service: &str, name: &str) -> BuildResult<String> {
    let doc = document(catalog, service)?;
    let mode = match doc {
        None => "missing_service",
        Some(doc)
            if doc
                .get("Operations")
                .and_then(Value::as_array)
                .is_some_and(|v| !v.is_empty()) =>
        {
            "explicit_operations"
        }
        Some(_) => "fallback_actions",
    };
    let mut operation = definition(catalog, service, "Operations", name)?.cloned();
    let mut carriers = BTreeMap::new();
    if let Some(operation) = operation.as_mut() {
        operation
            .as_object_mut()
            .ok_or("invalid preservation operation")?
            .remove("SDK");
        if let Some(links) = operation.get_mut("AuthorizedActions") {
            let links = links.as_array_mut().ok_or("invalid preservation links")?;
            links.sort_by_cached_key(|value| canonical(value).to_string());
            for link in links {
                let service = link["Service"].as_str().ok_or("invalid carrier service")?;
                let name = link["Name"].as_str().ok_or("invalid carrier name")?;
                let action = definition(catalog, service, "Actions", name)?;
                let mut resources = BTreeMap::new();
                let mut keys = BTreeSet::new();
                if let Some(context) = link.get("Context").and_then(Value::as_object) {
                    keys.extend(context.keys().map(String::as_str));
                }
                if let Some(action) = action {
                    add_keys(&mut keys, action.get("ActionConditionKeys"))?;
                    if let Some(refs) = action.get("Resources") {
                        for reference in refs.as_array().ok_or("invalid carrier resources")? {
                            let name = reference["Name"].as_str().ok_or("invalid resource name")?;
                            let resource = definition(catalog, service, "Resources", name)?;
                            add_keys(&mut keys, reference.get("ConditionKeys"))?;
                            if let Some(resource) = resource {
                                add_keys(&mut keys, resource.get("ConditionKeys"))?;
                            }
                            resources.insert(name, resource);
                        }
                    }
                }
                let condition_keys = keys
                    .into_iter()
                    .map(|key| {
                        definition(catalog, service, "ConditionKeys", key).map(|value| (key, value))
                    })
                    .collect::<BuildResult<BTreeMap<_, _>>>()?;
                carriers.insert(
                    format!("{service}:{name}"),
                    json!({
                        "action": action, "resources": resources, "condition_keys": condition_keys,
                    }),
                );
            }
        }
    }
    let evidence = canonical(&json!({
        "service": service, "operation_name": name, "mode": mode,
        "operation": operation, "carriers": carriers,
    }));
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&evidence)?)
    ))
}

fn add_keys<'a>(keys: &mut BTreeSet<&'a str>, value: Option<&'a Value>) -> BuildResult<()> {
    if let Some(value) = value {
        for key in value.as_array().ok_or("invalid condition key list")? {
            keys.insert(key.as_str().ok_or("invalid condition key")?);
        }
    }
    Ok(())
}
