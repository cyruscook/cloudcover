use std::iter::FusedIterator;

#[allow(dead_code)]
#[cfg(test)]
#[path = "../build.rs"]
mod build;

pub const SDK_NAME: &str = "aws-sdk-go-v2";

static MAPPING_INDEX: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sdk_mappings.bin"));

const MAGIC: &[u8; 8] = b"CCGO002\0";
const HEADER_LEN: usize = 36;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AwsSdkGoV2ApiMethodRef {
    pub service: &'static str,
    pub name: &'static str,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AwsSdkGoV2ApiMethods {
    next: u32,
    end: u32,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AwsSdkGoV2MethodMapping {
    pub package: &'static str,
    pub receiver: &'static str,
    pub method: &'static str,
    pub api_methods: AwsSdkGoV2ApiMethods,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AwsSdkGoV2Mappings {
    row_ids: Vec<u16>,
    next: usize,
}

#[derive(Clone, Copy)]
struct VersionIndex {
    checkpoint_start: u32,
    checkpoint_len: u32,
    delta_start: u32,
    delta_len: u32,
}

include!(concat!(env!("OUT_DIR"), "/sdk_mappings.rs"));

/// AWS service Go module paths with generated mappings.
#[must_use]
pub const fn service_modules() -> &'static [&'static str] {
    SERVICE_MODULES
}

/// Stable versions available for one exact AWS service Go module.
#[must_use]
pub fn service_versions(module_path: &str) -> Option<&'static [&'static str]> {
    let range_index = MODULE_VERSION_RANGES
        .binary_search_by(|(candidate, _, _)| candidate.cmp(&module_path))
        .ok()?;
    let (_, start, len) = MODULE_VERSION_RANGES[range_index];
    Some(&MODULE_VERSIONS[start..start + len])
}

/// Returns mappings generated from one exact AWS service module release.
#[must_use]
pub fn service_method_mappings(module_path: &str, version: &str) -> Option<AwsSdkGoV2Mappings> {
    let lookup_index = VERSION_LOOKUP
        .binary_search_by(|(candidate_module, candidate_version, _)| {
            (*candidate_module, *candidate_version).cmp(&(module_path, version))
        })
        .ok()?;
    let version_index = VERSION_INDEX[VERSION_LOOKUP[lookup_index].2];
    Some(AwsSdkGoV2Mappings {
        row_ids: reconstruct_row_ids(version_index)?,
        next: 0,
    })
}

impl AwsSdkGoV2Mappings {
    #[must_use]
    pub fn iter(&self) -> Self {
        self.clone()
    }
}

