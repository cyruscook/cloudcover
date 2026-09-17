use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use clap::{ArgGroup, Parser};
use semver::Version;
use serde::Serialize;
use tempfile::{NamedTempFile, TempDir};

#[path = "../data.rs"]
mod data;

use data::{MappingState, PermissionDataFile, TerraformProviderAwsMethodMappingRow};

const PROVIDER_REPOSITORY: &str = "https://github.com/hashicorp/terraform-provider-aws";

#[derive(Debug, Parser)]
#[command(
    name = "generate-terraform-provider-aws-data",
    about = "Generate checked-in Terraform AWS provider permission data",
    disable_version_flag = true,
    group = ArgGroup::new("selection").required(true).args(["versions", "all"])
)]
struct Args {
    #[arg(long = "version", value_name = "SEMVER", action = clap::ArgAction::Append)]
    versions: Vec<String>,

    #[arg(long)]
    all: bool,

    #[arg(long, requires = "all", value_name = "SEMVER")]
    from: Option<String>,

    #[arg(long, requires = "all", value_name = "SEMVER")]
    to: Option<String>,

    #[arg(
        long,
        value_name = "PATH",
        default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/data")
    )]
    output_dir: PathBuf,

    #[arg(long)]
    force: bool,
}

#[derive(Clone, Debug, Serialize)]
struct ApiMethodRef {
    service: &'static str,
    name: &'static str,
}

#[derive(Clone, Debug, Serialize)]
struct AwsSdkGoV2Mapping {
    package: &'static str,
    receiver: &'static str,
    method: &'static str,
    api_methods: Vec<ApiMethodRef>,
}

type GeneratorResult<T> = Result<T, Box<dyn Error>>;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> GeneratorResult<()> {
    let args = Args::parse();
    let selected_versions = select_versions(&args)?;
    let output_dir = args.output_dir;
    let existing_files = load_existing_files(&output_dir)?;
    let pending_versions = select_pending_versions(&existing_files, &selected_versions, args.force);
    if pending_versions.is_empty() {
        return Ok(());
    }

    fs::create_dir_all(&output_dir)?;
    let temporary = TempDir::new()?;
    let provider_dir = temporary
        .path()
        .join("gopath/src/github.com/hashicorp/terraform-provider-aws");
    fs::create_dir_all(provider_dir.parent().ok_or("provider path has no parent")?)?;
    let sdk_map_path = temporary
        .path()
        .join("cloudcover-aws-sdk-go-v2-mappings.json");
    write_sdk_map(&sdk_map_path)?;
    initialize_provider_checkout(&provider_dir)?;

    let mut replacements = BTreeMap::<Version, MappingState>::new();
    for version in pending_versions {
        checkout_provider_version(&provider_dir, &version)?;
        let mut mappings = analyze_provider(&provider_dir, &sdk_map_path)?;
        let state =
            state_from_rows(&mut mappings).map_err(|error| format!("v{version}: {error}"))?;
        replacements.insert(version, state);
    }
    persist_updated_files(&output_dir, existing_files, replacements)?;
    Ok(())
}

fn select_versions(args: &Args) -> GeneratorResult<Vec<Version>> {
    if !args.all {
        if args.versions.is_empty() {
            return Err("at least one --version is required".into());
        }
        let mut versions = args
            .versions
            .iter()
            .map(|raw| data::parse_canonical_version(raw).map_err(|error| error.into()))
            .collect::<GeneratorResult<Vec<_>>>()?;
        versions.sort();
        versions.dedup();
        return Ok(versions);
    }

    let from = args
        .from
        .as_deref()
        .map(data::parse_canonical_version)
        .transpose()
        .map_err(|error| -> Box<dyn Error> { error.into() })?;
    let to = args
        .to
        .as_deref()
        .map(data::parse_canonical_version)
        .transpose()
        .map_err(|error| -> Box<dyn Error> { error.into() })?;
    if let (Some(from), Some(to)) = (&from, &to) {
        if from > to {
            return Err(format!("--from {from} is greater than --to {to}").into());
        }
    }

    let mut versions = discover_stable_versions()?;
    versions.retain(|version| {
        from.as_ref().is_none_or(|from| version >= from)
            && to.as_ref().is_none_or(|to| version <= to)
    });
    if versions.is_empty() {
        return Err("no stable provider versions match the requested range".into());
    }
    Ok(versions)
}

