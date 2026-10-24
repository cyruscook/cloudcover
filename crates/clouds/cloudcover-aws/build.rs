use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    fmt::{self, Write},
    fs,
    path::{Path, PathBuf},
};

const CACHE_FILE_NAME: &str = "actions-v2.rs";

type BuildResult<T> = Result<T, Box<dyn Error>>;

#[derive(serde::Deserialize)]
struct ServiceIndexEntry {
    service: String,
    url: String,
    #[serde(rename = "modified")]
    _modified: u64,
}

#[derive(serde::Deserialize)]
struct ServiceDocument {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Actions")]
    actions: Vec<ServiceAction>,
    #[serde(rename = "Resources", default)]
    resources: Vec<ServiceResource>,
    #[serde(rename = "Operations", default)]
    operations: Vec<ServiceOperation>,
    #[serde(rename = "ConditionKeys", default)]
    condition_keys: Vec<ServiceConditionKey>,
}

#[derive(serde::Deserialize)]
struct ServiceConditionKey {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Types", default)]
    types: Vec<String>,
}

#[derive(serde::Deserialize)]
struct ServiceAction {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Resources", default)]
    resources: Vec<ActionResource>,
    #[serde(rename = "ActionConditionKeys", default)]
    condition_keys: Vec<String>,
}

#[derive(serde::Deserialize)]
struct ActionResource {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "ConditionKeys", default)]
    condition_keys: Vec<String>,
}

#[derive(serde::Deserialize)]
struct ServiceResource {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "ARNFormats", default)]
    arn_formats: Vec<String>,
    #[serde(rename = "ConditionKeys", default)]
    condition_keys: Vec<String>,
}

#[derive(serde::Deserialize)]
struct ServiceOperation {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "AuthorizedActions", default)]
    authorized_actions: Vec<AuthorizedAction>,
    #[serde(rename = "SDK", default)]
    sdk: Vec<SdkEntry>,
}

#[derive(serde::Deserialize)]
struct AuthorizedAction {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Service")]
    service: String,
}

#[derive(serde::Deserialize)]
struct SdkEntry {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Method")]
    method: String,
    #[serde(rename = "Package")]
    package: String,
}

struct ActionRow {
    service: String,
    name: String,
    permission: String,
    resource_types: Vec<String>,
    resource_templates: Vec<String>,
    has_complete_resource_templates: bool,
    resources: Vec<ResourceRow>,
    context_keys: Vec<ContextKeyRow>,
}

struct ResourceRow {
    name: String,
    arn_formats: Vec<String>,
}

struct ContextKeyRow {
    name: String,
    types: Vec<String>,
    permission: bool,
    permission_resources: Vec<String>,
    resource_types: Vec<String>,
}

struct OperationRow {
    service: String,
    name: String,
    authorized_actions: Vec<(String, String)>,
}

struct SdkMappingRow {
    sdk_package: String,
    sdk_name: String,
    sdk_method: String,
    api_service: String,
    api_name: String,
}

struct GeneratedRows {
    actions: Vec<ActionRow>,
    operations: Vec<OperationRow>,
    sdk_mappings: Vec<SdkMappingRow>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    schema_version: u32,
    services: Vec<CatalogService>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogService {
    service: String,
    url: String,
    modified: u64,
    source_sha256: String,
    document: ServiceDocument,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    source: String,
    retrieved_at: String,
    index_sha256: String,
    catalog_sha256: String,
    refresh_script_sha256: String,
    extractor_revision: String,
    service_count: usize,
}

fn validate_manifest(manifest: &Manifest, contents: &[u8]) -> BuildResult<()> {
    let digest = format!("{:x}", Sha256::digest(contents));
    if manifest.schema_version != 1
        || manifest.catalog_sha256 != digest
        || manifest.source != "https://servicereference.us-east-1.amazonaws.com/"
        || manifest.retrieved_at.is_empty()
        || manifest.extractor_revision.is_empty()
        || [&manifest.index_sha256, &manifest.refresh_script_sha256]
            .iter()
            .any(|hash| hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(
            "SAR manifest schema, provenance or checksum mismatch; run tools/refresh_sar.py".into(),
        );
    }
    Ok(())
}

fn main() -> BuildResult<()> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=data/sar/catalog.json");
    println!("cargo:rerun-if-changed=data/sar/manifest.json");
    let mut rows = collect_rows()?;
    let action_keys = sort_and_validate_action_rows(&mut rows.actions)?;
    sort_and_validate_operation_rows(&mut rows.operations, &action_keys)?;
    sort_and_validate_sdk_mapping_rows(&mut rows.sdk_mappings)?;
    write_generated_file(&generate_code(&rows)?)
}

