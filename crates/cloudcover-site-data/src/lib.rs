use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs, io,
    path::Path,
};

use cloudcover_aws::AwsProvider;
use cloudcover_core::CloudProvider;
use cloudcover_terraform_provider_aws::{
    TerraformProviderAwsMappingsLookup, provider_versions, sdk_method_mappings,
};
use serde::Serialize;
use tempfile::Builder;

const SCHEMA_VERSION: u8 = 1;
const LIFECYCLE_ACTIONS: [&str; 4] = ["create", "read", "update", "delete"];

type ApiRow = (String, String, Vec<usize>);
type TerraformRow = (String, String, Vec<usize>);

#[derive(Serialize)]
struct ApiArtifact {
    schema_version: u8,
    iam_permissions: Vec<String>,
    api_methods: Vec<ApiRow>,
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
    let mut all_permissions = BTreeSet::new();
    for method in &api_methods {
        let permissions = provider.iam_permissions(method)?;
        for permission in &permissions {
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

    let api_rows = api_methods
        .iter()
        .zip(permissions_by_method)
        .map(|(method, permissions)| {
            let permission_ids = permissions
                .into_iter()
                .map(|permission| {
                    permission_ids.get(permission).copied().ok_or_else(|| {
                        io::Error::other(format!("missing IAM permission {permission:?}"))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((
                method.service().to_owned(),
                method.name().to_owned(),
                permission_ids,
            ))
        })
        .collect::<Result<Vec<_>, io::Error>>()?;

    let (terraform_index, terraform_rows, terraform_versions) = build_terraform_catalog(&api_ids)?;

    Ok(Catalog {
        api: ApiArtifact {
            schema_version: SCHEMA_VERSION,
            iam_permissions,
            api_methods: api_rows,
        },
        terraform_index,
        terraform_rows,
        terraform_versions,
    })
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
            TerraformProviderAwsMappingsLookup::Supported(mappings) => mappings,
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
            if mapping.kind != "resource" || !LIFECYCLE_ACTIONS.contains(&mapping.action) {
                continue;
            }

            let mut method_ids = mapping
                .api_methods
                .iter()
                .filter_map(|method| api_ids.get(&(method.service, method.name)).copied())
                .collect::<Vec<_>>();
            method_ids.sort_unstable();
            method_ids.dedup();
            rows.insert((
                mapping.type_name.to_owned(),
                mapping.action.to_owned(),
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
