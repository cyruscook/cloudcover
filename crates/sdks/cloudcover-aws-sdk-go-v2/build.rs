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

use serde::Deserialize;

const SDK_REPOSITORY: &str = "https://github.com/aws/aws-sdk-go-v2";
const SDK_BRANCH: &str = "main";
const MINIMUM_GO_MINOR: u32 = 24;
const CACHE_DIR_NAME: &str = "cloudcover-build-cache";
const CACHE_FILE_NAME: &str = "sdk_mappings.rs";
const SDK_CHECKOUT_DIR_NAME: &str = "aws-sdk-go-v2";
const REFRESH_ENV: &str = "CLOUDCOVER_AWS_GO_V2_REFRESH";

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
    println!("cargo:rerun-if-env-changed={REFRESH_ENV}");

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let cache_dir = shared_cache_dir(&out_dir)?.join("cloudcover-aws-sdk-go-v2");
    fs::create_dir_all(&cache_dir)?;

    let refresh = env_var_requested(REFRESH_ENV);
    let cache_key = generator_fingerprint()?;
    let cached_output_path = cache_dir.join(format!("{cache_key}-{CACHE_FILE_NAME}"));
    if !refresh && cached_output_path.exists() {
        let generated = fs::read_to_string(&cached_output_path)?;
        write_if_changed(&out_dir.join(CACHE_FILE_NAME), &generated)?;
        return Ok(());
    }

    ensure_command_available(
        "git",
        &[
            "git is required to clone github.com/aws/aws-sdk-go-v2 during build.",
            "Install git and retry.",
        ],
    )?;
    ensure_go_available()?;

    let sdk_dir = cache_dir.join(SDK_CHECKOUT_DIR_NAME);
    refresh_sdk_checkout(&sdk_dir, refresh)?;

    let mut rows = load_rows(&sdk_dir)?;
    validate_and_normalize_rows(&mut rows)?;

    let generated = generate_code(&rows)?;
    write_if_changed(&cached_output_path, &generated)?;
    if sdk_dir.exists() {
        fs::remove_dir_all(&sdk_dir)?;
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

fn refresh_sdk_checkout(sdk_dir: &Path, refresh: bool) -> BuildResult<()> {
    if !sdk_dir.join(".git").exists() {
        if sdk_dir.exists() {
            fs::remove_dir_all(sdk_dir)?;
        }
        return clone_sdk_checkout(sdk_dir);
    }

    if !refresh {
        return Ok(());
    }

    let fetch_output = Command::new("git")
        .args([
            "-C",
            sdk_dir.to_str().ok_or("invalid sdk path")?,
            "fetch",
            "--depth=1",
            "origin",
            SDK_BRANCH,
        ])
        .output()?;
    if !fetch_output.status.success() {
        return Err(format_command_failure(
            "git fetch",
            &fetch_output,
            &["failed to refresh github.com/aws/aws-sdk-go-v2"],
        )
        .into());
    }

    let reset_output = Command::new("git")
        .args([
            "-C",
            sdk_dir.to_str().ok_or("invalid sdk path")?,
            "reset",
            "--hard",
            "FETCH_HEAD",
        ])
        .output()?;
    if !reset_output.status.success() {
        return Err(format_command_failure(
            "git reset --hard",
            &reset_output,
            &["failed to update github.com/aws/aws-sdk-go-v2 checkout"],
        )
        .into());
    }

    let clean_output = Command::new("git")
        .args([
            "-C",
            sdk_dir.to_str().ok_or("invalid sdk path")?,
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
            &["failed to clean github.com/aws/aws-sdk-go-v2 checkout"],
        )
        .into())
    }
}

fn clone_sdk_checkout(sdk_dir: &Path) -> BuildResult<()> {
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
        .args(["run", "-mod=readonly", "./main.go", "--sdk-dir"])
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
        format!(
            "failed to parse analyzer output as JSON: {error}
stdout:
{stdout}"
        )
        .into()
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
    generated.push_str(
        "pub const SDK_METHOD_MAPPINGS: &[AwsSdkGoV2MethodMapping] = &[
",
    );
    for row in rows {
        generated.push_str(
            "    AwsSdkGoV2MethodMapping {
",
        );
        writeln!(generated, "        package: {:?},", row.package)?;
        writeln!(generated, "        receiver: {:?},", row.receiver)?;
        writeln!(generated, "        method: {:?},", row.method)?;
        generated.push_str(
            "        api_methods: &[
",
        );
        for api_method in &row.api_methods {
            generated.push_str("            AwsSdkGoV2ApiMethodRef { ");
            write!(
                generated,
                "service: {:?}, name: {:?}",
                api_method.service, api_method.name
            )?;
            generated.push_str(
                " },
",
            );
        }
        generated.push_str(
            "        ],
",
        );
        generated.push_str(
            "    },
",
        );
    }
    generated.push_str(
        "];
",
    );
    Ok(generated)
}

fn generator_fingerprint() -> BuildResult<String> {
    let mut hasher = DefaultHasher::new();
    SDK_REPOSITORY.hash(&mut hasher);
    SDK_BRANCH.hash(&mut hasher);
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
