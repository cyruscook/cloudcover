use std::collections::BTreeMap;

use semver::Version;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApiMethodRefRow {
    pub service: String,
    pub name: String,
}

pub type MappingKey = (String, String, String);
pub type ApiMethodTuple = (String, String);
pub type CompactMappingRow = (String, String, String, Vec<ApiMethodTuple>);
pub type MappingState = BTreeMap<MappingKey, Vec<ApiMethodRefRow>>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SdkDataFile {
    pub module_path: String,
    pub releases: Vec<SdkDataRelease>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SdkDataRelease {
    pub module_version: String,
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
    file: &mut SdkDataFile,
    expected_module_path: Option<&str>,
    state: &mut MappingState,
) -> Result<Vec<Version>, String> {
    if file.module_path.is_empty() {
        return Err("SDK module path must not be empty".to_owned());
    }
    if let Some(expected_module_path) = expected_module_path
        && file.module_path != expected_module_path
    {
        return Err(format!(
            "SDK module path {:?} does not match expected {:?}",
            file.module_path, expected_module_path
        ));
    }
    if file.releases.is_empty() {
        return Err(format!(
            "SDK module {:?} contains no stable releases",
            file.module_path
        ));
    }

    let mut versions = Vec::with_capacity(file.releases.len());
    for release in &mut file.releases {
        let version = validate_and_apply_release(release, state)?;
        if versions.last().is_some_and(|previous| previous >= &version) {
            return Err(format!(
                "SDK module {:?} releases are not strictly increasing at {}",
                file.module_path, release.module_version
            ));
        }
        versions.push(version);
    }
    Ok(versions)
}

pub fn validate_and_apply_release(
    release: &mut SdkDataRelease,
    state: &mut MappingState,
) -> Result<Version, String> {
    let version = parse_canonical_version(&release.module_version)?;

    for (package, receiver, method) in &release.remove {
        validate_key(package, receiver, method)?;
    }
    release.remove.sort();
    release.remove.dedup();

    let mut upsert_rows = release
        .upsert
        .drain(..)
        .map(compact_to_row)
        .collect::<Vec<_>>();
    if !upsert_rows.is_empty() {
        validate_and_normalize_rows(&mut upsert_rows)?;
    }
    release.upsert = upsert_rows.into_iter().map(row_to_compact).collect();

    let upsert_keys = release
        .upsert
        .iter()
        .map(|(package, receiver, method, _)| (package, receiver, method))
        .collect::<Vec<_>>();
    if upsert_keys.windows(2).any(|window| window[0] == window[1]) {
        return Err("SDK delta contains duplicate upsert keys".to_owned());
    }
    for (package, receiver, method) in &release.remove {
        if upsert_keys
            .binary_search(&(package, receiver, method))
            .is_ok()
        {
            return Err(format!(
                "SDK delta both removes and upserts {package} {receiver}.{method}"
            ));
        }
    }

    for key in &release.remove {
        if state.remove(key).is_none() {
            return Err(format!(
                "SDK delta removes missing mapping {} {}.{}",
                key.0, key.1, key.2
            ));
        }
    }
    for compact in &release.upsert {
        let row = compact_to_row(compact.clone());
        let key = (row.0.clone(), row.1.clone(), row.2.clone());
        if state.get(&key) == Some(&row.3) {
            return Err(format!(
                "SDK delta redundantly upserts unchanged mapping {} {}.{}",
                key.0, key.1, key.2
            ));
        }
        state.insert(key, row.3);
    }

    Ok(version)
}

fn validate_and_normalize_rows(
    rows: &mut Vec<(String, String, String, Vec<ApiMethodRefRow>)>,
) -> Result<(), String> {
    for (package, receiver, method, api_methods) in rows.iter_mut() {
        validate_key(package, receiver, method)?;
        if api_methods.is_empty() {
            return Err(format!(
                "SDK mapping {package} {receiver}.{method} has no API methods"
            ));
        }
        api_methods.sort();
        api_methods.dedup();
        for api_method in api_methods {
            if api_method.service.is_empty() || api_method.name.is_empty() {
                return Err(format!(
                    "SDK mapping {package} {receiver}.{method} has empty API method fields"
                ));
            }
        }
    }

    let mut by_key = BTreeMap::<MappingKey, Vec<ApiMethodRefRow>>::new();
    for row in rows.iter() {
        let key = (row.0.clone(), row.1.clone(), row.2.clone());
        match by_key.get(&key) {
            Some(existing) if existing != &row.3 => {
                return Err(format!(
                    "SDK mapping rows disagree for {} {}.{}",
                    key.0, key.1, key.2
                ));
            }
            Some(_) => {}
            None => {
                by_key.insert(key, row.3.clone());
            }
        }
    }

    rows.sort();
    rows.dedup();
    Ok(())
}

fn validate_key(package: &str, receiver: &str, method: &str) -> Result<(), String> {
    if package.is_empty() || receiver.is_empty() || method.is_empty() {
        return Err("SDK mapping has an empty package, receiver, or method".to_owned());
    }
    Ok(())
}

fn compact_to_row(compact: CompactMappingRow) -> (String, String, String, Vec<ApiMethodRefRow>) {
    (
        compact.0,
        compact.1,
        compact.2,
        compact
            .3
            .into_iter()
            .map(|(service, name)| ApiMethodRefRow { service, name })
            .collect(),
    )
}

fn row_to_compact(row: (String, String, String, Vec<ApiMethodRefRow>)) -> CompactMappingRow {
    (
        row.0,
        row.1,
        row.2,
        row.3
            .into_iter()
            .map(|api_method| (api_method.service, api_method.name))
            .collect(),
    )
}
