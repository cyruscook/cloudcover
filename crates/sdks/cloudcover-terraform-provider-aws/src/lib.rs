use std::{fmt, iter::FusedIterator};

pub const SDK_NAME: &str = "terraform-provider-aws";

static MAPPING_INDEX: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/terraform_provider_aws_mappings.bin"
));

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum TerraformProviderAwsUnsupportedReason {
    AwsSdkGoV1,
}

impl TerraformProviderAwsUnsupportedReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AwsSdkGoV1 => "aws_sdk_go_v1",
        }
    }
}

impl fmt::Display for TerraformProviderAwsUnsupportedReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum TerraformProviderAwsMappingsLookup {
    Supported(TerraformProviderAwsMappings),
    Unsupported(TerraformProviderAwsUnsupportedReason),
    Unknown,
}

#[derive(Clone, Copy, Debug)]
struct VersionIndex {
    start: u32,
    len: u32,
    unsupported_reason: Option<TerraformProviderAwsUnsupportedReason>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TerraformProviderAwsApiMethodRef {
    pub service: &'static str,
    pub name: &'static str,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TerraformProviderAwsApiMethods {
    next: u32,
    end: u32,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TerraformProviderAwsMethodMapping {
    pub kind: &'static str,
    pub type_name: &'static str,
    pub action: &'static str,
    pub api_methods: TerraformProviderAwsApiMethods,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TerraformProviderAwsMappings {
    next: u32,
    end: u32,
}
include!(concat!(
    env!("OUT_DIR"),
    "/terraform_provider_aws_mappings.rs"
));

#[must_use]
pub const fn provider_versions() -> &'static [&'static str] {
    PROVIDER_VERSIONS
}

#[must_use]
pub fn sdk_method_mappings(version: &str) -> TerraformProviderAwsMappingsLookup {
    let Some(lookup_index) = VERSION_LOOKUP
        .binary_search_by(|(candidate, _)| candidate.cmp(&version))
        .ok()
    else {
        return TerraformProviderAwsMappingsLookup::Unknown;
    };
    let version_index = VERSION_INDEX[VERSION_LOOKUP[lookup_index].1];
    match version_index.unsupported_reason {
        Some(reason) => TerraformProviderAwsMappingsLookup::Unsupported(reason),
        None => TerraformProviderAwsMappingsLookup::Supported(TerraformProviderAwsMappings {
            next: version_index.start,
            end: version_index.start + version_index.len,
        }),
    }
}

impl TerraformProviderAwsMappings {
    #[must_use]
    pub fn iter(&self) -> Self {
        self.clone()
    }
}
impl IntoIterator for &TerraformProviderAwsMappings {
    type Item = TerraformProviderAwsMethodMapping;
    type IntoIter = TerraformProviderAwsMappings;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl Iterator for TerraformProviderAwsMappings {
    type Item = TerraformProviderAwsMethodMapping;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            return None;
        }
        let row_id = read_u16(ROW_IDS_OFFSET + self.next as usize * 2);
        self.next += 1;
        let row_offset = ROWS_OFFSET + row_id as usize * 8;
        let kind = string(read_u16(row_offset));
        let type_name = string(read_u16(row_offset + 2));
        let action = string(read_u16(row_offset + 4));
        let api_list = read_u16(row_offset + 6);
        let api_list_offset = API_LISTS_OFFSET + api_list as usize * 8;
        let api_start = read_u32(api_list_offset);
        let api_len = read_u32(api_list_offset + 4);
        Some(TerraformProviderAwsMethodMapping {
            kind,
            type_name,
            action,
            api_methods: TerraformProviderAwsApiMethods {
                next: api_start,
                end: api_start + api_len,
            },
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}

impl ExactSizeIterator for TerraformProviderAwsMappings {
    fn len(&self) -> usize {
        (self.end - self.next) as usize
    }
}

impl FusedIterator for TerraformProviderAwsMappings {}

impl TerraformProviderAwsApiMethods {
    #[must_use]
    pub fn iter(&self) -> Self {
        self.clone()
    }

    #[must_use]
    pub fn contains(&self, expected: &TerraformProviderAwsApiMethodRef) -> bool {
        self.iter().any(|api_method| api_method == *expected)
    }
}
impl IntoIterator for &TerraformProviderAwsApiMethods {
    type Item = TerraformProviderAwsApiMethodRef;
    type IntoIter = TerraformProviderAwsApiMethods;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl Iterator for TerraformProviderAwsApiMethods {
    type Item = TerraformProviderAwsApiMethodRef;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            return None;
        }
        let offset = API_METHODS_OFFSET + self.next as usize * 4;
        self.next += 1;
        Some(TerraformProviderAwsApiMethodRef {
            service: string(read_u16(offset)),
            name: string(read_u16(offset + 2)),
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}

impl ExactSizeIterator for TerraformProviderAwsApiMethods {
    fn len(&self) -> usize {
        (self.end - self.next) as usize
    }
}

impl FusedIterator for TerraformProviderAwsApiMethods {}

fn string(index: u16) -> &'static str {
    STRINGS[index as usize]
}

fn read_u16(offset: usize) -> u16 {
    u16::from_le_bytes([MAPPING_INDEX[offset], MAPPING_INDEX[offset + 1]])
}

fn read_u32(offset: usize) -> u32 {
    u32::from_le_bytes([
        MAPPING_INDEX[offset],
        MAPPING_INDEX[offset + 1],
        MAPPING_INDEX[offset + 2],
        MAPPING_INDEX[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::{
        TerraformProviderAwsApiMethodRef, TerraformProviderAwsMappingsLookup,
        TerraformProviderAwsUnsupportedReason, provider_versions, sdk_method_mappings,
    };

    #[test]
    fn provider_versions_are_sorted_and_unique() {
        assert!(!provider_versions().is_empty());
        assert!(
            provider_versions()
                .windows(2)
                .all(|window| window[0] != window[1])
        );
    }

    #[test]
    fn mappings_and_api_methods_are_sorted_and_unique() -> Result<(), &'static str> {
        for version in provider_versions() {
            let mappings = match sdk_method_mappings(version) {
                TerraformProviderAwsMappingsLookup::Supported(mappings) => mappings,
                TerraformProviderAwsMappingsLookup::Unsupported(_) => continue,
                TerraformProviderAwsMappingsLookup::Unknown => {
                    return Err("missing indexed version");
                }
            };
            let rows = mappings.collect::<Vec<_>>();
            assert!(rows.windows(2).all(|window| {
                let left = (window[0].kind, window[0].type_name, window[0].action);
                let right = (window[1].kind, window[1].type_name, window[1].action);
                left < right
            }));
            for mapping in rows {
                let api_methods = mapping.api_methods.collect::<Vec<_>>();
                assert!(api_methods.windows(2).all(|window| window[0] < window[1]));
            }
        }
        Ok(())
    }

    #[test]
    fn exact_lookup_retrieves_known_version_only() -> Result<(), &'static str> {
        let TerraformProviderAwsMappingsLookup::Supported(mappings) = sdk_method_mappings("6.64.0")
        else {
            return Err("missing v6.64.0 mappings");
        };
        let mappings = mappings.collect::<Vec<_>>();
        assert_mapping_contains(
            &mappings,
            "resource",
            "aws_s3_bucket",
            "create",
            TerraformProviderAwsApiMethodRef {
                service: "s3",
                name: "CreateBucket",
            },
        );
        assert_mapping_contains(
            &mappings,
            "resource",
            "aws_s3_bucket",
            "delete",
            TerraformProviderAwsApiMethodRef {
                service: "s3",
                name: "DeleteBucket",
            },
        );
        assert_mapping_contains(
            &mappings,
            "list_resource",
            "aws_acm_certificate",
            "list",
            TerraformProviderAwsApiMethodRef {
                service: "acm",
                name: "ListCertificates",
            },
        );
        assert_mapping_contains(
            &mappings,
            "action",
            "aws_ec2_stop_instance",
            "invoke",
            TerraformProviderAwsApiMethodRef {
                service: "ec2",
                name: "StopInstances",
            },
        );
        assert!(matches!(
            sdk_method_mappings("1.56.0"),
            TerraformProviderAwsMappingsLookup::Unsupported(
                TerraformProviderAwsUnsupportedReason::AwsSdkGoV1
            )
        ));
        assert!(matches!(
            sdk_method_mappings("1.57.0"),
            TerraformProviderAwsMappingsLookup::Supported(_)
        ));
        assert!(matches!(
            sdk_method_mappings("7.0.0"),
            TerraformProviderAwsMappingsLookup::Unknown
        ));
        assert!(matches!(
            sdk_method_mappings("6.64"),
            TerraformProviderAwsMappingsLookup::Unknown
        ));
        assert!(matches!(
            sdk_method_mappings("v6.64.0"),
            TerraformProviderAwsMappingsLookup::Unknown
        ));
        assert!(matches!(
            sdk_method_mappings("6.64.0+local"),
            TerraformProviderAwsMappingsLookup::Unknown
        ));
        Ok(())
    }

    fn assert_mapping_contains(
        mappings: &[super::TerraformProviderAwsMethodMapping],
        kind: &str,
        type_name: &str,
        action: &str,
        expected_api_method: TerraformProviderAwsApiMethodRef,
    ) {
        let matches = mappings
            .iter()
            .filter(|mapping| {
                mapping.kind == kind && mapping.type_name == type_name && mapping.action == action
            })
            .collect::<Vec<_>>();
        assert_eq!(
            matches.len(),
            1,
            "missing mapping for {kind} {type_name} {action}"
        );
        assert!(matches[0].api_methods.contains(&expected_api_method));
    }
}