fn collect_rows() -> BuildResult<GeneratedRows> {
    let contents = fs::read("data/sar/catalog.json")?;
    let manifest: Manifest = serde_json::from_slice(&fs::read("data/sar/manifest.json")?)?;
    validate_manifest(&manifest, &contents)?;
    let catalog: Catalog = serde_json::from_slice(&contents)?;
    if catalog.schema_version != 1
        || catalog.services.is_empty()
        || manifest.service_count != catalog.services.len()
    {
        return Err("invalid SAR catalog schema or service count".into());
    }
    let mut previous = None;
    for entry in &catalog.services {
        if previous.is_some_and(|name: &str| name >= entry.service.as_str())
            || entry.source_sha256.len() != 64
            || !entry.source_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("SAR services must be unique, sorted, and have source checksums".into());
        }
        previous = Some(entry.service.as_str());
    }
    let mut actions = Vec::new();
    let mut operations = Vec::new();
    let mut sdk_mappings = Vec::new();

    for entry in catalog.services {
        let document = entry.document;
        let entry = ServiceIndexEntry {
            service: entry.service,
            url: entry.url,
            _modified: entry.modified,
        };
        validate_service_name(&entry, &document)?;

        let service = entry.service;
        let ServiceDocument {
            name: _,
            actions: document_actions,
            resources,
            operations: document_operations,
            condition_keys,
        } = document;

        actions.extend(build_action_rows(
            &service,
            document_actions,
            resources,
            condition_keys,
        ));
        operations.extend(build_operation_rows(
            &service,
            &actions,
            document_operations,
            &mut sdk_mappings,
        ));
    }

    Ok(GeneratedRows {
        actions,
        operations,
        sdk_mappings,
    })
}

fn validate_service_name(entry: &ServiceIndexEntry, document: &ServiceDocument) -> BuildResult<()> {
    if document.name == entry.service.as_str() {
        return Ok(());
    }

    Err(format!(
        "service name mismatch for {url}: index has {index_name:?}, document has {document_name:?}",
        url = entry.url,
        index_name = entry.service,
        document_name = document.name,
    )
    .into())
}

fn build_action_rows(
    service: &str,
    actions: Vec<ServiceAction>,
    resources: Vec<ServiceResource>,
    condition_keys: Vec<ServiceConditionKey>,
) -> Vec<ActionRow> {
    let resources = resources
        .into_iter()
        .map(|resource| (resource.name.clone(), resource))
        .collect::<BTreeMap<_, _>>();
    let key_types = condition_keys
        .into_iter()
        .map(|key| (key.name, key.types))
        .collect::<BTreeMap<_, _>>();
    actions
        .into_iter()
        .map(|action| build_action_row(service, action, &resources, &key_types))
        .collect()
}