fn discover_stable_versions() -> GeneratorResult<Vec<Version>> {
    let output = command_output(
        "git",
        &["ls-remote", "--tags", "--refs", PROVIDER_REPOSITORY, "v*"],
        None,
    )?;
    let stdout = String::from_utf8(output.stdout)?;
    let mut versions = BTreeMap::<Version, ()>::new();
    for line in stdout.lines() {
        let Some((_, reference)) = line.split_once('\t') else {
            continue;
        };
        let Some(tag) = reference.strip_prefix("refs/tags/v") else {
            continue;
        };
        if let Ok(version) = data::parse_canonical_version(tag) {
            versions.insert(version, ());
        }
    }
    Ok(versions.into_keys().collect())
}

fn load_existing_files(
    output_dir: &Path,
) -> GeneratorResult<BTreeMap<Version, PermissionDataFile>> {
    if !output_dir.exists() {
        return Ok(BTreeMap::new());
    }
    let mut paths = BTreeMap::<Version, PathBuf>::new();
    for entry in fs::read_dir(output_dir)? {
        let path = entry?.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| format!("invalid permission data filename: {}", path.display()))?;
        let version = data::parse_canonical_version(stem)?;
        if paths.insert(version.clone(), path).is_some() {
            return Err(format!("duplicate permission data version {version}").into());
        }
    }

    let mut state = MappingState::new();
    let mut files = BTreeMap::new();
    for (version, path) in paths {
        let contents = fs::read_to_string(&path)?;
        let mut file: PermissionDataFile = serde_json::from_str(&contents)
            .map_err(|error| format!("{}: invalid JSON: {error}", path.display()))?;
        data::validate_and_apply_file(&mut file, Some(&version.to_string()), &mut state)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        files.insert(version, file);
    }
    Ok(files)
}

fn select_pending_versions(
    existing_files: &BTreeMap<Version, PermissionDataFile>,
    versions: &[Version],
    force: bool,
) -> Vec<Version> {
    versions
        .iter()
        .filter(|version| force || !existing_files.contains_key(*version))
        .cloned()
        .collect()
}

