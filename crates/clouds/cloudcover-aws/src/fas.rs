//! Primary-source, operation-level supplement to AWS's FAS-excluding reference.
use std::{collections::BTreeSet, sync::LazyLock};

use cloudcover_core::ApiMethod;
use serde::{Deserialize, Serialize};

use crate::{AwsError, policy};

/// The checked-in source document, also distributed with the site catalog.
pub const FAS_DOCUMENT: &str = include_str!("../data/fas.json");

/// Evidence for an operation dependency, taken from direct AWS documentation.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FasSource {
    pub url: String,
    pub evidence: String,
}

/// A possible caller-authorized downstream call. Applicability is descriptive:
/// current analyzers do not evaluate request arguments or live configuration.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FasRule {
    pub id: String,
    pub service: String,
    pub operations: Vec<String>,
    pub downstream_service: String,
    pub downstream_operations: Vec<String>,
    pub applicability: String,
    pub resource_role: String,
    pub key_placeholder: Option<String>,
    pub region_placeholder: String,
    pub via_service: Option<String>,
    pub sources: Vec<FasSource>,
    pub reviewed: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoDependencies {
    service: String,
    operations: Vec<String>,
    sources: Vec<FasSource>,
    reviewed: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    schema_version: u8,
    description: String,
    rules: Vec<FasRule>,
    reviewed_no_dependencies: Vec<NoDependencies>,
}

/// Coverage is scoped to the checked-in review, not all AWS configurations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FasCoverage {
    DocumentedDependencies,
    ReviewedNoDependencies,
    Unknown,
}

/// A dependency retains its parent path, including transitive applicability.
#[derive(Debug, Serialize)]
pub struct FasDependency {
    pub rule: &'static FasRule,
    pub path: Vec<&'static str>,
    pub iam_permissions: Vec<&'static str>,
}

static CATALOG: LazyLock<Result<Catalog, String>> = LazyLock::new(|| {
    let catalog: Catalog = serde_json::from_str(FAS_DOCUMENT).map_err(|e| e.to_string())?;
    validate_catalog(&catalog)?;
    Ok(catalog)
});

fn catalog() -> Result<&'static Catalog, AwsError> {
    CATALOG
        .as_ref()
        .map_err(|message| AwsError::InvalidFasCatalog {
            message: message.clone(),
        })
}

pub(crate) fn coverage(method: &ApiMethod) -> Result<FasCoverage, AwsError> {
    policy::find_operation(method)?;
    let catalog = catalog()?;
    if catalog.rules.iter().any(|rule| applies(rule, method)) {
        Ok(FasCoverage::DocumentedDependencies)
    } else if catalog.reviewed_no_dependencies.iter().any(|entry| {
        entry.service == method.service() && entry.operations.iter().any(|op| op == method.name())
    }) {
        Ok(FasCoverage::ReviewedNoDependencies)
    } else {
        Ok(FasCoverage::Unknown)
    }
}

pub(crate) fn dependencies(method: &ApiMethod) -> Result<Vec<FasDependency>, AwsError> {
    policy::find_operation(method)?;
    let mut output = Vec::new();
    expand(
        catalog()?,
        method,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut output,
    )?;
    Ok(output)
}

fn applies(rule: &FasRule, method: &ApiMethod) -> bool {
    rule.service == method.service() && rule.operations.iter().any(|op| op == method.name())
}

fn expand(
    catalog: &'static Catalog,
    method: &ApiMethod,
    visiting: &mut Vec<ApiMethod>,
    path: &mut Vec<&'static str>,
    output: &mut Vec<FasDependency>,
) -> Result<(), AwsError> {
    if visiting.contains(method) {
        return Err(AwsError::InvalidFasCatalog {
            message: format!("dependency cycle at {}:{}", method.service(), method.name()),
        });
    }
    visiting.push(method.clone());
    for rule in catalog.rules.iter().filter(|rule| applies(rule, method)) {
        path.push(&rule.id);
        let mut permissions = BTreeSet::new();
        for operation in &rule.downstream_operations {
            let downstream = ApiMethod::new(&rule.downstream_service, operation);
            permissions.extend(
                policy::resolve_direct_actions(&downstream)?
                    .iter()
                    .map(|a| a.permission),
            );
            expand(catalog, &downstream, visiting, path, output)?;
        }
        output.push(FasDependency {
            rule,
            path: path.clone(),
            iam_permissions: permissions.into_iter().collect(),
        });
        path.pop();
    }
    visiting.pop();
    Ok(())
}

