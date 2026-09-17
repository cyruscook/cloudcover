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

pub type MappingKey = (String, String, String);
pub type ApiMethodTuple = (String, String);
pub type CompactMappingRow = (String, String, String, Vec<ApiMethodTuple>);
pub type MappingState = BTreeMap<MappingKey, Vec<ApiMethodRefRow>>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionDataFile {
    pub provider_version: String,
    pub remove: Vec<MappingKey>,
    pub upsert: Vec<CompactMappingRow>,
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

pub fn validate_and_apply_file(
    file: &mut PermissionDataFile,
    expected_version: Option<&str>,
    state: &mut MappingState,
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

    for (kind, type_name, action) in &file.remove {
        validate_key(kind, type_name, action)?;
    }
    file.remove.sort();
    file.remove.dedup();

    let mut upsert_rows = file
        .upsert
        .drain(..)
        .map(compact_to_row)
        .collect::<Vec<_>>();
    if !upsert_rows.is_empty() {
        validate_and_normalize_rows(&mut upsert_rows)?;
    }
    file.upsert = upsert_rows.into_iter().map(row_to_compact).collect();

    let upsert_keys = file
        .upsert
        .iter()
        .map(|(kind, type_name, action, _)| (kind, type_name, action))
        .collect::<Vec<_>>();
    if upsert_keys.windows(2).any(|window| window[0] == window[1]) {
        return Err("permission delta contains duplicate upsert keys".to_owned());
    }
    for (kind, type_name, action) in &file.remove {
        if upsert_keys
            .binary_search(&(kind, type_name, action))
            .is_ok()
        {
            return Err(format!(
                "permission delta both removes and upserts {kind} {type_name} {action}"
            ));
        }
    }

    for key in &file.remove {
        if state.remove(key).is_none() {
            return Err(format!(
                "permission delta removes missing mapping {} {} {}",
                key.0, key.1, key.2
            ));
        }
    }
    for compact in &file.upsert {
        let row = compact_to_row(compact.clone());
        let key = mapping_key(&row);
        if state.get(&key) == Some(&row.api_methods) {
            return Err(format!(
                "permission delta redundantly upserts unchanged mapping {} {} {}",
                key.0, key.1, key.2
            ));
        }
        state.insert(key, row.api_methods);
    }

    if state.is_empty() {
        return Err("permission data contains no method mappings".to_owned());
    }
    Ok(version)
}

pub fn validate_and_normalize_rows(
    rows: &mut Vec<TerraformProviderAwsMethodMappingRow>,
) -> Result<(), String> {
    if rows.is_empty() {
        return Err("permission data contains no method mappings".to_owned());
    }

    for row in rows.iter_mut() {
        validate_key(&row.kind, &row.type_name, &row.action)?;
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

    let mut by_key = BTreeMap::<MappingKey, Vec<ApiMethodRefRow>>::new();
    for row in rows.iter() {
        let key = mapping_key(row);
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

    rows.sort();
    rows.dedup();
    Ok(())
}

fn validate_key(kind: &str, type_name: &str, action: &str) -> Result<(), String> {
    if kind.is_empty() || type_name.is_empty() || action.is_empty() {
        return Err("permission data mapping row has empty kind/type_name/action".to_owned());
    }
    Ok(())
}

fn mapping_key(row: &TerraformProviderAwsMethodMappingRow) -> MappingKey {
    (row.kind.clone(), row.type_name.clone(), row.action.clone())
}

fn compact_to_row(compact: CompactMappingRow) -> TerraformProviderAwsMethodMappingRow {
    TerraformProviderAwsMethodMappingRow {
        kind: compact.0,
        type_name: compact.1,
        action: compact.2,
        api_methods: compact
            .3
            .into_iter()
            .map(|(service, name)| ApiMethodRefRow { service, name })
            .collect(),
    }
}

fn row_to_compact(row: TerraformProviderAwsMethodMappingRow) -> CompactMappingRow {
    (
        row.kind,
        row.type_name,
        row.action,
        row.api_methods
            .into_iter()
            .map(|api_method| (api_method.service, api_method.name))
            .collect(),
    )
}
