pub const SDK_NAME: &str = "aws-sdk-go-v2";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AwsSdkGoV2ApiMethodRef {
    pub service: &'static str,
    pub name: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AwsSdkGoV2MethodMapping {
    pub package: &'static str,
    pub receiver: &'static str,
    pub method: &'static str,
    pub api_methods: &'static [AwsSdkGoV2ApiMethodRef],
}

include!(concat!(env!("OUT_DIR"), "/sdk_mappings.rs"));

#[cfg(test)]
mod tests {
    use super::{AwsSdkGoV2ApiMethodRef, AwsSdkGoV2MethodMapping, SDK_METHOD_MAPPINGS};

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
    fn contains_known_client_methods() {
        assert!(SDK_METHOD_MAPPINGS.contains(&AwsSdkGoV2MethodMapping {
            package: "github.com/aws/aws-sdk-go-v2/service/s3",
            receiver: "Client",
            method: "GetObject",
            api_methods: &[AwsSdkGoV2ApiMethodRef {
                service: "s3",
                name: "GetObject",
            }],
        }));
        assert!(SDK_METHOD_MAPPINGS.contains(&AwsSdkGoV2MethodMapping {
            package: "github.com/aws/aws-sdk-go-v2/service/ec2",
            receiver: "Client",
            method: "DescribeInstances",
            api_methods: &[AwsSdkGoV2ApiMethodRef {
                service: "ec2",
                name: "DescribeInstances",
            }],
        }));
    }
}
