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

include!(concat!(
    env!("OUT_DIR"),
    "/terraform_provider_aws_mappings.rs"
));

#[cfg(test)]
mod tests {
    use super::{SDK_METHOD_MAPPINGS, TerraformProviderAwsApiMethodRef};

    #[test]
    fn sdk_method_mappings_are_sorted_and_unique() {
        let mut sorted = SDK_METHOD_MAPPINGS.to_vec();
        sorted.sort();
        assert_eq!(SDK_METHOD_MAPPINGS, sorted);
        assert!(
            SDK_METHOD_MAPPINGS
                .windows(2)
                .all(|window| window[0] != window[1])
        );
    }

    #[test]
    fn contains_known_terraform_entrypoints() {
        assert_mapping_contains(
            "resource",
            "aws_s3_bucket",
            "create",
            TerraformProviderAwsApiMethodRef {
                service: "s3",
                name: "CreateBucket",
            },
        );
        assert_mapping_contains(
            "resource",
            "aws_s3_bucket",
            "delete",
            TerraformProviderAwsApiMethodRef {
                service: "s3",
                name: "DeleteBucket",
            },
        );
        assert_mapping_contains(
            "resource",
            "aws_s3_directory_bucket",
            "create",
            TerraformProviderAwsApiMethodRef {
                service: "s3",
                name: "CreateBucket",
            },
        );
        assert_mapping_contains(
            "action",
            "aws_ec2_stop_instance",
            "invoke",
            TerraformProviderAwsApiMethodRef {
                service: "ec2",
                name: "StopInstances",
            },
        );
    }

    fn assert_mapping_contains(
        kind: &str,
        type_name: &str,
        action: &str,
        expected_api_method: TerraformProviderAwsApiMethodRef,
    ) {
        let matches: Vec<_> = SDK_METHOD_MAPPINGS
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
        let mapping = matches[0];
        assert!(mapping.api_methods.contains(&expected_api_method));
    }
}