fn build_action_row(
    service: &str,
    action: ServiceAction,
    resource_definitions: &BTreeMap<String, ServiceResource>,
    key_types: &BTreeMap<String, Vec<String>>,
) -> ActionRow {
    let permission = format!("{service}:{name}", name = action.name);
    let mut resources = BTreeMap::new();
    let mut keys = BTreeMap::new();
    for name in action.condition_keys {
        context_key(&mut keys, &name, key_types).permission = true;
    }
    let mut has_complete_resource_templates = true;
    for resource in action.resources {
        let definition = resource_definitions.get(&resource.name);
        let arn_formats = definition.map_or_else(Vec::new, |value| value.arn_formats.clone());
        if definition.is_none() {
            has_complete_resource_templates = false;
        }
        for name in resource.condition_keys {
            context_key(&mut keys, &name, key_types)
                .permission_resources
                .push(resource.name.clone());
        }
        if let Some(definition) = definition {
            for name in &definition.condition_keys {
                context_key(&mut keys, name, key_types)
                    .resource_types
                    .push(resource.name.clone());
            }
        }
        resources.insert(resource.name, arn_formats);
    }
    let resource_types = resources.keys().cloned().collect();
    let resource_templates = resources
        .values()
        .flatten()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let resources = resources
        .into_iter()
        .map(|(name, arn_formats)| ResourceRow { name, arn_formats })
        .collect();
    let context_keys = keys
        .into_values()
        .map(|mut key| {
            key.permission_resources.sort_unstable();
            key.permission_resources.dedup();
            key.resource_types.sort_unstable();
            key.resource_types.dedup();
            key
        })
        .collect();
    ActionRow {
        service: service.to_owned(),
        name: action.name,
        permission,
        resource_types,
        resource_templates,
        has_complete_resource_templates,
        resources,
        context_keys,
    }
}

fn context_key<'a>(
    keys: &'a mut BTreeMap<String, ContextKeyRow>,
    name: &str,
    key_types: &BTreeMap<String, Vec<String>>,
) -> &'a mut ContextKeyRow {
    keys.entry(name.to_owned())
        .or_insert_with(|| ContextKeyRow {
            name: name.to_owned(),
            types: key_types.get(name).cloned().unwrap_or_default(),
            permission: false,
            permission_resources: Vec::new(),
            resource_types: Vec::new(),
        })
}

fn build_operation_rows(
    service: &str,
    action_rows: &[ActionRow],
    operations: Vec<ServiceOperation>,
    sdk_mapping_rows: &mut Vec<SdkMappingRow>,
) -> Vec<OperationRow> {
    if operations.is_empty() {
        return action_rows
            .iter()
            .filter(|row| row.service == service)
            .map(|row| OperationRow {
                service: service.to_owned(),
                name: row.name.clone(),
                authorized_actions: vec![(service.to_owned(), row.name.clone())],
            })
            .collect();
    }

    operations
        .into_iter()
        .map(|operation| build_operation_row(service, operation, sdk_mapping_rows))
        .collect()
}

fn build_operation_row(
    service: &str,
    operation: ServiceOperation,
    sdk_mapping_rows: &mut Vec<SdkMappingRow>,
) -> OperationRow {
    let ServiceOperation {
        name,
        authorized_actions,
        sdk,
    } = operation;
    let mut authorized_actions = authorized_actions
        .into_iter()
        .map(|authorized_action| (authorized_action.service, authorized_action.name))
        .collect::<Vec<_>>();

    authorized_actions.sort_unstable();
    authorized_actions.dedup();

    for sdk_entry in sdk {
        if sdk_entry.package == "Boto3" {
            sdk_mapping_rows.push(SdkMappingRow {
                sdk_package: "boto3".to_owned(),
                sdk_name: sdk_entry.name,
                sdk_method: sdk_entry.method,
                api_service: service.to_owned(),
                api_name: name.clone(),
            });
        }
    }

    OperationRow {
        service: service.to_owned(),
        name,
        authorized_actions,
    }
}

fn sort_and_validate_action_rows(
    action_rows: &mut [ActionRow],
) -> BuildResult<BTreeSet<(String, String)>> {
    action_rows.sort_unstable_by(|left, right| {
        (
            &left.service,
            &left.name,
            &left.resource_types,
            &left.resource_templates,
            left.has_complete_resource_templates,
        )
            .cmp(&(
                &right.service,
                &right.name,
                &right.resource_types,
                &right.resource_templates,
                right.has_complete_resource_templates,
            ))
    });

    for pair in action_rows.windows(2) {
        let [left, right] = pair else {
            continue;
        };

        if left.service == right.service && left.name == right.name {
            return Err(format!(
                "duplicate action definition for {service}:{name}",
                service = left.service,
                name = left.name
            )
            .into());
        }
    }

    Ok(action_rows
        .iter()
        .map(|row| (row.service.clone(), row.name.clone()))
        .collect())
}

