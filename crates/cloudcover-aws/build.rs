use serde::de::DeserializeOwned;
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt::{self, Write},
    fs,
    path::PathBuf,
    time::Duration,
};
use ureq::Agent;

const SERVICE_REFERENCE_URL: &str = "https://servicereference.us-east-1.amazonaws.com/";

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
}

#[derive(serde::Deserialize)]
struct ServiceAction {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Resources", default)]
    resources: Vec<ActionResource>,
}

#[derive(serde::Deserialize)]
struct ActionResource {
    #[serde(rename = "Name")]
    name: String,
}

#[derive(serde::Deserialize)]
struct ServiceResource {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "ARNFormats", default)]
    arn_formats: Vec<String>,
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

fn main() -> BuildResult<()> {
    println!("cargo:rerun-if-env-changed=CLOUDCOVER_AWS_SAR_REFRESH");

    let agent = build_agent();
    let mut rows = collect_rows(&agent)?;
    let action_keys = sort_and_validate_action_rows(&mut rows.actions)?;

    sort_and_validate_operation_rows(&mut rows.operations, &action_keys)?;
    sort_and_validate_sdk_mapping_rows(&mut rows.sdk_mappings)?;

    let generated = generate_code(&rows)?;
    write_generated_file(&generated)
}

fn build_agent() -> Agent {
    let config = Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .build();
    Agent::new_with_config(config)
}

fn collect_rows(agent: &Agent) -> BuildResult<GeneratedRows> {
    let index: Vec<ServiceIndexEntry> = fetch_json(agent, SERVICE_REFERENCE_URL)?;
    let mut actions = Vec::new();
    let mut operations = Vec::new();
    let mut sdk_mappings = Vec::new();

    for entry in index {
        let document: ServiceDocument = fetch_json(agent, &entry.url)?;
        validate_service_name(&entry, &document)?;

        let service = entry.service;
        let ServiceDocument {
            name: _,
            actions: document_actions,
            resources,
            operations: document_operations,
        } = document;

        actions.extend(build_action_rows(&service, document_actions, resources));
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
) -> Vec<ActionRow> {
    let resource_templates_by_name = resources
        .into_iter()
        .map(|resource| (resource.name, resource.arn_formats))
        .collect::<BTreeMap<_, _>>();

    actions
        .into_iter()
        .map(|action| build_action_row(service, action, &resource_templates_by_name))
        .collect()
}

fn build_action_row(
    service: &str,
    action: ServiceAction,
    resource_templates_by_name: &BTreeMap<String, Vec<String>>,
) -> ActionRow {
    let permission = format!("{service}:{name}", name = action.name);
    let mut resource_types = action
        .resources
        .into_iter()
        .map(|resource| resource.name)
        .collect::<Vec<_>>();
    let mut resource_templates = Vec::new();
    let mut has_complete_resource_templates = true;

    resource_types.sort_unstable();
    resource_types.dedup();

    for resource_type in &resource_types {
        let Some(arn_formats) = resource_templates_by_name.get(resource_type) else {
            has_complete_resource_templates = false;
            continue;
        };
        resource_templates.extend(arn_formats.iter().cloned());
    }

    resource_templates.sort_unstable();
    resource_templates.dedup();

    ActionRow {
        service: service.to_owned(),
        name: action.name,
        permission,
        resource_types,
        resource_templates,
        has_complete_resource_templates,
    }
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
    let mut generated = String::from("pub(super) const ACTIONS: &[super::AwsAction] = &[\n");

    write_actions(&mut generated, &rows.actions)?;
    write_operations(&mut generated, &rows.operations)?;
    write_sdk_method_mappings(&mut generated, &rows.sdk_mappings)?;

    Ok(generated)
}

fn write_actions(generated: &mut String, action_rows: &[ActionRow]) -> Result<(), fmt::Error> {
    for row in action_rows {
        writeln!(
            generated,
            "    super::AwsAction {{ service: {service:?}, name: {name:?}, permission: {permission:?}, resource_types: &{resource_types:?}, resource_templates: &{resource_templates:?}, has_complete_resource_templates: {has_complete_resource_templates} }},",
            service = row.service,
            name = row.name,
            permission = row.permission,
            resource_types = row.resource_types,
            resource_templates = row.resource_templates,
            has_complete_resource_templates = row.has_complete_resource_templates
        )?;
    }
    generated.push_str("];\n");
    Ok(())
}

fn write_operations(
    generated: &mut String,
    operation_rows: &[OperationRow],
) -> Result<(), fmt::Error> {
    generated.push_str("pub(super) const OPERATIONS: &[super::AwsOperation] = &[\n");
    for row in operation_rows {
        writeln!(
            generated,
            "    super::AwsOperation {{ service: {service:?}, name: {name:?}, authorized_actions: &{authorized_actions} }},",
            service = row.service,
            name = row.name,
            authorized_actions = format_authorized_actions(&row.authorized_actions),
        )?;
    }
    generated.push_str("];\n");
    Ok(())
}

fn write_sdk_method_mappings(
    generated: &mut String,
    sdk_mapping_rows: &[SdkMappingRow],
) -> Result<(), fmt::Error> {
    generated
        .push_str("pub(super) const SDK_METHOD_MAPPINGS: &[super::AwsSdkMethodMapping] = &[\n");
    for row in sdk_mapping_rows {
        writeln!(
            generated,
            "    super::AwsSdkMethodMapping {{ sdk_package: {sdk_package:?}, sdk_name: {sdk_name:?}, sdk_method: {sdk_method:?}, api_service: {api_service:?}, api_name: {api_name:?} }},",
            sdk_package = row.sdk_package,
            sdk_name = row.sdk_name,
            sdk_method = row.sdk_method,
            api_service = row.api_service,
            api_name = row.api_name,
        )?;
    }
    generated.push_str("];\n");
    Ok(())
}

fn write_generated_file(generated: &str) -> BuildResult<()> {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let output_path = out_dir.join("actions.rs");
    fs::write(&output_path, generated).map_err(|error| {
        format!(
            "failed to write generated actions to {}: {error}",
            output_path.display()
        )
    })?;
    Ok(())
}

fn format_authorized_actions(authorized_actions: &[(String, String)]) -> String {
    if authorized_actions.is_empty() {
        return "[]".to_owned();
    }

    let entries = authorized_actions
        .iter()
        .map(|(service, name)| {
            format!("super::AwsApiMethodRef {{ service: {service:?}, name: {name:?} }}")
        })
        .collect::<Vec<_>>();

    format!("[ {} ]", entries.join(", "))
}

fn fetch_json<T>(agent: &Agent, url: &str) -> BuildResult<T>
where
    T: DeserializeOwned,
{
    let mut response = agent
        .get(url)
        .call()
        .map_err(|error| format!("failed to fetch {url}: {error}"))?;
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|error| format!("failed to read response body from {url}: {error}"))?;

    serde_json::from_str(&body)
        .map_err(|error| format!("failed to parse JSON from {url}: {error}"))
        .map_err(Into::into)
}
