use std::{error::Error, fmt::Write as _};

use serde::Deserialize;

#[derive(Deserialize)]
struct Data {
    packages: Vec<Release>,
}

#[derive(Deserialize)]
struct Release {
    package: String,
    version: String,
    mappings: Vec<Mapping>,
}

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
    data.packages.sort_by(|left, right| {
        (&left.package, &left.version).cmp(&(&right.package, &right.version))
    });
    if let Some(duplicates) = data.packages.windows(2).find(|packages| {
        packages[0].package == packages[1].package && packages[0].version == packages[1].version
    }) {
        return Err(format!(
            "duplicate AWS SDK for JavaScript package release {} {}",
            duplicates[0].package, duplicates[0].version
        )
        .into());
    }

    let mut generated = String::new();
    for (release_index, package) in data.packages.iter().enumerate() {
        let mapping_constant = format!("RELEASE_{release_index}_MAPPINGS");
        writeln!(
            generated,
            "static {mapping_constant}: &[AwsSdkJsV3MethodMapping] = &["
        )?;
        for (mapping_index, mapping) in package.mappings.iter().enumerate() {
            let api_constant = format!("RELEASE_{release_index}_API_{mapping_index}");
            writeln!(
                generated,
                "    AwsSdkJsV3MethodMapping {{ package: {}, receiver: {}, method: {}, api_methods: {api_constant} }},",
                quoted(&package.package),
                option_string(mapping.receiver.as_deref()),
                quoted(&mapping.method),
            )?;
        }
        writeln!(generated, "];\n")?;
        for (mapping_index, mapping) in package.mappings.iter().enumerate() {
            let api_constant = format!("RELEASE_{release_index}_API_{mapping_index}");
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
        }
    }

    let mut modules = Vec::new();
    let mut versions = Vec::new();
    let mut ranges = Vec::new();
    for releases in data
        .packages
        .chunk_by(|left, right| left.package == right.package)
    {
        let package = releases[0].package.as_str();
        modules.push(package);
        ranges.push((package, versions.len(), releases.len()));
        versions.extend(releases.iter().map(|release| release.version.as_str()));
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
    for (release_index, package) in data.packages.iter().enumerate() {
        writeln!(
            generated,
            "    ({}, {}, RELEASE_{release_index}_MAPPINGS),",
            quoted(&package.package),
            quoted(&package.version)
        )?;
    }
    writeln!(generated, "];")?;
    Ok(generated)
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
