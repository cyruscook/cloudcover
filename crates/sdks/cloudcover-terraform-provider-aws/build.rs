use std::{
    collections::{BTreeMap, hash_map::DefaultHasher},
    env,
    error::Error,
    fmt::{self, Write as _},
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    process::{Command, Output},
};

use cloudcover_aws_sdk_go_v2::SDK_METHOD_MAPPINGS;
use serde::{Deserialize, Serialize};

const PROVIDER_REPOSITORY: &str = "https://github.com/hashicorp/terraform-provider-aws";
const PROVIDER_BRANCH: &str = "main";
const MINIMUM_GO_MINOR: u32 = 26;
const CACHE_DIR_NAME: &str = "cloudcover-build-cache";
const CACHE_FILE_NAME: &str = "terraform_provider_aws_mappings.rs";
const PROVIDER_CHECKOUT_DIR_NAME: &str = "terraform-provider-aws";
const REFRESH_ENV: &str = "CLOUDCOVER_TERRAFORM_PROVIDER_AWS_REFRESH";

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
struct ApiMethodRefRow {
    service: String,
    name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
struct TerraformProviderAwsMethodMappingRow {
    kind: String,
    type_name: String,
    action: String,
    api_methods: Vec<ApiMethodRefRow>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
struct AwsSdkGoV2MappingRow {
    package: &'static str,
    receiver: &'static str,
    method: &'static str,
    api_methods: Vec<ApiMethodRefRow>,
}

type BuildResult<T> = Result<T, Box<dyn Error>>;

fn main() -> BuildResult<()> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=generator/go.mod");
    println!("cargo:rerun-if-changed=generator/go.sum");
    println!("cargo:rerun-if-changed=generator/main.go");
    println!("cargo:rerun-if-env-changed={REFRESH_ENV}");

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let cache_dir = shared_cache_dir(&out_dir)?.join("cloudcover-terraform-provider-aws");
    fs::create_dir_all(&cache_dir)?;

    let aws_sdk_map_json = serde_json::to_string(&aws_sdk_rows())?;
    let aws_sdk_map_hash = stable_hash(&aws_sdk_map_json);
    let refresh = env_var_requested(REFRESH_ENV);
    let cache_key = format!("{}-{}", generator_fingerprint()?, aws_sdk_map_hash);
    let cached_output_path = cache_dir.join(format!("{cache_key}-{CACHE_FILE_NAME}"));
    if !refresh && cached_output_path.exists() {
        let generated = fs::read_to_string(&cached_output_path)?;
        write_if_changed(&out_dir.join(CACHE_FILE_NAME), &generated)?;
        return Ok(());
    }

    ensure_command_available(
        "git",
        &[
            "git is required to clone github.com/hashicorp/terraform-provider-aws during build.",
            "Install git and retry.",
        ],
    )?;
    ensure_go_available()?;

    let provider_dir = cache_dir.join(PROVIDER_CHECKOUT_DIR_NAME);
    refresh_provider_checkout(&provider_dir, refresh)?;

    let sdk_map_json_path = provider_dir.join("cloudcover-aws-sdk-go-v2-mappings.json");
    let mut rows = load_rows(&provider_dir, &sdk_map_json_path, &aws_sdk_map_json)?;
    validate_and_normalize_rows(&mut rows)?;

    let generated = generate_code(&rows)?;
    write_if_changed(&cached_output_path, &generated)?;
    if provider_dir.exists() {
        fs::remove_dir_all(&provider_dir)?;
    }
    write_if_changed(&out_dir.join(CACHE_FILE_NAME), &generated)?;

    Ok(())
}

fn ensure_command_available(command: &str, missing_help: &[&str]) -> BuildResult<()> {
    match Command::new(command).arg("--version").output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(format_command_failure(command, &output, missing_help).into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(format_missing_executable(command, missing_help).into())
        }
        Err(error) => Err(format!("failed to execute {command}: {error}").into()),
    }
}

fn ensure_go_available() -> BuildResult<()> {
    let help = [
        "go is required to run the terraform-provider-aws analyzer during build.",
        "Go 1.26+ is required because github.com/hashicorp/terraform-provider-aws currently declares go 1.26.4.",
    ];
    let output = match Command::new("go").arg("version").output() {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(format_missing_executable("go", &help).into());
        }
        Err(error) => return Err(format!("failed to execute go: {error}").into()),
    };
    if !output.status.success() {
        return Err(format_command_failure("go", &output, &help).into());
    }

    let stdout = String::from_utf8(output.stdout)?;
    let version = stdout
        .split_whitespace()
        .nth(2)
        .ok_or_else(|| format!("unable to parse go version output: {stdout:?}"))?;
    let version = version
        .strip_prefix("go")
        .ok_or_else(|| format!("unexpected go version string: {version}"))?;
    let mut parts = version.split('.');
    let major = parts
        .next()
        .ok_or_else(|| format!("missing go major version in {version}"))?
        .parse::<u32>()?;
    let minor = parts
        .next()
        .ok_or_else(|| format!("missing go minor version in {version}"))?
        .parse::<u32>()?;
    if major > 1 || (major == 1 && minor >= MINIMUM_GO_MINOR) {
        return Ok(());
    }

    Err(format!(
        "go 1.{MINIMUM_GO_MINOR}+ is required because github.com/hashicorp/terraform-provider-aws currently declares go 1.26.4; found go{version}"
    )
    .into())
}

