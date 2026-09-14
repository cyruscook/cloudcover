use std::collections::BTreeMap;

use semver::Version;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApiMethodRefRow {
    pub service: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TerraformProviderAwsMethodMappingRow {
    pub kind: String,
    pub type_name: String,
    pub action: String,
    pub api_methods: Vec<ApiMethodRefRow>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionDataFile {
    pub provider_version: String,
    pub mappings: Vec<TerraformProviderAwsMethodMappingRow>,
}

pub fn parse_canonical_version(raw: &str) -> Result<Version, String> {
    let version = Version::parse(raw)
        .map_err(|error| format!("invalid semantic version {raw:?}: {error}"))?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        return Err(format!(
            "version {raw:?} must not contain prerelease or build metadata"
        ));
    }
    if version.to_string() != raw {
        return Err(format!("version {raw:?} is not canonical SemVer"));
    }
    Ok(version)
}

pub fn validate_and_normalize_file(
    file: &mut PermissionDataFile,
    expected_version: Option<&str>,
) -> Result<Version, String> {
    let version = parse_canonical_version(&file.provider_version)?;
    if let Some(expected_version) = expected_version {
        let expected = parse_canonical_version(expected_version)?;
        if version != expected || file.provider_version != expected_version {
            return Err(format!(
                "provider version {:?} does not match expected {:?}",
                file.provider_version, expected_version
            ));
        }
    }
    validate_and_normalize_rows(&mut file.mappings)?;
    Ok(version)
}

pub fn validate_and_normalize_rows(
    rows: &mut Vec<TerraformProviderAwsMethodMappingRow>,
) -> Result<(), String> {
    if rows.is_empty() {
        return Err("permission data contains no method mappings".to_owned());
    }

    for row in rows.iter_mut() {
        if row.kind.is_empty() || row.type_name.is_empty() || row.action.is_empty() {
            return Err("permission data mapping row has empty kind/type_name/action".to_owned());
        }

        row.api_methods.sort();
        row.api_methods.dedup();

        for api_method in &row.api_methods {
            if api_method.service.is_empty() || api_method.name.is_empty() {
                return Err(format!(
                    "permission data mapping row {} {} {} has empty api_method fields",
                    row.kind, row.type_name, row.action
                ));
            }
        }
    }

    let mut by_key = BTreeMap::<(String, String, String), Vec<ApiMethodRefRow>>::new();
    for row in rows.iter() {
        let key = (row.kind.clone(), row.type_name.clone(), row.action.clone());
        match by_key.get(&key) {
            Some(existing) if existing != &row.api_methods => {
                return Err(format!(
                    "permission data mapping rows disagree for {} {} {}",
                    row.kind, row.type_name, row.action
                ));
            }
            Some(_) => {}
            None => {
                by_key.insert(key, row.api_methods.clone());
            }
        }
    }

    rows.sort_by(|left, right| {
        (
            left.kind.as_str(),
            left.type_name.as_str(),
            left.action.as_str(),
            &left.api_methods,
        )
            .cmp(&(
                right.kind.as_str(),
                right.type_name.as_str(),
                right.action.as_str(),
                &right.api_methods,
            ))
    });
    rows.dedup();

    Ok(())
}
