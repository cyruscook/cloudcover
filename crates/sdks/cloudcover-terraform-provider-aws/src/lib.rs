pub const SDK_NAME: &str = "terraform-provider-aws";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TerraformProviderAwsApiMethodRef {
    pub service: &'static str,
    pub name: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TerraformProviderAwsMethodMapping {
    pub kind: &'static str,
    pub type_name: &'static str,
    pub action: &'static str,
    pub api_methods: &'static [TerraformProviderAwsApiMethodRef],
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TerraformProviderAwsVersionMappings {
    pub version: &'static str,
    pub mappings: &'static [TerraformProviderAwsMethodMapping],
}

include!(concat!(
    env!("OUT_DIR"),
    "/terraform_provider_aws_mappings.rs"
));

#[must_use]
pub fn sdk_method_mappings(version: &str) -> Option<&'static [TerraformProviderAwsMethodMapping]> {
    PROVIDER_VERSIONS
        .iter()
        .find(|entry| entry.version == version)
        .map(|entry| entry.mappings)
}

#[cfg(test)]
mod tests {
    use super::{PROVIDER_VERSIONS, TerraformProviderAwsApiMethodRef, sdk_method_mappings};

    #[test]
    fn provider_versions_are_sorted_and_unique() {
        assert!(!PROVIDER_VERSIONS.is_empty());
        assert!(
            PROVIDER_VERSIONS
                .windows(2)
                .all(|window| window[0].version != window[1].version)
        );
    }

    #[test]
    fn mappings_and_api_methods_are_sorted_and_unique() {
        for version in PROVIDER_VERSIONS {
            let mut mappings = version.mappings.to_vec();
            mappings.sort();
            assert_eq!(version.mappings, mappings);
            assert!(
                version
                    .mappings
                    .windows(2)
                    .all(|window| window[0] != window[1])
            );
            for mapping in version.mappings {
                let mut api_methods = mapping.api_methods.to_vec();
                api_methods.sort();
                assert_eq!(mapping.api_methods, api_methods);
                assert!(
                    mapping
                        .api_methods
                        .windows(2)
                        .all(|window| window[0] != window[1])
                );
            }
        }
    }

    #[test]
    fn exact_lookup_retrieves_known_version_only() -> Result<(), &'static str> {
        let mappings = sdk_method_mappings("6.64.0").ok_or("missing v6.64.0 mappings")?;
        assert_mapping_contains(
            mappings,
            "resource",
            "aws_s3_bucket",
            "create",
            TerraformProviderAwsApiMethodRef {
                service: "s3",
                name: "CreateBucket",
            },
        );
        assert_mapping_contains(
            mappings,
            "resource",
            "aws_s3_bucket",
            "delete",
            TerraformProviderAwsApiMethodRef {
                service: "s3",
                name: "DeleteBucket",
            },
        );
        assert_mapping_contains(
            mappings,
            "action",
            "aws_ec2_stop_instance",
            "invoke",
            TerraformProviderAwsApiMethodRef {
                service: "ec2",
                name: "StopInstances",
            },
        );
        assert!(sdk_method_mappings("6.63.0").is_none());
        assert!(sdk_method_mappings("6.64").is_none());
        assert!(sdk_method_mappings("v6.64.0").is_none());
        assert!(sdk_method_mappings("6.64.0+local").is_none());
        Ok(())
    }

    fn assert_mapping_contains(
        mappings: &[super::TerraformProviderAwsMethodMapping],
        kind: &str,
        type_name: &str,
        action: &str,
        expected_api_method: TerraformProviderAwsApiMethodRef,
    ) {
        let matches: Vec<_> = mappings
            .iter()
            .filter(|mapping| {
                mapping.kind == kind && mapping.type_name == type_name && mapping.action == action
            })
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "missing mapping for {kind} {type_name} {action}"
        );
        assert!(matches[0].api_methods.contains(&expected_api_method));
    }
}
