use std::{collections::BTreeMap, error::Error, fmt::Write as _};

use serde::Deserialize;

#[derive(Deserialize)]
struct Data {
    packages: Vec<PackageHistory>,
}

#[derive(Deserialize)]
struct PackageHistory {
    package: String,
    releases: Vec<Release>,
}

#[derive(Deserialize)]
struct Release {
    version: String,
    remove: Vec<Remove>,
    upsert: Vec<Mapping>,
}

#[derive(Deserialize)]
struct Remove(Option<String>, String);

#[derive(Deserialize)]
struct Mapping {
    receiver: Option<String>,
    method: String,
    api_methods: Vec<ApiMethod>,
}

#[derive(Deserialize)]
struct ApiMethod {
    service: String,
    name: String,
}

type MappingKey = (Option<String>, String);

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
struct MappingValue {
    receiver: Option<String>,
    method: String,
    api_methods: Vec<ApiMethodValue>,
}

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
struct ApiMethodValue {
    service: String,
    name: String,
}

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
struct Version(u64, u64, u64);

impl Version {
    fn parse(raw: &str) -> Result<Self, Box<dyn Error>> {
        let mut parts = raw.split('.');
        let numbers = [
            parts.next().ok_or("missing major version")?,
            parts.next().ok_or("missing minor version")?,
            parts.next().ok_or("missing patch version")?,
        ];
        if parts.next().is_some()
            || numbers.iter().any(|part| {
                part.is_empty()
                    || (part.len() > 1 && part.starts_with('0'))
                    || !part.bytes().all(|b| b.is_ascii_digit())
            })
        {
            return Err(format!("invalid stable semantic version {raw:?}").into());
        }
        Ok(Self(
            numbers[0].parse()?,
            numbers[1].parse()?,
            numbers[2].parse()?,
        ))
    }
}

#[cfg(not(test))]
fn main() -> Result<(), Box<dyn Error>> {
    use std::{env, fs, path::PathBuf};

    println!("cargo:rerun-if-changed=data/mappings.json");
    let contents = fs::read_to_string("data/mappings.json")?;
    let generated = generate(&contents)?;
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR is not set")?);
    fs::write(out_dir.join("generated.rs"), generated)?;
    Ok(())
}

