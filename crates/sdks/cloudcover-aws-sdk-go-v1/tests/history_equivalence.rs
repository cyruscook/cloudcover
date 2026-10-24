#[path = "../src/data.rs"]
mod data;

#[test]
fn every_checked_in_release_matches_compiled_lookup() -> Result<(), Box<dyn std::error::Error>> {
    let mut file: data::SdkDataFile =
        serde_json::from_str(include_str!("../data/aws-sdk-go.json"))?;
    let mut state = data::MappingState::new();
    for release in &mut file.releases {
        data::validate_and_apply_release(release, &mut state)?;
        let actual: data::MappingState =
            cloudcover_aws_sdk_go_v1::sdk_method_mappings(&release.module_version)
                .ok_or("missing exact version")?
                .map(|row| {
                    (
                        (
                            row.package.to_owned(),
                            row.receiver.to_owned(),
                            row.method.to_owned(),
                        ),
                        row.api_methods
                            .map(|api| data::ApiMethodRefRow {
                                service: api.service.to_owned(),
                                name: api.name.to_owned(),
                            })
                            .collect(),
                    )
                })
                .collect();
        assert_eq!(state, actual, "{}", release.module_version);
    }
    Ok(())
}