impl IntoIterator for &AwsSdkGoV2Mappings {
    type Item = AwsSdkGoV2MethodMapping;
    type IntoIter = AwsSdkGoV2Mappings;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl Iterator for AwsSdkGoV2Mappings {
    type Item = AwsSdkGoV2MethodMapping;

    fn next(&mut self) -> Option<Self::Item> {
        let row_id = *self.row_ids.get(self.next)?;
        self.next += 1;
        let row_offset = ROWS_OFFSET + row_id as usize * 8;
        let package = string(read_u16(row_offset));
        let receiver = string(read_u16(row_offset + 2));
        let method = string(read_u16(row_offset + 4));
        let api_list = read_u16(row_offset + 6);
        let api_list_offset = API_LISTS_OFFSET + api_list as usize * 8;
        let api_start = read_u32(api_list_offset);
        let api_len = read_u32(api_list_offset + 4);
        Some(AwsSdkGoV2MethodMapping {
            package,
            receiver,
            method,
            api_methods: AwsSdkGoV2ApiMethods {
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
impl ExactSizeIterator for AwsSdkGoV2Mappings {
    fn len(&self) -> usize {
        self.row_ids.len() - self.next
    }
}

impl FusedIterator for AwsSdkGoV2Mappings {}

impl AwsSdkGoV2ApiMethods {
    #[must_use]
    pub fn iter(&self) -> Self {
        self.clone()
    }

    #[must_use]
    pub fn contains(&self, expected: &AwsSdkGoV2ApiMethodRef) -> bool {
        self.iter().any(|api_method| api_method == *expected)
    }
}

impl IntoIterator for &AwsSdkGoV2ApiMethods {
    type Item = AwsSdkGoV2ApiMethodRef;
    type IntoIter = AwsSdkGoV2ApiMethods;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl Iterator for AwsSdkGoV2ApiMethods {
    type Item = AwsSdkGoV2ApiMethodRef;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            return None;
        }
        let offset = API_METHODS_OFFSET + self.next as usize * 4;
        self.next += 1;
        Some(AwsSdkGoV2ApiMethodRef {
            service: string(read_u16(offset)),
            name: string(read_u16(offset + 2)),
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}

impl ExactSizeIterator for AwsSdkGoV2ApiMethods {
    fn len(&self) -> usize {
        (self.end - self.next) as usize
    }
}
fn reconstruct_row_ids(version: VersionIndex) -> Option<Vec<u16>> {
    if MAPPING_INDEX.get(..MAGIC.len())? != MAGIC || MAPPING_INDEX.len() < HEADER_LEN {
        return None;
    }
    let string_count = read_u32(8) as usize;
    let api_method_count = read_u32(12) as usize;
    let api_list_count = read_u32(16) as usize;
    let row_count = read_u32(20) as usize;
    let checkpoint_row_count = read_u32(24) as usize;
    let delta_operation_count = read_u32(28) as usize;
    let version_count = read_u32(32) as usize;
    let api_lists_offset = checked_section_end(HEADER_LEN, api_method_count, 4)?;
    let rows_offset = checked_section_end(api_lists_offset, api_list_count, 8)?;
    let checkpoint_rows_offset = checked_section_end(rows_offset, row_count, 8)?;
    let delta_operations_offset =
        checked_section_end(checkpoint_rows_offset, checkpoint_row_count, 2)?;
    let binary_len = checked_section_end(delta_operations_offset, delta_operation_count, 3)?;
    if string_count != STRINGS.len()
        || version_count != VERSION_INDEX.len()
        || API_METHODS_OFFSET != HEADER_LEN
        || API_LISTS_OFFSET != api_lists_offset
        || ROWS_OFFSET != rows_offset
        || CHECKPOINT_ROWS_OFFSET != checkpoint_rows_offset
        || DELTA_OPERATIONS_OFFSET != delta_operations_offset
        || MAPPING_INDEX.len() != binary_len
    {
        return None;
    }

    let checkpoint_start = version.checkpoint_start as usize;
    let checkpoint_end = checkpoint_start.checked_add(version.checkpoint_len as usize)?;
    if checkpoint_end > checkpoint_row_count {
        return None;
    }
    let delta_start = version.delta_start as usize;
    let delta_end = delta_start.checked_add(version.delta_len as usize)?;
    if delta_end > delta_operation_count {
        return None;
    }

    let mut rows = Vec::with_capacity(version.checkpoint_len as usize);
    for offset in checkpoint_start..checkpoint_end {
        let row_id = read_u16(CHECKPOINT_ROWS_OFFSET + offset * 2);
        if !valid_row_id(row_id, row_count, string_count, api_list_count) {
            return None;
        }
        if rows
            .last()
            .is_some_and(|previous| compare_row_ids(*previous, row_id).is_ge())
        {
            return None;
        }
        rows.push(row_id);
    }
    for offset in delta_start..delta_end {
        let operation_offset = DELTA_OPERATIONS_OFFSET + offset * 3;
        let row_id = read_u16(operation_offset);
        if !valid_row_id(row_id, row_count, string_count, api_list_count) {
            return None;
        }
        match (
            MAPPING_INDEX[operation_offset + 2],
            rows.binary_search_by(|candidate| compare_row_ids(*candidate, row_id)),
        ) {
            (0, Ok(index)) => {
                rows.remove(index);
            }
            (1, Err(index)) => {
                rows.insert(index, row_id);
            }
            _ => return None,
        }
    }
    Some(rows)
}

fn valid_row_id(row_id: u16, row_count: usize, string_count: usize, api_list_count: usize) -> bool {
    if row_id as usize >= row_count {
        return false;
    }
    let row_offset = ROWS_OFFSET + row_id as usize * 8;
    (read_u16(row_offset) as usize) < string_count
        && (read_u16(row_offset + 2) as usize) < string_count
        && (read_u16(row_offset + 4) as usize) < string_count
        && (read_u16(row_offset + 6) as usize) < api_list_count
}

fn compare_row_ids(left: u16, right: u16) -> std::cmp::Ordering {
    let left_offset = ROWS_OFFSET + left as usize * 8;
    let right_offset = ROWS_OFFSET + right as usize * 8;
    (
        string(read_u16(left_offset)),
        string(read_u16(left_offset + 2)),
        string(read_u16(left_offset + 4)),
    )
        .cmp(&(
            string(read_u16(right_offset)),
            string(read_u16(right_offset + 2)),
            string(read_u16(right_offset + 4)),
        ))
}

fn checked_section_end(start: usize, count: usize, width: usize) -> Option<usize> {
    count.checked_mul(width)?.checked_add(start)
}

impl FusedIterator for AwsSdkGoV2ApiMethods {}

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
        AwsSdkGoV2ApiMethodRef, service_method_mappings, service_modules, service_versions,
    };

    #[test]
    fn service_modules_and_versions_are_sorted_and_unique() -> Result<(), &'static str> {
        if service_modules().is_empty() {
            return Err("missing service modules");
        }
        assert!(
            service_modules()
                .windows(2)
                .all(|window| window[0] < window[1])
        );
        for module_path in service_modules() {
            let versions = service_versions(module_path).ok_or("missing indexed service module")?;
            assert!(!versions.is_empty());
            assert!(versions.windows(2).all(|window| window[0] != window[1]));
        }
        Ok(())
    }

    #[test]
    fn mappings_and_api_methods_are_sorted_and_unique() -> Result<(), &'static str> {
        for module_path in service_modules() {
            for version in service_versions(module_path).ok_or("missing module versions")? {
                let mappings = service_method_mappings(module_path, version)
                    .ok_or("missing indexed module version")?;
                let rows = mappings.collect::<Vec<_>>();
                assert!(rows.windows(2).all(|window| window[0] < window[1]));
                for mapping in rows {
                    let api_methods = mapping.api_methods.collect::<Vec<_>>();
                    assert!(api_methods.windows(2).all(|window| window[0] < window[1]));
                }
            }
        }
        Ok(())
    }

    #[test]
    fn exact_service_lookup_retrieves_known_version_only() -> Result<(), &'static str> {
        let module_path = "github.com/aws/aws-sdk-go-v2/service/s3";
        let mappings = service_method_mappings(module_path, "1.104.0")
            .ok_or("missing S3 v1.104.0 mappings")?
            .collect::<Vec<_>>();
        assert!(mappings.iter().any(|mapping| {
            mapping.package == module_path
                && mapping.method == "GetObject"
                && mapping.api_methods.contains(&AwsSdkGoV2ApiMethodRef {
                    service: "s3",
                    name: "GetObject",
                })
        }));
        assert!(service_method_mappings(module_path, "1.104").is_none());
        assert!(service_method_mappings(module_path, "v1.104.0").is_none());
        assert!(service_method_mappings(module_path, "1.104.0+local").is_none());
        assert!(service_method_mappings("github.com/aws/aws-sdk-go-v2", "1.42.0").is_none());
        Ok(())
    }

    #[test]
    fn exact_service_version_controls_added_operations() -> Result<(), &'static str> {
        let module_path = "github.com/aws/aws-sdk-go-v2/service/connect";
        let before = service_method_mappings(module_path, "1.192.1")
            .ok_or("missing Connect v1.192.1 mappings")?;
        let added = service_method_mappings(module_path, "1.193.0")
            .ok_or("missing Connect v1.193.0 mappings")?;
        assert!(
            !before
                .into_iter()
                .any(|mapping| mapping.method == "GetCrossRegionRouting")
        );
        assert!(added.into_iter().any(|mapping| {
            mapping.method == "GetCrossRegionRouting"
                && mapping.api_methods.contains(&AwsSdkGoV2ApiMethodRef {
                    service: "connect",
                    name: "GetCrossRegionRouting",
                })
        }));
        Ok(())
    }

    #[test]
    fn historic_and_current_stable_versions_are_indexed() -> Result<(), &'static str> {
        let module_path = "github.com/aws/aws-sdk-go-v2/service/s3";
        let versions = service_versions(module_path).ok_or("missing S3 versions")?;
        assert!(versions.contains(&"0.1.0"));
        assert!(versions.contains(&"1.104.0"));
        assert!(service_method_mappings(module_path, "0.1.0").is_some());
        assert!(service_method_mappings(module_path, "1.104.0").is_some());
        Ok(())
    }
}
