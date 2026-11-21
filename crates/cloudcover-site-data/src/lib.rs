use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs, io,
    path::Path,
};

use cloudcover_aws::{AwsError, AwsProvider, IamPermission, iam_permission_catalog};
use cloudcover_core::{ApiMethod, CloudProvider, Language, MethodReference, ResolvedSdk, Sdk};
use cloudcover_terraform_provider_aws::{
    TerraformProviderAwsMappingsLookup, provider_versions, sdk_method_mappings,
};
use serde::Serialize;
use tempfile::Builder;

const SCHEMA_VERSION: u8 = 1;
const API_SCHEMA_VERSION: u8 = 2;
const LIFECYCLE_ACTIONS: [&str; 4] = ["create", "read", "update", "delete"];

// null means unresolved authorization; [] is an affirmatively resolved empty set.
type ApiRow = (String, String, Option<Vec<usize>>);
type TerraformRow = (String, String, Vec<usize>);

#[derive(Serialize)]
struct ApiArtifact {
    schema_version: u8,
    iam_permissions: Vec<String>,
    api_methods: Vec<ApiRow>,
    fas: Vec<Option<ApiFas>>,
    policy_status: Vec<PolicyStatus>,
}

// This is a blocker projection, never a record of request-specific approval.
// Blocked legacy rows stay null; their independently indexed evidence is retained
// here for newer browsers without enabling old clients to copy a partial list.
#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum PolicyStatus {
    Unknown {
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<&'static str>,
    },
    Blocked {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<&'static str>,
        evidence_permission_ids: Option<Vec<usize>>,
    },
}

struct PolicyBlocker {
    message: String,
    source: Option<&'static str>,
}

#[derive(Serialize)]
struct ApiFas {
    coverage: cloudcover_aws::fas::FasCoverage,
    dependencies: Vec<cloudcover_aws::fas::FasDependency>,
}

#[derive(Serialize)]
struct TerraformIndexArtifact {
    schema_version: u8,
    latest: String,
    versions: Vec<String>,
}

#[derive(Serialize)]
struct TerraformRowsArtifact {
    schema_version: u8,
    rows: Vec<TerraformRow>,
}

#[derive(Serialize)]
struct TerraformVersionArtifact {
    schema_version: u8,
    version: String,
    row_ids: Vec<usize>,
}

#[derive(Serialize)]
struct IamArtifact {
    schema_version: u8,
    permissions: Vec<PermissionDetails>,
}

#[derive(Serialize)]
struct PermissionDetails {
    permission: &'static str,
    resources: Vec<ResourceDetails>,
    context_keys: Vec<ContextKeyDetails>,
}

#[derive(Serialize)]
struct ResourceDetails {
    name: &'static str,
    arn_formats: &'static [&'static str],
}

#[derive(Serialize)]
struct ContextKeyDetails {
    name: &'static str,
    types: &'static [&'static str],
    permission: bool,
    permission_resources: &'static [&'static str],
    resource_types: &'static [&'static str],
}

impl From<&IamPermission> for PermissionDetails {
    fn from(value: &IamPermission) -> Self {
        Self {
            permission: value.permission,
            resources: value
                .resources
                .iter()
                .map(|resource| ResourceDetails {
                    name: resource.name,
                    arn_formats: resource.arn_formats,
                })
                .collect(),
            context_keys: value
                .context_keys
                .iter()
                .map(|key| ContextKeyDetails {
                    name: key.name,
                    types: key.types,
                    permission: key.permission,
                    permission_resources: key.permission_resources,
                    resource_types: key.resource_types,
                })
                .collect(),
        }
    }
}

struct Catalog {
    api: ApiArtifact,
    terraform_index: TerraformIndexArtifact,
    terraform_rows: TerraformRowsArtifact,
    terraform_versions: Vec<TerraformVersionArtifact>,
}

/// Generates the complete static data tree used by the browser application.
///
/// # Errors
///
/// Returns an error when source data cannot be resolved, an artifact cannot be
/// serialized or written, or the output directory cannot be replaced.
pub fn generate(output_dir: &Path) -> Result<(), Box<dyn Error>> {
    let output_exists = match fs::symlink_metadata(output_dir) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() {
                return Err(io::Error::other(format!(
                    "output path is not a directory: {}",
                    output_dir.display()
                ))
                .into());
            }
            true
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };

    let parent = output_dir.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let staging = Builder::new()
        .prefix(".cloudcover-site-data-")
        .tempdir_in(parent)?;
    let catalog = build_catalog()?;
    write_catalog(staging.path(), &catalog)?;
    replace_output(output_dir, parent, staging.path(), output_exists)?;
    Ok(())
}