fn persist_updated_files(
    output_dir: &Path,
    existing_files: BTreeMap<Version, PermissionDataFile>,
    replacements: BTreeMap<Version, MappingState>,
) -> GeneratorResult<()> {
    let versions = existing_files
        .keys()
        .chain(replacements.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut original_state = MappingState::new();
    let mut rewritten_state = MappingState::new();

    for version in versions {
        if let Some(existing) = existing_files.get(&version) {
            let mut existing = existing.clone();
            data::validate_and_apply_file(
                &mut existing,
                Some(&version.to_string()),
                &mut original_state,
            )
            .map_err(|error| format!("v{version}: {error}"))?;
        }
        let target = replacements.get(&version).unwrap_or(&original_state);
        let mut delta = create_delta(version.to_string(), &rewritten_state, target);
        data::validate_and_apply_file(&mut delta, Some(&version.to_string()), &mut rewritten_state)
            .map_err(|error| format!("v{version}: generated invalid delta: {error}"))?;
        persist_data_file(output_dir, &version, &delta)?;
    }
    Ok(())
}
fn state_from_rows(
    rows: &mut Vec<TerraformProviderAwsMethodMappingRow>,
) -> Result<MappingState, String> {
    data::validate_and_normalize_rows(rows)?;
    Ok(rows
        .iter()
        .map(|row| {
            (
                (row.kind.clone(), row.type_name.clone(), row.action.clone()),
                row.api_methods.clone(),
            )
        })
        .collect())
}

fn create_delta(
    provider_version: String,
    previous: &MappingState,
    current: &MappingState,
) -> PermissionDataFile {
    let remove = previous
        .keys()
        .filter(|key| !current.contains_key(*key))
        .cloned()
        .collect();
    let upsert = current
        .iter()
        .filter(|(key, api_methods)| previous.get(*key) != Some(*api_methods))
        .map(|((kind, type_name, action), api_methods)| {
            (
                kind.clone(),
                type_name.clone(),
                action.clone(),
                api_methods
                    .iter()
                    .map(|api_method| (api_method.service.clone(), api_method.name.clone()))
                    .collect(),
            )
        })
        .collect();
    PermissionDataFile {
        provider_version,
        remove,
        upsert,
    }
}

fn write_sdk_map(path: &Path) -> GeneratorResult<()> {
    let rows = cloudcover_aws_sdk_go_v2::SDK_METHOD_MAPPINGS
        .iter()
        .map(|row| AwsSdkGoV2Mapping {
            package: row.package,
            receiver: row.receiver,
            method: row.method,
            api_methods: row
                .api_methods
                .iter()
                .map(|api_method| ApiMethodRef {
                    service: api_method.service,
                    name: api_method.name,
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    let mut file = fs::File::create(path)?;
    serde_json::to_writer(&mut file, &rows)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn initialize_provider_checkout(provider_dir: &Path) -> GeneratorResult<()> {
    command_output("git", &["init", "--quiet", path_arg(provider_dir)?], None)?;
    command_output(
        "git",
        &[
            "-C",
            path_arg(provider_dir)?,
            "remote",
            "add",
            "origin",
            PROVIDER_REPOSITORY,
        ],
        None,
    )?;
    Ok(())
}

fn checkout_provider_version(provider_dir: &Path, version: &Version) -> GeneratorResult<()> {
    let tag = format!("v{version}");
    command_output(
        "git",
        &[
            "-C",
            path_arg(provider_dir)?,
            "fetch",
            "--depth=1",
            "origin",
            &format!("refs/tags/{tag}:refs/tags/{tag}"),
        ],
        None,
    )?;
    command_output(
        "git",
        &[
            "-C",
            path_arg(provider_dir)?,
            "checkout",
            "--force",
            "--detach",
            &format!("refs/tags/{tag}"),
        ],
        None,
    )?;
    command_output(
        "git",
        &["-C", path_arg(provider_dir)?, "clean", "-fdx"],
        None,
    )?;
    Ok(())
}

fn analyze_provider(
    provider_dir: &Path,
    sdk_map_path: &Path,
) -> GeneratorResult<Vec<TerraformProviderAwsMethodMappingRow>> {
    let generator_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("generator");
    let output = command_output(
        "go",
        &[
            "run",
            "-mod=readonly",
            "./main.go",
            "--provider-dir",
            path_arg(provider_dir)?,
            "--sdk-map-json",
            path_arg(sdk_map_path)?,
        ],
        Some(&generator_dir),
    )?;
    let stdout = String::from_utf8(output.stdout)?;
    serde_json::from_str(&stdout)
        .map_err(|error| format!("failed to parse analyzer output as JSON: {error}").into())
}

fn persist_data_file(
    output_dir: &Path,
    version: &Version,
    file: &PermissionDataFile,
) -> GeneratorResult<()> {
    let path = output_dir.join(format!("{version}.json"));
    let mut temporary = NamedTempFile::new_in(output_dir)?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), file)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(&path)
        .map_err(|error| format!("failed to atomically persist {}: {error}", path.display()))?;
    Ok(())
}

fn path_arg(path: &Path) -> GeneratorResult<&str> {
    path.to_str()
        .ok_or_else(|| format!("path is not valid UTF-8: {}", path.display()).into())
}

fn command_output(
    command: &str,
    args: &[&str],
    current_dir: Option<&Path>,
) -> GeneratorResult<Output> {
    let mut process = Command::new(command);
    process.args(args);
    if let Some(current_dir) = current_dir {
        process.current_dir(current_dir);
    }
    let output = process.output()?;
    if output.status.success() {
        return Ok(output);
    }

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
    Err(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use data::ApiMethodRefRow;

    #[test]
    fn historical_replacement_repairs_successor_delta() -> Result<(), Box<dyn Error>> {
        let first_version = Version::parse("1.0.0")?;
        let replaced_version = Version::parse("1.1.0")?;
        let successor_version = Version::parse("1.2.0")?;
        let first = mapping_state("First");
        let original = mapping_state("Original");
        let replacement = mapping_state("Replacement");
        let successor = mapping_state("Successor");

        let snapshots = [
            (first_version.clone(), first),
            (replaced_version.clone(), original),
            (successor_version.clone(), successor.clone()),
        ];
        let mut previous = MappingState::new();
        let mut existing_files = BTreeMap::new();
        for (version, snapshot) in snapshots {
            existing_files.insert(
                version.clone(),
                create_delta(version.to_string(), &previous, &snapshot),
            );
            previous = snapshot;
        }

        let output = TempDir::new()?;
        persist_updated_files(
            output.path(),
            existing_files,
            BTreeMap::from([(replaced_version.clone(), replacement.clone())]),
        )?;

        let files = load_existing_files(output.path())?;
        assert!(
            files
                .get(&successor_version)
                .is_some_and(|file| !file.upsert.is_empty())
        );
        let mut reconstructed = MappingState::new();
        for (version, mut file) in files {
            data::validate_and_apply_file(
                &mut file,
                Some(&version.to_string()),
                &mut reconstructed,
            )?;
            if version == replaced_version {
                assert_eq!(reconstructed, replacement);
            } else if version == successor_version {
                assert_eq!(reconstructed, successor);
            }
        }
        Ok(())
    }

    fn mapping_state(api_name: &str) -> MappingState {
        BTreeMap::from([(
            (
                "resource".to_owned(),
                "aws_test".to_owned(),
                "read".to_owned(),
            ),
            vec![ApiMethodRefRow {
                service: "test".to_owned(),
                name: api_name.to_owned(),
            }],
        )])
    }
}
