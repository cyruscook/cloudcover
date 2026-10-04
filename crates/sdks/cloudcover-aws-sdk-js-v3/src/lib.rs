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

#[must_use]
pub fn service_method_mappings(
    package: &str,
    version: &str,
) -> Option<&'static [AwsSdkJsV3MethodMapping]> {
    lookup_service_method_mappings(package, version, VERSION_LOOKUP)
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

fn lookup_service_method_mappings(
    package: &str,
    version: &str,
    lookup: &'static [(
        &'static str,
        &'static str,
        &'static [AwsSdkJsV3MethodMapping],
    )],
) -> Option<&'static [AwsSdkJsV3MethodMapping]> {
    let index = lookup
        .binary_search_by(|(candidate_package, candidate_version, _)| {
            (*candidate_package, *candidate_version).cmp(&(package, version))
        })
        .ok()?;
    Some(lookup[index].2)
}