fn refresh_provider_checkout(provider_dir: &Path, refresh: bool) -> BuildResult<()> {
    if !provider_dir.join(".git").exists() {
        if provider_dir.exists() {
            fs::remove_dir_all(provider_dir)?;
        }
        return clone_provider_checkout(provider_dir);
    }

    if !refresh {
        return Ok(());
    }

    let fetch_output = Command::new("git")
        .args([
            "-C",
            provider_dir.to_str().ok_or("invalid provider path")?,
            "fetch",
            "--depth=1",
            "origin",
            PROVIDER_BRANCH,
        ])
        .output()?;
    if !fetch_output.status.success() {
        return Err(format_command_failure(
            "git fetch",
            &fetch_output,
            &["failed to refresh github.com/hashicorp/terraform-provider-aws"],
        )
        .into());
    }

    let reset_output = Command::new("git")
        .args([
            "-C",
            provider_dir.to_str().ok_or("invalid provider path")?,
            "reset",
            "--hard",
            "FETCH_HEAD",
        ])
        .output()?;
    if !reset_output.status.success() {
        return Err(format_command_failure(
            "git reset --hard",
            &reset_output,
            &["failed to update github.com/hashicorp/terraform-provider-aws checkout"],
        )
        .into());
    }

    let clean_output = Command::new("git")
        .args([
            "-C",
            provider_dir.to_str().ok_or("invalid provider path")?,
            "clean",
            "-fdx",
        ])
        .output()?;
    if clean_output.status.success() {
        Ok(())
    } else {
        Err(format_command_failure(
            "git clean -fdx",
            &clean_output,
            &["failed to clean github.com/hashicorp/terraform-provider-aws checkout"],
        )
        .into())
    }
}

fn clone_provider_checkout(provider_dir: &Path) -> BuildResult<()> {
    let output = Command::new("git")
        .args([
            "clone",
            "--depth=1",
            "--branch",
            PROVIDER_BRANCH,
            PROVIDER_REPOSITORY,
        ])
        .arg(provider_dir)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format_command_failure(
            "git clone",
            &output,
            &["failed to clone github.com/hashicorp/terraform-provider-aws"],
        )
        .into())
    }
}

fn load_rows(
    provider_dir: &Path,
    sdk_map_json_path: &Path,
    aws_sdk_map_json: &str,
) -> BuildResult<Vec<TerraformProviderAwsMethodMappingRow>> {
    write_if_changed(sdk_map_json_path, aws_sdk_map_json)?;

    let output = Command::new("go")
        .args([
            "run",
            "-mod=readonly",
            "./main.go",
            "--provider-dir",
            provider_dir.to_str().ok_or("invalid provider path")?,
            "--sdk-map-json",
            sdk_map_json_path.to_str().ok_or("invalid sdk map path")?,
        ])
        .current_dir("generator")
        .output()?;
    if !output.status.success() {
        return Err(format_command_failure(
            "go run",
            &output,
            &["failed to run terraform-provider-aws analyzer"],
        )
        .into());
    }

    let stdout = String::from_utf8(output.stdout)?;
    serde_json::from_str(&stdout).map_err(|error| {
        format!("failed to parse analyzer output as JSON: {error}\nstdout:\n{stdout}").into()
    })
}

fn validate_and_normalize_rows(
    rows: &mut Vec<TerraformProviderAwsMethodMappingRow>,
) -> BuildResult<()> {
    if rows.is_empty() {
        return Err("terraform-provider-aws analyzer returned no method mappings".into());
    }

    for row in rows.iter_mut() {
        if row.kind.is_empty() || row.type_name.is_empty() || row.action.is_empty() {
            return Err(
                "terraform-provider-aws mapping row has empty kind/type_name/action".into(),
            );
        }

        row.api_methods.sort();
        row.api_methods.dedup();

        for api_method in &row.api_methods {
            if api_method.service.is_empty() || api_method.name.is_empty() {
                return Err(format!(
                    "terraform-provider-aws mapping row {} {} {} has empty api_method fields",
                    row.kind, row.type_name, row.action
                )
                .into());
            }
        }
    }

    let mut by_key = BTreeMap::<(String, String, String), Vec<ApiMethodRefRow>>::new();
    for row in rows.iter() {
        let key = (row.kind.clone(), row.type_name.clone(), row.action.clone());
        match by_key.get(&key) {
            Some(existing) if existing != &row.api_methods => {
                return Err(format!(
                    "terraform-provider-aws mapping rows disagree for {} {} {}",
                    row.kind, row.type_name, row.action
                )
                .into());
            }
            Some(_) => {}
            None => {
                by_key.insert(key, row.api_methods.clone());
            }
        }
    }

    rows.sort_by(|left, right| {
        (
            left.kind.as_str(),
            left.type_name.as_str(),
            left.action.as_str(),
            &left.api_methods,
        )
            .cmp(&(
                right.kind.as_str(),
                right.type_name.as_str(),
                right.action.as_str(),
                &right.api_methods,
            ))
    });
    rows.dedup();

    Ok(())
}

