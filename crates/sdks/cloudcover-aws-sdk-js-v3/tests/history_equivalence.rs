use cloudcover_aws_sdk_js_v3::{service_method_mappings, service_versions};
use serde_json::Value;
use std::collections::BTreeMap;

#[test]
fn every_source_release_matches_pooled_compiled_rows() -> Result<(), Box<dyn std::error::Error>> {
    let data: Value = serde_json::from_str(include_str!("../data/mappings.json"))?;
    for package in data["packages"].as_array().ok_or("packages")? {
        let name = package["package"].as_str().ok_or("name")?;
        let mut state = BTreeMap::new();
        let releases = package["releases"].as_array().ok_or("releases")?;
        assert_eq!(
            service_versions(name).ok_or("missing package")?.len(),
            releases.len()
        );
        for release in releases {
            for row in release["remove"].as_array().ok_or("remove")? {
                let key = (row[0].as_str(), row[1].as_str().ok_or("method")?);
                assert!(state.remove(&key).is_some());
            }
            for row in release["upsert"].as_array().ok_or("upsert")? {
                let key = (
                    row["receiver"].as_str(),
                    row["method"].as_str().ok_or("method")?,
                );
                let mut apis = row["api_methods"]
                    .as_array()
                    .ok_or("api_methods")?
                    .iter()
                    .map(|api| {
                        Ok((
                            api["service"].as_str().ok_or("service")?,
                            api["name"].as_str().ok_or("api name")?,
                        ))
                    })
                    .collect::<Result<Vec<_>, &str>>()?;
                apis.sort_unstable();
                apis.dedup();
                state.insert(key, apis);
            }
            let version = release["version"].as_str().ok_or("version")?;
            let rows = service_method_mappings(name, version).ok_or("missing exact release")?;
            let actual: BTreeMap<_, _> = rows
                .iter()
                .map(|row| {
                    assert_eq!(row.package, name);
                    (
                        (row.receiver, row.method),
                        row.api_methods
                            .iter()
                            .map(|api| (api.service, api.name))
                            .collect::<Vec<_>>(),
                    )
                })
                .collect();
            assert_eq!(actual, state, "{name}@{version}");
        }
    }
    Ok(())
}
