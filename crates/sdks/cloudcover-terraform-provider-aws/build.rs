use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fmt::{self, Write as _},
    fs,
    path::{Path, PathBuf},
};

use semver::Version;

#[path = "src/data.rs"]
mod data;

use data::{PermissionDataFile, TerraformProviderAwsMethodMappingRow};
type BuildResult<T> = Result<T, Box<dyn Error>>;

fn main() -> BuildResult<()> {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let data_dir = manifest_dir.join("data");
    println!("cargo:rerun-if-changed={}", data_dir.display());
    println!("cargo:rerun-if-changed=src/data.rs");

    if !data_dir.is_dir() {
        return Err(format!(
            "permission data directory is missing: {}",
            data_dir.display()
        )
        .into());
    }

    let mut paths = fs::read_dir(&data_dir)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "json")
    });
    paths.sort();
    if paths.is_empty() {
        return Err(format!(
            "permission data directory has no JSON files: {}",
            data_dir.display()
        )
        .into());
    }
    for path in &paths {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let mut files = BTreeMap::<Version, PermissionDataFile>::new();
    for path in paths {
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| {
                format!(
                    "permission data filename is not valid UTF-8: {}",
                    path.display()
                )
            })?;
        let expected_version = data::parse_canonical_version(stem)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let contents = fs::read_to_string(&path)?;
        let mut file: PermissionDataFile = serde_json::from_str(&contents)
            .map_err(|error| format!("{}: invalid JSON: {error}", path.display()))?;
        let version = data::validate_and_normalize_file(&mut file, Some(stem))
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if version != expected_version {
            return Err(format!(
                "{}: parsed version {} does not match filename version {}",
                path.display(),
                version,
                expected_version
            )
            .into());
        }
        if files.insert(version.clone(), file).is_some() {
            return Err(format!("duplicate permission data version {version}").into());
        }
    }

    let generated = generate_code(&files)?;
    let out_path = PathBuf::from(env::var("OUT_DIR")?).join("terraform_provider_aws_mappings.rs");
    write_if_changed(&out_path, &generated)?;
    Ok(())
}

fn generate_code(files: &BTreeMap<Version, PermissionDataFile>) -> Result<String, fmt::Error> {
    let mut generated = String::new();
    generated
        .push_str("pub const PROVIDER_VERSIONS: &[TerraformProviderAwsVersionMappings] = &[\n");
    for (version, file) in files {
        writeln!(generated, "    TerraformProviderAwsVersionMappings {{")?;
        writeln!(generated, "        version: {:?},", version.to_string())?;
        generated.push_str("        mappings: &[\n");
        for row in &file.mappings {
            write_mapping(&mut generated, row)?;
        }
        generated.push_str("        ],\n    },\n");
    }
    generated.push_str("];\n");
    Ok(generated)
}

fn write_mapping(
    generated: &mut String,
    row: &TerraformProviderAwsMethodMappingRow,
) -> Result<(), fmt::Error> {
    generated.push_str("            TerraformProviderAwsMethodMapping {\n");
    writeln!(generated, "                kind: {:?},", row.kind)?;
    writeln!(generated, "                type_name: {:?},", row.type_name)?;
    writeln!(generated, "                action: {:?},", row.action)?;
    generated.push_str("                api_methods: &[\n");
    for api_method in &row.api_methods {
        writeln!(
            generated,
            "                    TerraformProviderAwsApiMethodRef {{ service: {:?}, name: {:?} }},",
            api_method.service, api_method.name
        )?;
    }
    generated.push_str("                ],\n            },\n");
    Ok(())
}

fn write_if_changed(path: &Path, contents: &str) -> BuildResult<()> {
    match fs::read_to_string(path) {
        Ok(existing) if existing == contents => Ok(()),
        Ok(_) | Err(_) => {
            fs::write(path, contents)?;
            Ok(())
        }
    }
}