fn generate_code(rows: &[TerraformProviderAwsMethodMappingRow]) -> Result<String, fmt::Error> {
    let mut generated = String::new();
    generated
        .push_str("pub const SDK_METHOD_MAPPINGS: &[TerraformProviderAwsMethodMapping] = &[\n");
    for row in rows {
        generated.push_str("    TerraformProviderAwsMethodMapping {\n");
        writeln!(generated, "        kind: {:?},", row.kind)?;
        writeln!(generated, "        type_name: {:?},", row.type_name)?;
        writeln!(generated, "        action: {:?},", row.action)?;
        generated.push_str("        api_methods: &[\n");
        for api_method in &row.api_methods {
            generated.push_str("            TerraformProviderAwsApiMethodRef { ");
            write!(
                generated,
                "service: {:?}, name: {:?}",
                api_method.service, api_method.name
            )?;
            generated.push_str(" },\n");
        }
        generated.push_str("        ],\n");
        generated.push_str("    },\n");
    }
    generated.push_str(
        "];
",
    );
    Ok(generated)
}

fn generator_fingerprint() -> BuildResult<String> {
    let mut hasher = DefaultHasher::new();
    PROVIDER_REPOSITORY.hash(&mut hasher);
    PROVIDER_BRANCH.hash(&mut hasher);
    for path in [
        "build.rs",
        "generator/go.mod",
        "generator/go.sum",
        "generator/main.go",
    ] {
        path.hash(&mut hasher);
        fs::read(path)?.hash(&mut hasher);
    }
    Ok(format!("{:016x}", hasher.finish()))
}

fn shared_cache_dir(out_dir: &Path) -> BuildResult<PathBuf> {
    if let Some(target_dir) = env::var_os("CARGO_TARGET_DIR") {
        return Ok(PathBuf::from(target_dir).join(CACHE_DIR_NAME));
    }

    let profile = env::var("PROFILE")?;
    let target = env::var("TARGET")?;
    let profile_dir = out_dir
        .ancestors()
        .find(|ancestor| {
            ancestor.file_name().and_then(|name| name.to_str()) == Some(profile.as_str())
        })
        .ok_or_else(|| {
            format!(
                "failed to infer target directory from {}",
                out_dir.display()
            )
        })?;
    let parent = profile_dir.parent().ok_or_else(|| {
        format!(
            "failed to infer target directory parent from {}",
            profile_dir.display()
        )
    })?;

    if parent.file_name().and_then(|name| name.to_str()) == Some(target.as_str()) {
        return Ok(parent
            .parent()
            .ok_or_else(|| format!("failed to infer target root from {}", out_dir.display()))?
            .join(CACHE_DIR_NAME));
    }

    Ok(parent.join(CACHE_DIR_NAME))
}

fn env_var_requested(name: &str) -> bool {
    env::var_os(name).is_some_and(|value| !value.is_empty() && value != "0")
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

fn aws_sdk_rows() -> Vec<AwsSdkGoV2MappingRow> {
    SDK_METHOD_MAPPINGS
        .iter()
        .map(|row| AwsSdkGoV2MappingRow {
            package: row.package,
            receiver: row.receiver,
            method: row.method,
            api_methods: row
                .api_methods
                .iter()
                .map(|api_method| ApiMethodRefRow {
                    service: api_method.service.to_owned(),
                    name: api_method.name.to_owned(),
                })
                .collect(),
        })
        .collect()
}

fn stable_hash(value: &str) -> String {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn format_missing_executable(command: &str, help: &[&str]) -> String {
    let mut message = format!("missing required executable `{command}`");
    for line in help {
        message.push('\n');
        message.push_str(line);
    }
    message
}

fn format_command_failure(command: &str, output: &Output, help: &[&str]) -> String {
    let mut message = format!("{command} failed with status {}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.trim().is_empty() {
        message.push_str("\nstdout:\n");
        message.push_str(stdout.trim_end());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.trim().is_empty() {
        message.push_str("\nstderr:\n");
        message.push_str(stderr.trim_end());
    }
    for line in help {
        message.push('\n');
        message.push_str(line);
    }
    message
}
