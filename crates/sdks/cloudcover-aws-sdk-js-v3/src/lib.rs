pub const SDK_NAME: &str = "aws-sdk-js-v3";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AwsSdkJsV3ApiMethod {
    pub service: &'static str,
    pub name: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AwsSdkJsV3MethodMapping {
    pub package: &'static str,
    pub receiver: Option<&'static str>,
    pub method: &'static str,
    pub api_methods: &'static [AwsSdkJsV3ApiMethod],
}

include!(concat!(env!("OUT_DIR"), "/generated.rs"));

#[must_use]
pub const fn service_modules() -> &'static [&'static str] {
    SERVICE_MODULES
}

#[must_use]
pub fn service_versions(package: &str) -> Option<&'static [&'static str]> {
    lookup_service_versions(package, MODULE_VERSION_RANGES, MODULE_VERSIONS)
}

/// A borrowed view over pooled mapping rows. This replaces the previous static
/// slice; use `iter()` for borrowed rows or collect explicitly for an owned list.
#[derive(Clone, Copy, Debug)]
pub struct AwsSdkJsV3Mappings {
    ids: &'static [u32],
}

impl AwsSdkJsV3Mappings {
    #[must_use]
    #[inline]
    pub fn iter(&self) -> AwsSdkJsV3MappingIter {
        AwsSdkJsV3MappingIter {
            ids: self.ids.iter(),
        }
    }
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&'static AwsSdkJsV3MethodMapping> {
        self.ids.get(index).map(|id| &MAPPINGS[*id as usize])
    }
    /// Explicitly materialize the previous contiguous representation when needed.
    #[must_use]
    pub fn to_vec(&self) -> Vec<AwsSdkJsV3MethodMapping> {
        self.iter().copied().collect()
    }
    #[must_use]
    pub const fn len(&self) -> usize {
        self.ids.len()
    }
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

impl std::ops::Index<usize> for AwsSdkJsV3Mappings {
    type Output = AwsSdkJsV3MethodMapping;
    fn index(&self, index: usize) -> &Self::Output {
        &MAPPINGS[self.ids[index] as usize]
    }
}

#[derive(Clone, Debug)]
pub struct AwsSdkJsV3MappingIter {
    ids: std::slice::Iter<'static, u32>,
}
impl Iterator for AwsSdkJsV3MappingIter {
    type Item = &'static AwsSdkJsV3MethodMapping;
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.ids.next().map(|id| &MAPPINGS[*id as usize])
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.ids.size_hint()
    }
}
impl ExactSizeIterator for AwsSdkJsV3MappingIter {}
impl std::iter::FusedIterator for AwsSdkJsV3MappingIter {}
impl IntoIterator for AwsSdkJsV3Mappings {
    type Item = &'static AwsSdkJsV3MethodMapping;
    type IntoIter = AwsSdkJsV3MappingIter;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl IntoIterator for &AwsSdkJsV3Mappings {
    type Item = &'static AwsSdkJsV3MethodMapping;
    type IntoIter = AwsSdkJsV3MappingIter;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[must_use]
#[inline]
pub fn service_method_mappings(package: &str, version: &str) -> Option<AwsSdkJsV3Mappings> {
    let range = MODULE_VERSION_RANGES
        .binary_search_by(|(candidate, _, _)| candidate.cmp(&package))
        .ok()?;
    let (_, start, len) = MODULE_VERSION_RANGES[range];
    let versions = &MODULE_VERSIONS[start..start + len];
    let key = version_numbers(version)?;
    let index = versions
        .binary_search_by_key(&key, |v| version_numbers(v).unwrap_or_default())
        .ok()?;
    // Numeric equality alone must never accept an unrecorded spelling.
    if versions[index] != version {
        return None;
    }
    Some(AwsSdkJsV3Mappings {
        ids: SNAPSHOTS[RELEASE_SNAPSHOTS[start + index] as usize],
    })
}

#[inline]
fn version_numbers(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.');
    let result = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    if parts.next().is_some() {
        return None;
    }
    Some(result)
}

fn lookup_service_versions(
    package: &str,
    ranges: &'static [(&'static str, usize, usize)],
    versions: &'static [&'static str],
) -> Option<&'static [&'static str]> {
    let range_index = ranges
        .binary_search_by(|(candidate, _, _)| candidate.cmp(&package))
        .ok()?;
    let (_, start, len) = ranges[range_index];
    Some(&versions[start..start + len])
}