pub(crate) fn generate(contents: &str) -> Result<String, Box<dyn Error>> {
    let mut data: Data = serde_json::from_str(contents)?;
    data.packages
        .sort_by(|left, right| left.package.cmp(&right.package));

    let mut generated = String::new();
    let mut modules = Vec::new();
    let mut versions = Vec::new();
    let mut ranges = Vec::new();
    let mut lookup = Vec::new();
    let mut api_ids = BTreeMap::<Vec<ApiMethodValue>, usize>::new();
    let mut mapping_ids = BTreeMap::<(String, MappingValue), usize>::new();
    let mut release_index = 0;
    for package in &mut data.packages {
        if package.package.is_empty() {
            return Err("AWS SDK package name must not be empty".into());
        }
        let start = versions.len();
        let mut state = BTreeMap::<MappingKey, MappingValue>::new();
        let mut previous_version = None;
        for release in &package.releases {
            let version = Version::parse(&release.version)?;
            if previous_version
                .as_ref()
                .is_some_and(|previous| previous >= &version)
            {
                return Err(format!(
                    "package {} releases are not strictly increasing at {}",
                    package.package, release.version
                )
                .into());
            }
            previous_version = Some(version);
            apply_release(&package.package, release, &mut state)?;
            let mappings = state.values().collect::<Vec<_>>();
            let mut mapping_indices = Vec::with_capacity(mappings.len());
            for mapping in mappings {
                let mapping = (*mapping).clone();
                let key = (package.package.clone(), mapping.clone());
                let mapping_index = if let Some(index) = mapping_ids.get(&key) {
                    *index
                } else {
                    let mapping_index = mapping_ids.len();
                    let api_index = if let Some(index) = api_ids.get(&mapping.api_methods) {
                        *index
                    } else {
                        let api_index = api_ids.len();
                        api_ids.insert(mapping.api_methods.clone(), api_index);
                        let api_constant = format!("API_METHODS_{api_index}");
                        writeln!(
                            generated,
                            "static {api_constant}: &[AwsSdkJsV3ApiMethod] = &["
                        )?;
                        for api_method in &mapping.api_methods {
                            writeln!(
                                generated,
                                "    AwsSdkJsV3ApiMethod {{ service: {}, name: {} }},",
                                quoted(&api_method.service),
                                quoted(&api_method.name),
                            )?;
                        }
                        writeln!(generated, "];\n")?;
                        api_index
                    };
                    let mapping_constant = format!("MAPPING_{mapping_index}");
                    writeln!(
                        generated,
                        "static {mapping_constant}: AwsSdkJsV3MethodMapping = AwsSdkJsV3MethodMapping {{ package: {}, receiver: {}, method: {}, api_methods: API_METHODS_{api_index} }};",
                        quoted(&package.package),
                        option_string(mapping.receiver.as_deref()),
                        quoted(&mapping.method),
                    )?;
                    mapping_ids.insert(key, mapping_index);
                    mapping_index
                };
                mapping_indices.push(mapping_index);
            }
            let mapping_constant = format!("RELEASE_{release_index}_MAPPINGS");
            writeln!(
                generated,
                "static {mapping_constant}: &[AwsSdkJsV3MethodMapping] = &["
            )?;
            for mapping_index in mapping_indices {
                writeln!(generated, "    MAPPING_{mapping_index},")?;
            }
            writeln!(generated, "];\n")?;
            versions.push(release.version.as_str());
            lookup.push((
                package.package.as_str(),
                release.version.as_str(),
                release_index,
            ));
            release_index += 1;
        }
        if package.releases.is_empty() {
            return Err(format!("package {} contains no stable releases", package.package).into());
        }
        modules.push(package.package.as_str());
        ranges.push((package.package.as_str(), start, package.releases.len()));
    }

    writeln!(
        generated,
        "pub static SERVICE_MODULES: &[&str] = &{};",
        string_slice(&modules)
    )?;
    writeln!(
        generated,
        "static MODULE_VERSIONS: &[&str] = &{};",
        string_slice(&versions)
    )?;
    writeln!(
        generated,
        "static MODULE_VERSION_RANGES: &[(&str, usize, usize)] = &["
    )?;
    for (package, start, len) in ranges {
        writeln!(generated, "    ({}, {start}, {len}),", quoted(package))?;
    }
    writeln!(generated, "];")?;
    writeln!(
        generated,
        "static VERSION_LOOKUP: &[(&str, &str, &[AwsSdkJsV3MethodMapping])] = &["
    )?;
    for (package, version, index) in lookup {
        writeln!(
            generated,
            "    ({}, {}, RELEASE_{index}_MAPPINGS),",
            quoted(package),
            quoted(version)
        )?;
    }
    writeln!(generated, "];")?;
    Ok(generated)
}

fn apply_release(
    package: &str,
    release: &Release,
    state: &mut BTreeMap<MappingKey, MappingValue>,
) -> Result<(), Box<dyn Error>> {
    let mut removals = release
        .remove
        .iter()
        .map(|remove| (remove.0.clone(), remove.1.clone()))
        .collect::<Vec<_>>();
    removals.sort();
    if removals.windows(2).any(|window| window[0] == window[1]) {
        return Err(format!(
            "package {package} release {} has duplicate removals",
            release.version
        )
        .into());
    }
    for key in removals {
        if state.remove(&key).is_none() {
            return Err(format!(
                "package {package} release {} removes missing mapping",
                release.version
            )
            .into());
        }
    }
    let mut upserts = release
        .upsert
        .iter()
        .map(|mapping| {
            let value = MappingValue {
                receiver: mapping.receiver.clone(),
                method: mapping.method.clone(),
                api_methods: mapping
                    .api_methods
                    .iter()
                    .map(|api| ApiMethodValue {
                        service: api.service.clone(),
                        name: api.name.clone(),
                    })
                    .collect(),
            };
            ((mapping.receiver.clone(), mapping.method.clone()), value)
        })
        .collect::<Vec<_>>();
    upserts.sort_by(|left, right| left.0.cmp(&right.0));
    if upserts.windows(2).any(|window| window[0].0 == window[1].0) {
        return Err(format!(
            "package {package} release {} has duplicate upserts",
            release.version
        )
        .into());
    }
    for (key, value) in upserts {
        if value.method.is_empty() || value.api_methods.is_empty() {
            return Err(format!(
                "package {package} release {} has an invalid mapping",
                release.version
            )
            .into());
        }
        state.insert(key, value);
    }
    Ok(())
}
fn quoted(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn option_string(value: Option<&str>) -> String {
    value.map_or_else(
        || "None".to_owned(),
        |value| format!("Some({})", quoted(value)),
    )
}

fn string_slice(values: &[&str]) -> String {
    let values = values.iter().map(|value| quoted(value)).collect::<Vec<_>>();
    format!("[{}]", values.join(", "))
}
