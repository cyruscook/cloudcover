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

pub fn validate_and_apply_release(
    release: &mut SdkDataRelease,
    state: &mut MappingState,
) -> Result<Version, String> {
    let version = Version::parse(&release.module_version).map_err(|error| {
        format!(
            "invalid semantic version {:?}: {error}",
            release.module_version
        )
    })?;
    if !version.pre.is_empty()
        || !version.build.is_empty()
        || version.to_string() != release.module_version
    {
        return Err(format!(
            "version {:?} must be canonical stable SemVer",
            release.module_version
        ));
    }

    let remove_keys = release
        .remove
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    let upsert_keys = release
        .upsert
        .iter()
        .map(|row| (&row.0, &row.1, &row.2))
        .collect::<std::collections::BTreeSet<_>>();
    if remove_keys.len() != release.remove.len() || upsert_keys.len() != release.upsert.len() {
        return Err("metadata delta contains duplicate keys".to_owned());
    }
    release.remove.sort();
    release.remove.dedup();
    release.upsert.sort();
    release.upsert.dedup();
    for (package, receiver, method) in &release.remove {
        validate_key(package, receiver, method)?;
    }
    for (package, receiver, method, api_methods) in &mut release.upsert {
        validate_key(package, receiver, method)?;
        api_methods.sort();
        api_methods.dedup();
        if api_methods.is_empty()
            || api_methods
                .iter()
                .any(|(service, name)| service.is_empty() || name.is_empty())
        {
            return Err(format!(
                "SDK mapping {package} {receiver}.{method} has invalid API methods"
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
    for (package, receiver, method, api_methods) in &release.upsert {
        let key = (package.clone(), receiver.clone(), method.clone());
        if release.remove.binary_search(&key).is_ok() {
            return Err(format!(
                "SDK delta both removes and upserts {package} {receiver}.{method}"
            ));
        }
        let methods = api_methods
            .iter()
            .map(|(service, name)| ApiMethodRefRow {
                service: service.clone(),
                name: name.clone(),
            })
            .collect::<Vec<_>>();
        if state.get(&key) == Some(&methods) {
            return Err(format!(
                "SDK delta redundantly upserts unchanged mapping {package} {receiver}.{method}"
            ));
        }
        state.insert(key, methods);
    }
    Ok(version)
}

fn validate_key(package: &str, receiver: &str, method: &str) -> Result<(), String> {
    if package.is_empty() || receiver.is_empty() || method.is_empty() {
        return Err("SDK mapping has an empty package, receiver, or method".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    #[test]
    fn rejects_duplicate_keys_and_null_collections() -> Result<(), Box<dyn std::error::Error>> {
        let mut release: SdkDataRelease = serde_json::from_str(
            r#"{"module_version":"1.0.0","remove":[],"upsert":[["p","C","M",[["s","A"]]],["p","C","M",[["s","A"]]]]}"#,
        )?;
        let mut state = MappingState::new();
        assert!(validate_and_apply_release(&mut release, &mut state).is_err());
        assert!(
            serde_json::from_str::<SdkDataRelease>(
                r#"{"module_version":"1.0.0","remove":null,"upsert":[]}"#
            )
            .is_err()
        );
        Ok(())
    }
}