fn validate_catalog(catalog: &Catalog) -> Result<(), String> {
    if catalog.schema_version != 1 || catalog.description.is_empty() {
        return Err("unsupported or empty FAS document".to_owned());
    }
    let mut ids = BTreeSet::new();
    for rule in &catalog.rules {
        if !ids.insert(&rule.id)
            || rule.id.is_empty()
            || rule.applicability.is_empty()
            || rule.resource_role.is_empty()
            || rule.downstream_operations.is_empty()
            || rule.region_placeholder.is_empty()
        {
            return Err(format!("incomplete or duplicate rule {}", rule.id));
        }
        validate_sources(&rule.sources, &rule.reviewed)?;
        validate_operations(&rule.service, &rule.operations)?;
        validate_operations(&rule.downstream_service, &rule.downstream_operations)?;
        for operation in &rule.downstream_operations {
            let method = ApiMethod::new(&rule.downstream_service, operation);
            let actions = match policy::resolve_direct_actions(&method) {
                Ok(actions) => actions,
                // The operation exists, but its upstream authorization mapping
                // is incomplete. Reject requests that expand this dependency;
                // do not invalidate unrelated operations' FAS metadata.
                Err(AwsError::UnresolvedAuthorization { .. }) => continue,
                Err(error) => return Err(error.to_string()),
            };
            if actions.is_empty() {
                return Err(format!(
                    "{} references an actionless downstream operation",
                    rule.id
                ));
            }
            if rule.via_service.is_some() && actions.iter().any(|action| action.service != "kms") {
                return Err(format!(
                    "{} uses kms:ViaService on a non-KMS action",
                    rule.id
                ));
            }
        }
        if rule.downstream_service == "kms"
            && (rule.key_placeholder.as_ref().is_none_or(String::is_empty)
                || rule.via_service.is_none())
        {
            return Err(format!("{} omits KMS resource/condition context", rule.id));
        }
    }
    for entry in &catalog.reviewed_no_dependencies {
        validate_sources(&entry.sources, &entry.reviewed)?;
        validate_operations(&entry.service, &entry.operations)?;
        if entry.operations.iter().any(|op| {
            catalog
                .rules
                .iter()
                .any(|rule| applies(rule, &ApiMethod::new(&entry.service, op)))
        }) {
            return Err("conflicting FAS coverage entries".to_owned());
        }
    }
    // Validate every path before serving any policy, including disconnected cycles.
    for rule in &catalog.rules {
        for operation in &rule.operations {
            validate_acyclic(
                catalog,
                &ApiMethod::new(&rule.service, operation),
                &mut Vec::new(),
            )?;
        }
    }
    Ok(())
}

