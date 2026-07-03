use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fmt::{self, Write as _},
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use serde::Deserialize;

const SDK_REPOSITORY: &str = "https://github.com/aws/aws-sdk-go-v2";
const SDK_BRANCH: &str = "main";
const MINIMUM_GO_MINOR: u32 = 24;

// These rows are JSON objects emitted by generator/main.go. The Go analyzer
// discovers SDK method references. Then we validate, deduplicate, and bake
// them into a Rust static.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
struct ApiMethodRefRow {
    service: String,
    name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
struct SdkMethodMappingRow {
    package: String,
    receiver: String,
    method: String,
    api_methods: Vec<ApiMethodRefRow>,
}

type BuildResult<T> = Result<T, Box<dyn Error>>;

fn main() -> BuildResult<()> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=generator/go.mod");
    println!("cargo:rerun-if-changed=generator/go.sum");
    println!("cargo:rerun-if-changed=generator/main.go");
    println!("cargo:rerun-if-env-changed=CLOUDCOVER_AWS_GO_V2_REFRESH");

    ensure_command_available(
        "git",
        &[
            "git is required to clone github.com/aws/aws-sdk-go-v2 during build.",
            "Install git and retry.",
        ],
    )?;
    ensure_go_available()?;

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let sdk_dir = out_dir.join("aws-sdk-go-v2");
    refresh_sdk_checkout(&sdk_dir)?;

    let mut rows = load_rows(&sdk_dir)?;
    validate_and_normalize_rows(&mut rows)?;

    let generated = generate_code(&rows)?;
    fs::write(out_dir.join("sdk_mappings.rs"), generated)?;

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
        "go is required to run the aws-sdk-go-v2 analyzer during build.",
        "Go 1.24+ is required because github.com/aws/aws-sdk-go-v2 service modules declare go 1.24.",
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
        "go 1.{MINIMUM_GO_MINOR}+ is required because github.com/aws/aws-sdk-go-v2 service modules declare go 1.24; found go{version}"
    )
    .into())
}

fn refresh_sdk_checkout(sdk_dir: &Path) -> BuildResult<()> {
    if sdk_dir.exists() {
        fs::remove_dir_all(sdk_dir)?;
    }

    let output = Command::new("git")
        .args(["clone", "--depth=1", "--branch", SDK_BRANCH, SDK_REPOSITORY])
        .arg(sdk_dir)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format_command_failure(
            "git clone",
            &output,
            &["failed to clone github.com/aws/aws-sdk-go-v2"],
        )
        .into())
    }
}

fn load_rows(sdk_dir: &Path) -> BuildResult<Vec<SdkMethodMappingRow>> {
    let output = Command::new("go")
        .args(["run", "-mod=readonly", ".", "--sdk-dir"])
        .arg(sdk_dir)
        .current_dir("generator")
        .output()?;
    if !output.status.success() {
        return Err(format_command_failure(
            "go run",
            &output,
            &["failed to run aws-sdk-go-v2 analyzer"],
        )
        .into());
    }

    let stdout = String::from_utf8(output.stdout)?;
    serde_json::from_str(&stdout).map_err(|error| {
        format!("failed to parse analyzer output as JSON: {error}\nstdout:\n{stdout}").into()
    })
}

fn validate_and_normalize_rows(rows: &mut Vec<SdkMethodMappingRow>) -> BuildResult<()> {
    // The Go analyzer emits one row per discovered call edge. Collapse that to
    // the public method reference we expose from Rust, and fail fast if two
    // edges imply different API methods for the same package/receiver/method.
    if rows.is_empty() {
        return Err("aws-sdk-go-v2 analyzer returned no method mappings".into());
    }

    for row in rows.iter_mut() {
        if row.package.is_empty() {
            return Err("aws-sdk-go-v2 mapping row has empty package".into());
        }
        if row.receiver != "Client" {
            return Err(format!(
                "aws-sdk-go-v2 mapping row has unexpected receiver {} for {}.{}",
                row.receiver, row.package, row.method
            )
            .into());
        }
        if row.method.is_empty() {
            return Err(format!(
                "aws-sdk-go-v2 mapping row has empty method for package {}",
                row.package
            )
            .into());
        }
        if row.api_methods.is_empty() {
            return Err(format!(
                "aws-sdk-go-v2 mapping row {}.{} has no api_methods",
                row.package, row.method
            )
            .into());
        }

        row.api_methods.sort();
        row.api_methods.dedup();

        for api_method in &row.api_methods {
            if api_method.service.is_empty() || api_method.name.is_empty() {
                return Err(format!(
                    "aws-sdk-go-v2 mapping row {}.{} has empty api_method fields",
                    row.package, row.method
                )
                .into());
            }
        }
    }

    let mut by_key = BTreeMap::<(String, String, String), Vec<ApiMethodRefRow>>::new();
    for row in rows.iter() {
        let key = (
            row.package.clone(),
            row.receiver.clone(),
            row.method.clone(),
        );
        match by_key.get(&key) {
            Some(existing) if existing != &row.api_methods => {
                return Err(format!(
                    "aws-sdk-go-v2 mapping rows disagree for {} {}.{}",
                    row.package, row.receiver, row.method
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
            left.package.as_str(),
            left.receiver.as_str(),
            left.method.as_str(),
            &left.api_methods,
        )
            .cmp(&(
                right.package.as_str(),
                right.receiver.as_str(),
                right.method.as_str(),
                &right.api_methods,
            ))
    });
    rows.dedup();

    Ok(())
}

fn generate_code(rows: &[SdkMethodMappingRow]) -> Result<String, fmt::Error> {
    let mut generated = String::new();
    generated.push_str("pub const SDK_METHOD_MAPPINGS: &[AwsSdkGoV2MethodMapping] = &[\n");
    for row in rows {
        generated.push_str("    AwsSdkGoV2MethodMapping {\n");
        writeln!(generated, "        package: {:?},", row.package)?;
        writeln!(generated, "        receiver: {:?},", row.receiver)?;
        writeln!(generated, "        method: {:?},", row.method)?;
        generated.push_str("        api_methods: &[\n");
        for api_method in &row.api_methods {
            generated.push_str("            AwsSdkGoV2ApiMethodRef { ");
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
    generated.push_str("];\n");
    Ok(generated)
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