fn sort_and_validate_operation_rows(
    operation_rows: &mut [OperationRow],
    action_keys: &BTreeSet<(String, String)>,
) -> BuildResult<()> {
    operation_rows.sort_unstable_by(|left, right| {
        (&left.service, &left.name, &left.authorized_actions).cmp(&(
            &right.service,
            &right.name,
            &right.authorized_actions,
        ))
    });

    for pair in operation_rows.windows(2) {
        let [left, right] = pair else {
            continue;
        };

        if left.service == right.service && left.name == right.name {
            return Err(format!(
                "duplicate operation definition for {service}:{name}",
                service = left.service,
                name = left.name
            )
            .into());
        }
    }

    for row in operation_rows {
        for (authorized_service, authorized_name) in &row.authorized_actions {
            if !action_keys.contains(&(authorized_service.clone(), authorized_name.clone())) {
                return Err(format!(
                    "operation {operation_service}:{operation_name} references missing authorized action {authorized_service}:{authorized_name}",
                    operation_service = row.service,
                    operation_name = row.name,
                    authorized_service = authorized_service,
                    authorized_name = authorized_name,
                )
                .into());
            }
        }
    }

    Ok(())
}

fn sort_and_validate_sdk_mapping_rows(sdk_mapping_rows: &mut [SdkMappingRow]) -> BuildResult<()> {
    sdk_mapping_rows.sort_unstable_by(|left, right| {
        (
            &left.sdk_package,
            &left.sdk_name,
            &left.sdk_method,
            &left.api_service,
            &left.api_name,
        )
            .cmp(&(
                &right.sdk_package,
                &right.sdk_name,
                &right.sdk_method,
                &right.api_service,
                &right.api_name,
            ))
    });

    for pair in sdk_mapping_rows.windows(2) {
        let [left, right] = pair else {
            continue;
        };

        if left.sdk_package == right.sdk_package
            && left.sdk_name == right.sdk_name
            && left.sdk_method == right.sdk_method
        {
            return Err(format!(
                "duplicate SDK mapping for {package}:{name}.{method}: {left_service}:{left_name} and {right_service}:{right_name}",
                package = left.sdk_package,
                name = left.sdk_name,
                method = left.sdk_method,
                left_service = left.api_service,
                left_name = left.api_name,
                right_service = right.api_service,
                right_name = right.api_name,
            )
            .into());
        }
    }

    Ok(())
}

fn generate_code(rows: &GeneratedRows) -> Result<String, fmt::Error> {
    let mut generated = String::from(
        "pub(super) const ACTIONS: &[crate::model::AwsAction] = &[
",
    );

    write_actions(&mut generated, &rows.actions)?;
    write_permission_details(&mut generated, &rows.actions)?;
    write_operations(&mut generated, &rows.operations)?;
    write_sdk_method_mappings(&mut generated, &rows.sdk_mappings)?;

    Ok(generated)
}

fn write_actions(generated: &mut String, action_rows: &[ActionRow]) -> Result<(), fmt::Error> {
    for row in action_rows {
        writeln!(
            generated,
            "    crate::model::AwsAction {{ service: {service:?}, name: {name:?}, permission: {permission:?}, resource_types: &{resource_types:?}, resource_templates: &{resource_templates:?}, has_complete_resource_templates: {has_complete_resource_templates} }},",
            service = row.service,
            name = row.name,
            permission = row.permission,
            resource_types = row.resource_types,
            resource_templates = row.resource_templates,
            has_complete_resource_templates = row.has_complete_resource_templates
        )?;
    }
    generated.push_str(
        "];
",
    );
    Ok(())
}

