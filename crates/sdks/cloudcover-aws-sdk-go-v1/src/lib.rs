use std::iter::FusedIterator;

#[cfg(test)]
#[path = "build_index.rs"]
mod build_index;
#[cfg(test)]
#[path = "data.rs"]
mod data;

pub const SDK_NAME: &str = "aws-sdk-go-v1";
pub const MODULE_PATH: &str = "github.com/aws/aws-sdk-go";

static MAPPING_INDEX: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sdk_mappings.bin"));

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AwsSdkGoV1ApiMethodRef {
    pub service: &'static str,
    pub name: &'static str,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AwsSdkGoV1ApiMethods {
    next: u32,
    end: u32,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AwsSdkGoV1MethodMapping {
    pub package: &'static str,
    pub receiver: &'static str,
    pub method: &'static str,
    pub api_methods: AwsSdkGoV1ApiMethods,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AwsSdkGoV1Mappings {
    next: u32,
    end: u32,
}

#[derive(Clone, Copy)]
struct VersionIndex {
    start: u32,
    len: u32,
}

include!(concat!(env!("OUT_DIR"), "/sdk_mappings.rs"));

#[must_use]
pub const fn sdk_versions() -> &'static [&'static str] {
    SDK_VERSIONS
}

#[must_use]
pub fn sdk_method_mappings(version: &str) -> Option<AwsSdkGoV1Mappings> {
    let lookup_index = SDK_VERSIONS
        .iter()
        .position(|candidate| *candidate == version)?;
    let version_index = VERSION_INDEX[lookup_index];
    Some(AwsSdkGoV1Mappings {
        next: version_index.start,
        end: version_index.start + version_index.len,
    })
}

impl AwsSdkGoV1Mappings {
    #[must_use]
    pub fn iter(&self) -> Self {
        self.clone()
    }
}

impl IntoIterator for &AwsSdkGoV1Mappings {
    type Item = AwsSdkGoV1MethodMapping;
    type IntoIter = AwsSdkGoV1Mappings;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl Iterator for AwsSdkGoV1Mappings {
    type Item = AwsSdkGoV1MethodMapping;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            return None;
        }
        let row_id = read_u16(ROW_IDS_OFFSET + self.next as usize * 2);
        self.next += 1;
        let row_offset = ROWS_OFFSET + row_id as usize * 8;
        let api_list = read_u16(row_offset + 6);
        let api_list_offset = API_LISTS_OFFSET + api_list as usize * 8;
        let api_start = read_u32(api_list_offset);
        let api_len = read_u32(api_list_offset + 4);
        Some(AwsSdkGoV1MethodMapping {
            package: string(read_u16(row_offset)),
            receiver: string(read_u16(row_offset + 2)),
            method: string(read_u16(row_offset + 4)),
            api_methods: AwsSdkGoV1ApiMethods {
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

impl ExactSizeIterator for AwsSdkGoV1Mappings {
    fn len(&self) -> usize {
        (self.end - self.next) as usize
    }
}

impl FusedIterator for AwsSdkGoV1Mappings {}

impl AwsSdkGoV1ApiMethods {
    #[must_use]
    pub fn iter(&self) -> Self {
        self.clone()
    }

    #[must_use]
    pub fn contains(&self, expected: &AwsSdkGoV1ApiMethodRef) -> bool {
        self.iter().any(|api_method| api_method == *expected)
    }
}

impl IntoIterator for &AwsSdkGoV1ApiMethods {
    type Item = AwsSdkGoV1ApiMethodRef;
    type IntoIter = AwsSdkGoV1ApiMethods;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl Iterator for AwsSdkGoV1ApiMethods {
    type Item = AwsSdkGoV1ApiMethodRef;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            return None;
        }
        let offset = API_METHODS_OFFSET + self.next as usize * 4;
        self.next += 1;
        Some(AwsSdkGoV1ApiMethodRef {
            service: string(read_u16(offset)),
            name: string(read_u16(offset + 2)),
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}

impl ExactSizeIterator for AwsSdkGoV1ApiMethods {
    fn len(&self) -> usize {
        (self.end - self.next) as usize
    }
}

impl FusedIterator for AwsSdkGoV1ApiMethods {}

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
    use super::{sdk_method_mappings, sdk_versions, AwsSdkGoV1ApiMethodRef};

    #[test]
    fn versions_are_sorted_and_exact() {
        assert!(sdk_versions().windows(2).all(|window| {
            match (
                semver::Version::parse(window[0]),
                semver::Version::parse(window[1]),
            ) {
                (Ok(left), Ok(right)) => left < right,
                _ => false,
            }
        }));
        assert!(sdk_versions().contains(&"1.55.8"));
        assert!(sdk_method_mappings("1.55.8").is_some());
        assert!(sdk_method_mappings("v1.55.8").is_none());
        assert!(sdk_method_mappings("1.55").is_none());
    }

    #[test]
    fn exact_versions_control_operations() -> Result<(), &'static str> {
        let old = sdk_method_mappings("1.0.0").ok_or("missing v1.0.0")?;
        let current = sdk_method_mappings("1.55.8").ok_or("missing v1.55.8")?;
        assert!(!old
            .into_iter()
            .any(|row| row.package.ends_with("/accessanalyzer")));
        assert!(current.into_iter().any(|row| {
            row.package == "github.com/aws/aws-sdk-go/service/s3"
                && row.receiver == "S3"
                && row.method == "GetObject"
                && row.api_methods.contains(&AwsSdkGoV1ApiMethodRef {
                    service: "s3",
                    name: "GetObject",
                })
        }));
        Ok(())
    }

    #[test]
    fn mappings_are_sorted_and_unique() -> Result<(), &'static str> {
        for version in sdk_versions() {
            let rows = sdk_method_mappings(version)
                .ok_or("missing indexed version")?
                .collect::<Vec<_>>();
            assert!(rows.windows(2).all(|window| window[0] < window[1]));
        }
        Ok(())
    }
}