fn validate_operations(service: &str, operations: &[String]) -> Result<(), String> {
    if operations.is_empty() {
        return Err("empty FAS operation list".to_owned());
    }
    let mut seen = BTreeSet::new();
    for operation in operations {
        if !seen.insert(operation) {
            return Err(format!("duplicate operation {service}:{operation}"));
        }
        policy::find_operation(&ApiMethod::new(service, operation)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn validate_sources(sources: &[FasSource], reviewed: &str) -> Result<(), String> {
    if sources.is_empty()
        || reviewed.len() != 10
        || sources.iter().any(|source| {
            !source.url.starts_with("https://docs.aws.amazon.com/") || source.evidence.is_empty()
        })
    {
        return Err("FAS mappings require direct AWS evidence and a review date".to_owned());
    }
    Ok(())
}

fn validate_acyclic(
    catalog: &Catalog,
    method: &ApiMethod,
    visiting: &mut Vec<ApiMethod>,
) -> Result<(), String> {
    if visiting.contains(method) {
        return Err(format!(
            "dependency cycle at {}:{}",
            method.service(),
            method.name()
        ));
    }
    visiting.push(method.clone());
    for rule in catalog.rules.iter().filter(|rule| applies(rule, method)) {
        for operation in &rule.downstream_operations {
            validate_acyclic(
                catalog,
                &ApiMethod::new(&rule.downstream_service, operation),
                visiting,
            )?;
        }
    }
    visiting.pop();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> Result<Catalog, Box<dyn std::error::Error>> {
        Ok(serde_json::from_str(FAS_DOCUMENT)?)
    }

    #[test]
    fn unresolved_downstream_authorization_is_rejected_only_when_requested()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut catalog = document()?;
        catalog
            .rules
            .retain(|rule| rule.id == "ssm-secrets-reference");
        catalog.reviewed_no_dependencies.clear();
        let rule = catalog.rules.first_mut().ok_or("missing test rule")?;
        rule.downstream_service = "s3".to_owned();
        rule.downstream_operations = vec!["RenameObject".to_owned()];
        rule.via_service = None;
        rule.key_placeholder = None;
        let catalog: &'static Catalog = Box::leak(Box::new(catalog));
        validate_catalog(catalog)?;
        assert!(matches!(
            expand(
                catalog,
                &ApiMethod::new("ssm", "GetParameter"),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
            ),
            Err(AwsError::UnresolvedAuthorization { service, name })
                if service == "s3" && name == "RenameObject"
        ));
        expand(
            catalog,
            &ApiMethod::new("ec2", "DescribeInstances"),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
        )?;
        Ok(())
    }

    #[test]
    fn rejects_missing_references_evidence_and_conflicting_coverage()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut catalog = document()?;
        validate_catalog(&catalog)?;
        catalog.rules[0].downstream_operations = vec!["MissingOperation".to_owned()];
        assert!(validate_catalog(&catalog).is_err());
        let mut catalog = document()?;
        catalog.rules[0].sources.clear();
        assert!(validate_catalog(&catalog).is_err());
        let mut catalog = document()?;
        catalog.rules[0].sources[0].url = "https://example.com/indirect-map".to_owned();
        assert!(validate_catalog(&catalog).is_err());
        let mut catalog = document()?;
        catalog.reviewed_no_dependencies[0].operations = vec!["GetSecretValue".to_owned()];
        assert!(validate_catalog(&catalog).is_err());
        Ok(())
    }

    #[test]
    fn rejects_cycles_without_dropping_dependencies_silently()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut catalog = document()?;
        catalog
            .rules
            .retain(|rule| rule.id == "ssm-secrets-reference" || rule.id == "secrets-read");
        catalog.reviewed_no_dependencies.clear();
        let secret = catalog
            .rules
            .iter_mut()
            .find(|rule| rule.id == "secrets-read")
            .ok_or("missing secret rule")?;
        secret.downstream_service = "ssm".to_owned();
        secret.downstream_operations = vec!["GetParameter".to_owned()];
        secret.via_service = None;
        secret.key_placeholder = None;
        let error = validate_catalog(&catalog)
            .err()
            .ok_or("cycle was accepted")?;
        assert!(error.contains("dependency cycle"));
        Ok(())
    }

    #[test]
    fn rejects_duplicate_ids_and_unknown_document_fields() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut catalog = document()?;
        let duplicate = catalog.rules[0].id.clone();
        catalog.rules[1].id = duplicate;
        assert!(validate_catalog(&catalog).is_err());
        let mut value: serde_json::Value = serde_json::from_str(FAS_DOCUMENT)?;
        value["rules"][0]["misspelled_predicate"] = serde_json::json!(true);
        assert!(serde_json::from_value::<Catalog>(value).is_err());
        Ok(())
    }
}