fn write_permission_details(generated: &mut String, rows: &[ActionRow]) -> Result<(), fmt::Error> {
    generated.push_str("pub(super) const IAM_PERMISSIONS: &[crate::model::IamPermission] = &[\n");
    for row in rows {
        writeln!(
            generated,
            "crate::model::IamPermission {{ permission: {:?}, resources: &[",
            row.permission
        )?;
        for resource in &row.resources {
            writeln!(
                generated,
                "crate::model::IamResource {{ name: {:?}, arn_formats: &{:?} }},",
                resource.name, resource.arn_formats
            )?;
        }
        generated.push_str("], context_keys: &[\n");
        for key in &row.context_keys {
            writeln!(
                generated,
                "crate::model::IamContextKey {{ name: {:?}, types: &{:?}, permission: {}, permission_resources: &{:?}, resource_types: &{:?} }},",
                key.name, key.types, key.permission, key.permission_resources, key.resource_types
            )?;
        }
        generated.push_str("] },\n");
    }
    generated.push_str("];\n");
    Ok(())
}

fn write_operations(
    generated: &mut String,
    operation_rows: &[OperationRow],
) -> Result<(), fmt::Error> {
    generated.push_str(
        "pub(super) const OPERATIONS: &[crate::model::AwsOperation] = &[
",
    );
    for row in operation_rows {
        writeln!(
            generated,
            "    crate::model::AwsOperation {{ service: {service:?}, name: {name:?}, authorized_actions: &{authorized_actions} }},",
            service = row.service,
            name = row.name,
            authorized_actions = format_authorized_actions(&row.authorized_actions),
        )?;
    }
    generated.push_str(
        "];
",
    );
    Ok(())
}

fn write_sdk_method_mappings(
    generated: &mut String,
    sdk_mapping_rows: &[SdkMappingRow],
) -> Result<(), fmt::Error> {
    generated.push_str(
        "pub(super) const SDK_METHOD_MAPPINGS: &[crate::model::AwsSdkMethodMapping] = &[
",
    );
    for row in sdk_mapping_rows {
        writeln!(
            generated,
            "    crate::model::AwsSdkMethodMapping {{ sdk_package: {sdk_package:?}, sdk_name: {sdk_name:?}, sdk_method: {sdk_method:?}, api_service: {api_service:?}, api_name: {api_name:?} }},",
            sdk_package = row.sdk_package,
            sdk_name = row.sdk_name,
            sdk_method = row.sdk_method,
            api_service = row.api_service,
            api_name = row.api_name,
        )?;
    }
    generated.push_str(
        "];
",
    );
    Ok(())
}

fn write_generated_file(generated: &str) -> BuildResult<()> {
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let output_path = out_dir.join(CACHE_FILE_NAME);
    write_if_changed(&output_path, generated)
}

fn write_if_changed(path: &Path, contents: &str) -> BuildResult<()> {
    match fs::read_to_string(path) {
        Ok(existing) if existing == contents => Ok(()),
        Ok(_) | Err(_) => {
            fs::write(path, contents)?;
            Ok(())
        }
    }
}

fn format_authorized_actions(authorized_actions: &[(String, String)]) -> String {
    if authorized_actions.is_empty() {
        return "[]".to_owned();
    }

    let entries = authorized_actions
        .iter()
        .map(|(service, name)| {
            format!("crate::model::AwsApiMethodRef {{ service: {service:?}, name: {name:?} }}")
        })
        .collect::<Vec<_>>();

    format!("[ {} ]", entries.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_corruption_and_unsupported_schema() -> BuildResult<()> {
        let mut manifest: Manifest = serde_json::from_slice(&fs::read("data/sar/manifest.json")?)?;
        assert!(validate_manifest(&manifest, b"corrupted catalog").is_err());
        manifest.schema_version += 1;
        assert!(validate_manifest(&manifest, &fs::read("data/sar/catalog.json")?).is_err());
        Ok(())
    }

    #[test]
    fn pinned_catalog_generates_identical_bytes() -> BuildResult<()> {
        fn generate() -> BuildResult<String> {
            let mut rows = collect_rows()?;
            let keys = sort_and_validate_action_rows(&mut rows.actions)?;
            sort_and_validate_operation_rows(&mut rows.operations, &keys)?;
            sort_and_validate_sdk_mapping_rows(&mut rows.sdk_mappings)?;
            generate_code(&rows).map_err(Into::into)
        }
        assert_eq!(generate()?, generate()?);
        Ok(())
    }
}