fn build_catalog() -> Result<Catalog, Box<dyn Error>> {
    let provider = AwsProvider::new();
    let mut api_methods = provider.list_api_methods();
    api_methods.sort();
    api_methods.dedup();

    let mut permissions_by_method = Vec::with_capacity(api_methods.len());
    let mut blockers = Vec::with_capacity(api_methods.len());
    let mut all_permissions = iam_permission_catalog()
        .iter()
        .map(|entry| entry.permission.to_owned())
        .collect::<BTreeSet<_>>();
    for method in &api_methods {
        let permissions = resolved_authorization(provider.iam_permissions(method))?;
        blockers.push(policy_blocker(
            provider
                .review_permissions_policy(std::slice::from_ref(method), false)
                .map(drop),
        )?);
        for permission in permissions.iter().flatten() {
            all_permissions.insert((*permission).to_owned());
        }
        permissions_by_method.push(permissions);
    }

    let iam_permissions = all_permissions.into_iter().collect::<Vec<_>>();
    let permission_ids = iam_permissions
        .iter()
        .enumerate()
        .map(|(id, permission)| (permission.as_str(), id))
        .collect::<BTreeMap<_, _>>();
    let api_ids = api_methods
        .iter()
        .enumerate()
        .map(|(id, method)| ((method.service(), method.name()), id))
        .collect::<BTreeMap<_, _>>();

    let mut policy_status = Vec::with_capacity(api_methods.len());
    let api_rows = api_methods
        .iter()
        .zip(permissions_by_method)
        .zip(blockers)
        .map(|((method, permissions), blocker)| {
            let permission_ids = permissions
                .map(|permissions| {
                    permissions
                        .into_iter()
                        .map(|permission| {
                            permission_ids.get(permission).copied().ok_or_else(|| {
                                io::Error::other(format!("missing IAM permission {permission:?}"))
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?;
            let (status, permission_ids) = project_policy_status(
                provider.authorization_disposition(method),
                blocker,
                permission_ids,
            );
            policy_status.push(status);
            Ok((
                method.service().to_owned(),
                method.name().to_owned(),
                permission_ids,
            ))
        })
        .collect::<Result<Vec<_>, io::Error>>()?;

    let fas = api_methods
        .iter()
        .map(|method| {
            let coverage = provider.fas_coverage(method)?;
            Ok(
                resolved_authorization(provider.fas_dependencies(method))?.map(|dependencies| {
                    ApiFas {
                        coverage,
                        dependencies,
                    }
                }),
            )
        })
        .collect::<Result<Vec<_>, cloudcover_aws::AwsError>>()?;

    let (terraform_index, terraform_rows, terraform_versions) = build_terraform_catalog(&api_ids)?;

    Ok(Catalog {
        api: ApiArtifact {
            schema_version: API_SCHEMA_VERSION,
            iam_permissions,
            api_methods: api_rows,
            fas,
            policy_status,
        },
        terraform_index,
        terraform_rows,
        terraform_versions,
    })
}

fn project_policy_status(
    disposition: Option<&cloudcover_aws::OperationDisposition>,
    blocker: Option<PolicyBlocker>,
    evidence: Option<Vec<usize>>,
) -> (PolicyStatus, Option<Vec<usize>>) {
    if let Some(blocker) = blocker {
        (
            PolicyStatus::Blocked {
                message: blocker.message,
                source: blocker.source,
                evidence_permission_ids: evidence,
            },
            None,
        )
    } else {
        (
            PolicyStatus::Unknown {
                message: disposition.map(ToString::to_string),
                source: disposition.map(|value| value.source),
            },
            evidence,
        )
    }
}

fn policy_blocker(value: Result<(), AwsError>) -> Result<Option<PolicyBlocker>, AwsError> {
    match value {
        Ok(()) => Ok(None),
        Err(error) => {
            if let AwsError::UnresolvedAuthorization { service, name } = &error {
                // A FAS error may identify a leaf rather than the requested root.
                // Only that actual failure's blocking disposition can supply a source.
                let source = AwsProvider::new()
                    .authorization_disposition(&ApiMethod::new(service, name))
                    .filter(|diagnostic| diagnostic.blocks_policy())
                    .map(|diagnostic| diagnostic.source);
                Ok(Some(PolicyBlocker {
                    message: error.to_string(),
                    source,
                }))
            } else {
                Err(error)
            }
        }
    }
}

fn resolved_authorization<T>(value: Result<T, AwsError>) -> Result<Option<T>, AwsError> {
    match value {
        Ok(value) => Ok(Some(value)),
        Err(AwsError::UnresolvedAuthorization { .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

fn build_terraform_catalog(
    api_ids: &BTreeMap<(&str, &str), usize>,
) -> Result<
    (
        TerraformIndexArtifact,
        TerraformRowsArtifact,
        Vec<TerraformVersionArtifact>,
    ),
    Box<dyn Error>,
> {
    let mut supported_versions = Vec::new();
    let mut version_rows = Vec::new();
    let mut all_rows = BTreeSet::new();

    for version in provider_versions() {
        let mappings = match sdk_method_mappings(version) {
            TerraformProviderAwsMappingsLookup::Supported(_) => AwsProvider::new()
                .sdk_method_mappings(&ResolvedSdk::new(
                    Sdk::new(
                        cloudcover_terraform_provider_aws::SDK_NAME,
                        Language::Terraform,
                    )
                    .with_version(*version),
                ))?,
            TerraformProviderAwsMappingsLookup::Unsupported(_) => continue,
            TerraformProviderAwsMappingsLookup::Unknown => {
                return Err(io::Error::other(format!(
                    "Terraform provider AWS version {version:?} is not indexed"
                ))
                .into());
            }
        };

        let mut rows = BTreeSet::new();
        for mapping in mappings {
            let MethodReference::Terraform(reference) = mapping.method() else {
                return Err(io::Error::other("non-Terraform method in Terraform catalog").into());
            };
            if reference.kind() != "resource" || !LIFECYCLE_ACTIONS.contains(&reference.action()) {
                continue;
            }

            let mut method_ids = mapping
                .api_methods()
                .iter()
                .map(|method| {
                    api_ids
                        .get(&(method.service(), method.name()))
                        .copied()
                        .ok_or_else(|| {
                            io::Error::other(format!(
                                "missing canonical API method {}:{}",
                                method.service(),
                                method.name()
                            ))
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            method_ids.sort_unstable();
            method_ids.dedup();
            rows.insert((
                reference.type_name().to_owned(),
                reference.action().to_owned(),
                method_ids,
            ));
        }

        let rows = rows.into_iter().collect::<Vec<_>>();
        all_rows.extend(rows.iter().cloned());
        supported_versions.push((*version).to_owned());
        version_rows.push(((*version).to_owned(), rows));
    }

    let latest = supported_versions
        .last()
        .cloned()
        .ok_or_else(|| io::Error::other("no supported Terraform provider AWS versions"))?;
    let rows = all_rows.into_iter().collect::<Vec<_>>();
    let row_ids = rows
        .iter()
        .enumerate()
        .map(|(id, row)| (row.clone(), id))
        .collect::<BTreeMap<_, _>>();
    let terraform_versions = version_rows
        .into_iter()
        .map(|(version, rows)| {
            let mut ids = rows
                .into_iter()
                .map(|row| {
                    row_ids.get(&row).copied().ok_or_else(|| {
                        io::Error::other(format!("missing Terraform row for {version:?}"))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            ids.sort_unstable();
            ids.dedup();
            Ok(TerraformVersionArtifact {
                schema_version: SCHEMA_VERSION,
                version,
                row_ids: ids,
            })
        })
        .collect::<Result<Vec<_>, io::Error>>()?;

    Ok((
        TerraformIndexArtifact {
            schema_version: SCHEMA_VERSION,
            latest,
            versions: supported_versions,
        },
        TerraformRowsArtifact {
            schema_version: SCHEMA_VERSION,
            rows,
        },
        terraform_versions,
    ))
}

fn write_catalog(root: &Path, catalog: &Catalog) -> Result<(), Box<dyn Error>> {
    write_json(&root.join("api.json"), &catalog.api)?;
    fs::write(root.join("fas.json"), cloudcover_aws::fas::FAS_DOCUMENT)?;
    let mut services = BTreeMap::<&str, Vec<PermissionDetails>>::new();
    for permission in iam_permission_catalog() {
        let (service, _) = permission
            .permission
            .split_once(':')
            .ok_or("invalid IAM permission")?;
        services.entry(service).or_default().push(permission.into());
    }
    for (service, permissions) in services {
        write_json(
            &root.join(format!("iam/{service}.json")),
            &IamArtifact {
                schema_version: SCHEMA_VERSION,
                permissions,
            },
        )?;
    }
    write_json(&root.join("terraform/index.json"), &catalog.terraform_index)?;
    write_json(&root.join("terraform/rows.json"), &catalog.terraform_rows)?;
    for version in &catalog.terraform_versions {
        write_json(
            &root.join(format!("terraform/versions/{}.json", version.version)),
            version,
        )?;
    }
    Ok(())
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = serde_json::to_vec(value)?;
    fs::write(path, contents)?;
    Ok(())
}

fn replace_output(
    output_dir: &Path,
    parent: &Path,
    staging: &Path,
    output_exists: bool,
) -> Result<(), Box<dyn Error>> {
    if !output_exists {
        fs::rename(staging, output_dir)?;
        return Ok(());
    }

    let backup = tempfile::tempdir_in(parent)?;
    let backup_path = backup.path().join("previous");
    fs::rename(output_dir, &backup_path)?;
    if let Err(error) = fs::rename(staging, output_dir) {
        let restore_result = fs::rename(&backup_path, output_dir);
        return match restore_result {
            Ok(()) => Err(error.into()),
            Err(restore_error) => Err(io::Error::other(format!(
                "failed to replace {}: {error}; failed to restore previous output: {restore_error}",
                output_dir.display()
            ))
            .into()),
        };
    }
    drop(backup);
    Ok(())
}

#[cfg(test)]
mod authorization_tests {
    use super::*;

    #[test]
    fn missing_authorization_uses_null_but_other_errors_remain_errors() {
        assert_eq!(
            resolved_authorization(Ok(Vec::<String>::new())).ok(),
            Some(Some(Vec::new()))
        );
        assert_eq!(
            resolved_authorization::<Vec<String>>(Err(AwsError::UnresolvedAuthorization {
                service: "s3".to_owned(),
                name: "RenameObject".to_owned(),
            }))
            .ok(),
            Some(None)
        );
        assert!(matches!(
            resolved_authorization::<()>(Err(AwsError::UnknownApiMethod {
                service: "missing".to_owned(),
                name: "Missing".to_owned(),
            })),
            Err(AwsError::UnknownApiMethod { .. })
        ));
        assert!(matches!(
            resolved_authorization::<()>(Err(AwsError::InvalidFasCatalog {
                message: "invalid evidence".to_owned(),
            })),
            Err(AwsError::InvalidFasCatalog { .. })
        ));
    }

    #[test]
    fn blocker_projection_never_treats_success_as_approval_or_hides_other_errors() {
        assert!(matches!(policy_blocker(Ok(())), Ok(None)));
        let error = AwsError::UnresolvedAuthorization {
            service: "s3".to_owned(),
            name: "RenameObject".to_owned(),
        };
        assert!(
            matches!(policy_blocker(Err(error)), Ok(Some(blocker)) if blocker.message.contains("s3:RenameObject"))
        );
        assert!(matches!(
            policy_blocker(Err(AwsError::UnknownApiMethod {
                service: "missing".to_owned(),
                name: "Missing".to_owned(),
            })),
            Err(AwsError::UnknownApiMethod { .. })
        ));
        assert!(matches!(
            policy_blocker(Err(AwsError::InvalidFasCatalog {
                message: "invalid evidence".to_owned(),
            })),
            Err(AwsError::InvalidFasCatalog { .. })
        ));
    }

    #[test]
    fn unknown_fas_leaf_cannot_inherit_the_roots_diagnostic_source() -> Result<(), Box<dyn Error>> {
        let root = AwsProvider::new()
            .authorization_disposition(&ApiMethod::new("network-firewall", "CreateProxy"))
            .ok_or("legacy root diagnostic")?;
        // This root is an informational warning on the old pin. Whether retained
        // or removed, its source cannot explain a different operation's failure.
        let blocker = policy_blocker(Err(AwsError::UnresolvedAuthorization {
            service: "kms".to_owned(),
            name: "UnknownLeaf".to_owned(),
        }))?;
        let (status, legacy) = project_policy_status(Some(root), blocker, Some(vec![0]));
        assert!(legacy.is_none());
        assert!(matches!(status, PolicyStatus::Blocked { source: None, .. }));
        // Even when the actual failing operation has a disposition, an old-pin
        // nonblocking warning cannot be used as evidence for its policy failure.
        let actual_root_failure = policy_blocker(Err(AwsError::UnresolvedAuthorization {
            service: root.service.to_owned(),
            name: root.operation.to_owned(),
        }))?
        .ok_or("missing blocker")?;
        assert_eq!(
            actual_root_failure.source,
            root.blocks_policy().then_some(root.source)
        );
        Ok(())
    }

    #[test]
    fn known_blocking_leaf_supplies_its_own_source() -> Result<(), Box<dyn Error>> {
        let provider = AwsProvider::new();
        let root = provider
            .authorization_disposition(&ApiMethod::new("network-firewall", "CreateProxy"))
            .ok_or("legacy root diagnostic")?;
        let leaf = provider
            .authorization_disposition(&ApiMethod::new("cloudwatch", "CreateAlert"))
            .ok_or("blocking leaf diagnostic")?;
        assert!(leaf.blocks_policy());
        assert_ne!(root.source, leaf.source);
        let blocker = policy_blocker(Err(AwsError::UnresolvedAuthorization {
            service: "cloudwatch".to_owned(),
            name: "CreateAlert".to_owned(),
        }))?;
        let (status, legacy) = project_policy_status(Some(root), blocker, Some(vec![0]));
        assert!(legacy.is_none());
        assert!(
            matches!(status, PolicyStatus::Blocked { source: Some(source), .. } if source == leaf.source)
        );
        Ok(())
    }
}
