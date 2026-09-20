use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    fs,
    hash::{Hash, Hasher},
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Output},
};

use clap::{ArgGroup, Parser};
use semver::Version;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
#[cfg(test)]
use tempfile::TempDir;

#[path = "../data.rs"]
mod data;

use data::{
    MappingState, PermissionDataFile, TerraformProviderAwsMethodMappingRow,
    TerraformProviderAwsUnsupportedReason,
};

const PROVIDER_REPOSITORY: &str = "https://github.com/hashicorp/terraform-provider-aws";
const SNAPSHOT_FORMAT_VERSION: &str = "3";
const ANALYZER_SOURCE: &[u8] = include_bytes!("../../generator/main.go");
const ANALYZER_MODULE: &[u8] = include_bytes!("../../generator/go.mod");
const ANALYZER_SUMS: &[u8] = include_bytes!("../../generator/go.sum");

const AWS_SDK_GO_V1_MODULE_PATH: &str = "github.com/aws/aws-sdk-go";
const AWS_SDK_SERVICE_MODULE_PREFIX: &str = "github.com/aws/aws-sdk-go-v2/service/";
const MAX_EMPTY_HANDLER_PERCENT: usize = 20;

const FORBIDDEN_SETTER_HELPERS: &[&str] =
    &["SetEncryptionContextEquals", "SetFilter", "SetSAMLOptions"];

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

    #[arg(
        long,
        value_name = "PATH",
        help = "Persistent snapshots, provider checkout, and bounded Go cache",
        default_value = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../target/terraform-provider-aws-data"
        )
    )]
    work_dir: PathBuf,

    #[arg(
        long,
        help = "Regenerate selected versions instead of skipping existing files"
    )]
    force: bool,

    #[arg(
        long,
        requires = "force",
        help = "Ignore valid cached provider analyses and recompute them"
    )]
    refresh_cache: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ApiMethodRef {
    service: &'static str,
    name: &'static str,
}

#[derive(Clone, Debug, Serialize)]
struct AwsSdkGoMapping {
    package: &'static str,
    receiver: &'static str,
    method: &'static str,
    api_methods: Vec<ApiMethodRef>,
}

struct Snapshot {
    state: MappingState,
    unsupported_reason: Option<TerraformProviderAwsUnsupportedReason>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GoModule {
    path: String,
    version: Option<String>,
    replace: Option<Box<GoModule>>,
}

type GeneratorResult<T> = Result<T, Box<dyn Error>>;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> GeneratorResult<()> {
    let mut args = Args::parse();
    args.output_dir = absolute_path(&args.output_dir)?;
    args.work_dir = absolute_path(&args.work_dir)?;
    let output_dir = args.output_dir.clone();
    fs::create_dir_all(&args.work_dir)?;
    recover_pending_publish(&args.work_dir, &output_dir)?;

    let selected_versions = select_versions(&args)?;
    let policy_deferred_versions = args
        .force
        .then(|| selected_versions.iter().cloned().collect::<BTreeSet<_>>())
        .unwrap_or_default();
    let existing_files = load_existing_files(&output_dir, &policy_deferred_versions)?;
    let pending_versions = select_pending_versions(&existing_files, &selected_versions, args.force);
    if pending_versions.is_empty() {
        return Ok(());
    }

    let mut replacements = BTreeMap::<Version, PathBuf>::new();
    let supported_versions = pending_versions;

    if !supported_versions.is_empty() {
        let fingerprint = analyzer_fingerprint()?;
        let snapshot_dir = args.work_dir.join("snapshots").join(&fingerprint);
        fs::create_dir_all(&snapshot_dir)?;
        let analyzer_dir = args.work_dir.join("analyzers");
        fs::create_dir_all(&analyzer_dir)?;
        let analyzer_path = analyzer_dir.join(&fingerprint);
        if !analyzer_path.exists() {
            build_analyzer(&analyzer_path)?;
        }

        let provider_dir = args
            .work_dir
            .join("provider/gopath/src/github.com/hashicorp/terraform-provider-aws");
        let module_cache = args.work_dir.join("go-mod-cache");
        fs::create_dir_all(&module_cache)?;
        fs::create_dir_all(provider_dir.parent().ok_or("provider path has no parent")?)?;
        initialize_provider_checkout(&provider_dir)?;
        for version in supported_versions {
            let snapshot_path = snapshot_dir.join(format!("{version}.json"));
            if !args.refresh_cache && load_snapshot(&snapshot_path, &version).is_ok() {
                eprintln!("Using cached Terraform AWS provider v{version} analysis");
                replacements.insert(version, snapshot_path);
                continue;
            }
            eprintln!("Analyzing Terraform AWS provider v{version}");
            with_clean_provider_checkout(&provider_dir, || {
                checkout_provider_version(&provider_dir, &version)?;
                if !provider_dir.join("go.mod").is_file() {
                    persist_unsupported_snapshot(
                        &snapshot_dir,
                        &version,
                        TerraformProviderAwsUnsupportedReason::AwsSdkGoV1,
                    )?;
                    return Ok(());
                }
                let sdk_map_path = args.work_dir.join("cloudcover-aws-sdk-go-v2-mappings.json");
                write_sdk_map(&sdk_map_path, &provider_dir, &module_cache)?;
                let mut mappings =
                    analyze_provider(&provider_dir, &sdk_map_path, &analyzer_path, &module_cache)?;
                let state = state_from_rows(&mut mappings)
                    .map_err(|error| format!("v{version}: {error}"))?;
                persist_snapshot(&snapshot_dir, &version, &state)?;
                Ok(())
            })?;
            replacements.insert(version, snapshot_path);
        }
    }

    let publish_dir = prepare_publish(&args.work_dir, existing_files, &replacements)?;
    publish_transaction(&publish_dir, &output_dir)?;
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
    policy_deferred_versions: &BTreeSet<Version>,
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
        if !policy_deferred_versions.contains(&version) {
            validate_snapshot_policy(&version, &state, file.unsupported_reason)
                .map_err(|error| format!("{}: {error}", path.display()))?;
        }
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

fn prepare_publish(
    work_dir: &Path,
    existing_files: BTreeMap<Version, PermissionDataFile>,
    replacements: &BTreeMap<Version, PathBuf>,
) -> GeneratorResult<PathBuf> {
    let publish_dir = create_unique_directory(work_dir, "publish")?;
    let staged_data_dir = publish_dir.join("data");
    fs::create_dir(&staged_data_dir)?;

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
        let replacement = replacements
            .get(&version)
            .map(|path| load_snapshot(path, &version))
            .transpose()?;
        let unsupported_reason = replacement
            .as_ref()
            .and_then(|snapshot| snapshot.unsupported_reason)
            .or_else(|| {
                existing_files
                    .get(&version)
                    .and_then(|file| file.unsupported_reason)
            });
        let mut delta = match (unsupported_reason, replacement) {
            (Some(reason), _) => PermissionDataFile {
                provider_version: version.to_string(),
                unsupported_reason: Some(reason),
                remove: Vec::new(),
                upsert: Vec::new(),
            },
            (None, Some(snapshot)) => {
                create_delta(version.to_string(), &rewritten_state, &snapshot.state)
            }
            (None, None) => create_delta(version.to_string(), &rewritten_state, &original_state),
        };
        data::validate_and_apply_file(&mut delta, Some(&version.to_string()), &mut rewritten_state)
            .map_err(|error| format!("v{version}: generated invalid delta: {error}"))?;
        validate_snapshot_policy(&version, &rewritten_state, delta.unsupported_reason)
            .map_err(|error| format!("v{version}: generated invalid snapshot: {error}"))?;
        persist_data_file(&staged_data_dir, &version, &delta)?;
    }
    Ok(publish_dir)
}

fn publish_transaction(publish_dir: &Path, output_dir: &Path) -> GeneratorResult<()> {
    let staged_data_dir = publish_dir.join("data");
    let output_parent = output_dir
        .parent()
        .ok_or_else(|| format!("output directory {} has no parent", output_dir.display()))?;
    let output_name = output_dir
        .file_name()
        .ok_or_else(|| format!("output directory {} has no name", output_dir.display()))?
        .to_string_lossy();
    fs::create_dir_all(output_parent)?;
    recover_output_transaction(output_dir)?;

    let candidate =
        create_unique_directory(output_parent, &format!(".{output_name}.publish-candidate"))?;
    sync_directory(output_parent)?;
    let backup = unique_sibling_path(output_parent, &format!(".{output_name}.publish-backup"))?;

    let result = (|| -> GeneratorResult<()> {
        copy_publish_data(&staged_data_dir, &candidate)?;
        sync_directory(&candidate)?;

        if output_dir.exists() {
            fs::rename(output_dir, &backup)?;
            if let Err(error) = sync_directory(output_parent) {
                if let Err(rollback) =
                    restore_output_from_backup(&backup, output_dir, output_parent)
                {
                    return Err(format!(
                        "failed to sync output move for {}: {error}; failed to restore {}: {rollback}",
                        output_dir.display(),
                        backup.display()
                    )
                    .into());
                }
                return Err(error);
            }
        }
        if let Err(error) = fs::rename(&candidate, output_dir) {
            if backup.exists() {
                if let Err(rollback) =
                    restore_output_from_backup(&backup, output_dir, output_parent)
                {
                    return Err(format!(
                        "failed to publish {}: {error}; failed to restore {}: {rollback}",
                        output_dir.display(),
                        backup.display()
                    )
                    .into());
                }
            }
            return Err(error.into());
        }
        sync_directory(output_parent)?;
        if backup.exists() {
            fs::remove_dir_all(&backup)?;
            sync_directory(output_parent)?;
        }
        Ok(())
    })();

    if result.is_ok() {
        fs::remove_dir_all(publish_dir)?;
    }
    result
}

fn copy_publish_data(staged_data_dir: &Path, candidate: &Path) -> GeneratorResult<()> {
    for entry in fs::read_dir(staged_data_dir)? {
        let entry = entry?;
        let source = entry.path();
        if !source.is_file() {
            return Err(format!("staged data path {} is not a file", source.display()).into());
        }
        let target = candidate.join(entry.file_name());
        let mut destination = fs::File::create(target)?;
        let mut input = fs::File::open(source)?;
        io::copy(&mut input, &mut destination)?;
        destination.sync_all()?;
    }
    Ok(())
}

fn restore_output_from_backup(
    backup: &Path,
    output_dir: &Path,
    output_parent: &Path,
) -> GeneratorResult<()> {
    fs::rename(backup, output_dir)?;
    sync_directory(output_parent)
}

fn sync_directory(directory: &Path) -> GeneratorResult<()> {
    fs::File::open(directory)?.sync_all()?;
    Ok(())
}

fn recover_pending_publish(work_dir: &Path, output_dir: &Path) -> GeneratorResult<()> {
    recover_output_transaction(output_dir)?;
    if !work_dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(work_dir)? {
        let path = entry?.path();
        if path.is_dir()
            && path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("publish-"))
        {
            fs::remove_dir_all(path)?;
        }
    }
    Ok(())
}

fn recover_output_transaction(output_dir: &Path) -> GeneratorResult<()> {
    let parent = output_dir
        .parent()
        .ok_or_else(|| format!("output directory {} has no parent", output_dir.display()))?;
    if !parent.exists() {
        return Ok(());
    }
    let output_name = output_dir
        .file_name()
        .ok_or_else(|| format!("output directory {} has no name", output_dir.display()))?
        .to_string_lossy();
    let candidate_prefix = format!(".{output_name}.publish-candidate-");
    let backup_prefix = format!(".{output_name}.publish-backup-");
    let mut candidates = Vec::new();
    let mut backups = Vec::new();
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&candidate_prefix) {
            candidates.push(entry.path());
        } else if name.starts_with(&backup_prefix) {
            backups.push(entry.path());
        }
    }
    candidates.sort();
    backups.sort();

    if !output_dir.exists() {
        if let Some(candidate) = candidates.pop() {
            fs::rename(candidate, output_dir)?;
        } else if let Some(backup) = backups.pop() {
            fs::rename(backup, output_dir)?;
        }
        sync_directory(parent)?;
    }
    for candidate in candidates {
        fs::remove_dir_all(candidate)?;
    }
    for backup in backups {
        fs::remove_dir_all(backup)?;
    }
    sync_directory(parent)?;
    Ok(())
}

fn create_unique_directory(parent: &Path, prefix: &str) -> GeneratorResult<PathBuf> {
    for suffix in 0..1000 {
        let name = if suffix == 0 {
            format!("{prefix}-{}", std::process::id())
        } else {
            format!("{prefix}-{}-{suffix}", std::process::id())
        };
        let path = parent.join(name);
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(format!(
        "could not create unique {prefix} directory in {}",
        parent.display()
    )
    .into())
}

fn unique_sibling_path(parent: &Path, prefix: &str) -> GeneratorResult<PathBuf> {
    for suffix in 0..1000 {
        let name = if suffix == 0 {
            format!("{prefix}-{}", std::process::id())
        } else {
            format!("{prefix}-{}-{suffix}", std::process::id())
        };
        let path = parent.join(name);
        if !path.exists() {
            return Ok(path);
        }
    }
    Err(format!(
        "could not reserve unique {prefix} path in {}",
        parent.display()
    )
    .into())
}

fn persist_snapshot(
    snapshot_dir: &Path,
    version: &Version,
    state: &MappingState,
) -> GeneratorResult<()> {
    persist_snapshot_with_reason(snapshot_dir, version, state, None)
}

fn persist_unsupported_snapshot(
    snapshot_dir: &Path,
    version: &Version,
    reason: TerraformProviderAwsUnsupportedReason,
) -> GeneratorResult<()> {
    persist_snapshot_with_reason(snapshot_dir, version, &MappingState::new(), Some(reason))
}

fn persist_snapshot_with_reason(
    snapshot_dir: &Path,
    version: &Version,
    state: &MappingState,
    unsupported_reason: Option<TerraformProviderAwsUnsupportedReason>,
) -> GeneratorResult<()> {
    let file = PermissionDataFile {
        provider_version: version.to_string(),
        unsupported_reason,
        remove: Vec::new(),
        upsert: state
            .iter()
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
            .collect(),
    };
    persist_data_file(snapshot_dir, version, &file)
}

fn load_snapshot(path: &Path, version: &Version) -> GeneratorResult<Snapshot> {
    let contents = fs::read_to_string(path)?;
    let mut file: PermissionDataFile = serde_json::from_str(&contents)?;
    let unsupported_reason = file.unsupported_reason;
    let mut state = MappingState::new();
    data::validate_and_apply_file(&mut file, Some(&version.to_string()), &mut state)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    validate_snapshot_policy(version, &state, unsupported_reason)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(Snapshot {
        state,
        unsupported_reason,
    })
}

fn validate_snapshot_policy(
    _version: &Version,
    state: &MappingState,
    unsupported_reason: Option<TerraformProviderAwsUnsupportedReason>,
) -> Result<(), String> {
    if unsupported_reason.is_some() {
        return Ok(());
    }
    validate_mapping_state(state)
}

fn analyzer_fingerprint() -> GeneratorResult<String> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    SNAPSHOT_FORMAT_VERSION.hash(&mut hasher);
    ANALYZER_SOURCE.hash(&mut hasher);
    ANALYZER_MODULE.hash(&mut hasher);
    ANALYZER_SUMS.hash(&mut hasher);
    AWS_SDK_GO_V1_MODULE_PATH.hash(&mut hasher);
    for version in cloudcover_aws_sdk_go_v1::sdk_versions() {
        version.hash(&mut hasher);
        let mappings = cloudcover_aws_sdk_go_v1::sdk_method_mappings(version)
            .ok_or_else(|| format!("AWS Go SDK v1 v{version} is not indexed"))?;
        for mapping in mappings {
            mapping.package.hash(&mut hasher);
            mapping.receiver.hash(&mut hasher);
            mapping.method.hash(&mut hasher);
            for api_method in mapping.api_methods {
                api_method.service.hash(&mut hasher);
                api_method.name.hash(&mut hasher);
            }
        }
    }
    for module_path in cloudcover_aws_sdk_go_v2::service_modules() {
        module_path.hash(&mut hasher);
        let versions = cloudcover_aws_sdk_go_v2::service_versions(module_path)
            .ok_or_else(|| format!("AWS Go SDK service module {module_path} has no versions"))?;
        for version in versions {
            version.hash(&mut hasher);
            let mappings = cloudcover_aws_sdk_go_v2::service_method_mappings(module_path, version)
                .ok_or_else(|| {
                    format!("AWS Go SDK service module {module_path} v{version} is not indexed")
                })?;
            for mapping in mappings {
                mapping.package.hash(&mut hasher);
                mapping.receiver.hash(&mut hasher);
                mapping.method.hash(&mut hasher);
                for api_method in mapping.api_methods {
                    api_method.service.hash(&mut hasher);
                    api_method.name.hash(&mut hasher);
                }
            }
        }
    }
    Ok(format!("{:016x}", hasher.finish()))
}

fn state_from_rows(
    rows: &mut Vec<TerraformProviderAwsMethodMappingRow>,
) -> Result<MappingState, String> {
    validate_mapping_rows(rows)?;
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

fn validate_mapping_rows(rows: &[TerraformProviderAwsMethodMappingRow]) -> Result<(), String> {
    let total_rows = rows.len();
    let empty_handler_rows = rows.iter().filter(|row| row.api_methods.is_empty()).count();
    let api_reference_count = rows.iter().map(|row| row.api_methods.len()).sum();
    validate_mapping_counts(total_rows, empty_handler_rows, api_reference_count)?;
    for row in rows {
        for api_method in &row.api_methods {
            validate_api_operation_name(&api_method.service, &api_method.name)?;
        }
    }
    Ok(())
}

fn validate_mapping_state(state: &MappingState) -> Result<(), String> {
    let total_rows = state.len();
    let empty_handler_rows = state
        .values()
        .filter(|api_methods| api_methods.is_empty())
        .count();
    let api_reference_count = state.values().map(Vec::len).sum();
    validate_mapping_counts(total_rows, empty_handler_rows, api_reference_count)?;
    for api_methods in state.values() {
        for api_method in api_methods {
            validate_api_operation_name(&api_method.service, &api_method.name)?;
        }
    }
    Ok(())
}

fn validate_mapping_counts(
    total_rows: usize,
    empty_handler_rows: usize,
    api_reference_count: usize,
) -> Result<(), String> {
    if total_rows == 0 {
        return Err("provider analysis has 0 total rows (requires more than 0)".to_owned());
    }
    if api_reference_count == 0 {
        return Err(format!(
            "provider analysis has {total_rows} total rows but 0 API references (requires more than 0)"
        ));
    }
    if empty_handler_rows * 100 > total_rows * MAX_EMPTY_HANDLER_PERCENT {
        return Err(format!(
            "provider analysis has {empty_handler_rows} empty-handler rows out of {total_rows} total rows ({empty_handler_rows}/{total_rows} = {:.2}%; maximum {MAX_EMPTY_HANDLER_PERCENT}%)",
            empty_handler_rows as f64 * 100.0 / total_rows as f64
        ));
    }
    Ok(())
}

fn validate_api_operation_name(service: &str, name: &str) -> Result<(), String> {
    let forbidden_exact = matches!(
        name,
        "String" | "Validate" | "New" | "newClient" | "NormalizeBucketLocation"
    );
    if forbidden_exact || name.ends_with("_Values") || FORBIDDEN_SETTER_HELPERS.contains(&name) {
        return Err(format!(
            "provider analysis emits forbidden helper API operation {service}.{name}"
        ));
    }
    Ok(())
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
        unsupported_reason: None,
        remove,
        upsert,
    }
}

fn write_sdk_map(path: &Path, provider_dir: &Path, module_cache: &Path) -> GeneratorResult<()> {
    let versions = provider_service_versions(provider_dir, module_cache)?;
    let mut rows = Vec::new();
    for (module_path, requested_version) in versions {
        let version = sdk_data_version(&module_path, &requested_version)?;
        if module_path == AWS_SDK_GO_V1_MODULE_PATH {
            let mappings = cloudcover_aws_sdk_go_v1::sdk_method_mappings(version)
                .ok_or_else(|| format!("AWS Go SDK v1 v{version} is not indexed"))?;
            rows.extend(mappings.map(|row| {
                AwsSdkGoMapping {
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
                }
            }));
        } else {
            let mappings = cloudcover_aws_sdk_go_v2::service_method_mappings(&module_path, version)
                .ok_or_else(|| {
                    format!("AWS Go SDK service module {module_path} v{version} is not indexed")
                })?;
            rows.extend(mappings.map(|row| {
                AwsSdkGoMapping {
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
                }
            }));
        }
    }
    validate_sdk_paginator_parity(&rows)?;
    let mut file = fs::File::create(path)?;
    serde_json::to_writer(&mut file, &rows)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn validate_sdk_paginator_parity(rows: &[AwsSdkGoMapping]) -> GeneratorResult<()> {
    for paginator in rows
        .iter()
        .filter(|row| row.receiver.ends_with("Paginator") && row.method == "NextPage")
    {
        let operation = paginator
            .receiver
            .strip_suffix("Paginator")
            .expect("Paginator suffix was checked");
        let client = rows.iter().find(|row| {
            row.package == paginator.package && row.receiver == "Client" && row.method == operation
        });
        let Some(client) = client else {
            return Err(format!(
                "AWS SDK mapping {}.{}.{} has no matching Client.{} mapping",
                paginator.package, paginator.receiver, paginator.method, operation
            )
            .into());
        };
        if paginator.api_methods.is_empty() || client.api_methods.is_empty() {
            return Err(format!(
                "AWS SDK mapping {}.{}.{} and Client.{} must have nonempty API references",
                paginator.package, paginator.receiver, paginator.method, operation
            )
            .into());
        }
        if paginator.api_methods != client.api_methods {
            return Err(format!(
                "AWS SDK mapping {}.{}.{} does not match Client.{} API references",
                paginator.package, paginator.receiver, paginator.method, operation
            )
            .into());
        }
    }
    Ok(())
}

/// Select the newest generated SDK snapshot not newer than the provider's
/// resolved module. Return an error if no indexed snapshot is compatible.
fn sdk_data_version(module_path: &str, requested_version: &str) -> GeneratorResult<&'static str> {
    let requested = Version::parse(requested_version)?;
    if module_path == AWS_SDK_GO_V1_MODULE_PATH {
        return sdk_versions_not_newer_than(
            cloudcover_aws_sdk_go_v1::sdk_versions(),
            requested,
            "AWS Go SDK v1",
            None,
        );
    }
    let versions = cloudcover_aws_sdk_go_v2::service_versions(module_path)
        .ok_or_else(|| format!("AWS Go SDK service module {module_path} is not indexed"))?;
    sdk_versions_not_newer_than(
        versions,
        requested,
        "AWS Go SDK service module",
        Some(module_path),
    )
}

fn sdk_versions_not_newer_than(
    versions: &'static [&'static str],
    requested: Version,
    label: &str,
    module_path: Option<&str>,
) -> GeneratorResult<&'static str> {
    versions
        .iter()
        .rev()
        .find(|candidate| Version::parse(candidate).is_ok_and(|version| version <= requested))
        .copied()
        .ok_or_else(|| {
            let module_path = module_path
                .map(|module_path| format!(" {module_path}"))
                .unwrap_or_default();
            format!(
                "{label}{module_path} has no compatible snapshot for requested version {requested}"
            )
            .into()
        })
}

fn provider_service_versions(
    provider_dir: &Path,
    module_cache: &Path,
) -> GeneratorResult<BTreeMap<String, String>> {
    let go_mod_path = provider_dir.join("go.mod");
    if !go_mod_path.is_file() {
        return Ok(BTreeMap::new());
    }
    let indexed_modules = cloudcover_aws_sdk_go_v2::service_modules();
    let output = command_output_with_env(
        "go",
        &["list", "-m", "-json", "all"],
        Some(provider_dir),
        Some(("GOMODCACHE", path_arg(module_cache)?)),
    );
    match output {
        Ok(output) => service_versions_from_go_list(&output.stdout, indexed_modules),
        Err(error) => {
            eprintln!(
                "warning: go module resolution failed for {}; using direct go.mod requirements",
                provider_dir.display()
            );
            service_versions_from_go_mod(&fs::read_to_string(go_mod_path)?, indexed_modules)
                .map_err(|fallback| {
                    format!(
                        "go module resolution failed: {error}; go.mod fallback failed: {fallback}"
                    )
                    .into()
                })
        }
    }
}

fn service_versions_from_go_list(
    contents: &[u8],
    indexed_modules: &[&str],
) -> GeneratorResult<BTreeMap<String, String>> {
    let mut versions = BTreeMap::new();
    for module in serde_json::Deserializer::from_slice(contents).into_iter::<GoModule>() {
        let module = module?;
        let is_v1 = module.path == AWS_SDK_GO_V1_MODULE_PATH;
        let is_v2 = module.path.starts_with(AWS_SDK_SERVICE_MODULE_PREFIX)
            && indexed_modules.binary_search(&module.path.as_str()).is_ok();
        if !is_v1 && !is_v2 {
            continue;
        }
        let effective = module.replace.as_deref().unwrap_or(&module);
        if effective.path != module.path {
            return Err(format!(
                "AWS Go SDK service module {} is replaced by unsupported module {}",
                module.path, effective.path
            )
            .into());
        }
        let version = effective.version.as_deref().ok_or_else(|| {
            format!(
                "AWS Go SDK service module {} has no resolved version",
                module.path
            )
        })?;
        insert_service_version(&mut versions, &module.path, version, indexed_modules)?;
    }
    Ok(versions)
}

fn service_versions_from_go_mod(
    contents: &str,
    indexed_modules: &[&str],
) -> GeneratorResult<BTreeMap<String, String>> {
    let mut versions = BTreeMap::new();
    let mut in_require_block = false;
    for raw_line in contents.lines() {
        let line = raw_line
            .split_once("//")
            .map_or(raw_line, |(line, _)| line)
            .trim();
        if line == "require (" {
            in_require_block = true;
            continue;
        }
        if in_require_block && line == ")" {
            in_require_block = false;
            continue;
        }
        let requirement = if in_require_block {
            Some(line)
        } else {
            line.strip_prefix("require ")
        };
        let Some(requirement) = requirement else {
            continue;
        };
        let mut fields = requirement.split_whitespace();
        let Some(module_path) = fields.next() else {
            continue;
        };
        let Some(version) = fields.next() else {
            continue;
        };
        insert_service_version(&mut versions, module_path, version, indexed_modules)?;
    }
    Ok(versions)
}

fn insert_service_version(
    versions: &mut BTreeMap<String, String>,
    module_path: &str,
    raw_version: &str,
    indexed_modules: &[&str],
) -> GeneratorResult<()> {
    if module_path != AWS_SDK_GO_V1_MODULE_PATH
        && (!module_path.starts_with(AWS_SDK_SERVICE_MODULE_PREFIX)
            || indexed_modules.binary_search(&module_path).is_err())
    {
        return Ok(());
    }
    let version = Version::parse(raw_version.strip_prefix('v').unwrap_or(raw_version))
        .map_err(|error| {
            format!(
                "AWS Go SDK service module {module_path} has invalid version {raw_version}: {error}"
            )
        })?
        .to_string();
    versions.insert(module_path.to_owned(), version);
    Ok(())
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

fn absolute_path(path: &Path) -> GeneratorResult<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_owned());
    }
    Ok(env::current_dir()?.join(path))
}

fn path_arg(path: &Path) -> GeneratorResult<&str> {
    path.to_str()
        .ok_or_else(|| format!("path is not valid UTF-8: {}", path.display()).into())
}

fn initialize_provider_checkout(provider_dir: &Path) -> GeneratorResult<()> {
    command_output("git", &["init", "--quiet", path_arg(provider_dir)?], None)?;
    match command_output(
        "git",
        &["-C", path_arg(provider_dir)?, "remote", "get-url", "origin"],
        None,
    ) {
        Ok(output) => {
            let remote = String::from_utf8(output.stdout)?;
            if remote.trim() != PROVIDER_REPOSITORY {
                return Err(format!(
                    "{} has unexpected origin remote {}",
                    provider_dir.display(),
                    remote.trim()
                )
                .into());
            }
        }
        Err(_) => {
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
        }
    }
    Ok(())
}

fn checkout_provider_version(provider_dir: &Path, version: &Version) -> GeneratorResult<()> {
    clean_provider_checkout(provider_dir)?;
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
    Ok(())
}

fn build_analyzer(analyzer_path: &Path) -> GeneratorResult<()> {
    let generator_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("generator");
    command_output(
        "go",
        &["build", "-o", path_arg(analyzer_path)?, "./main.go"],
        Some(&generator_dir),
    )?;
    Ok(())
}

fn with_clean_provider_checkout<T>(
    provider_dir: &Path,
    operation: impl FnOnce() -> GeneratorResult<T>,
) -> GeneratorResult<T> {
    let result = operation();
    let cleanup = clean_provider_checkout(provider_dir);
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(operation), Ok(())) => Err(operation),
        (Err(operation), Err(cleanup)) => Err(format!(
            "{operation}; additionally failed to clean provider checkout: {cleanup}"
        )
        .into()),
    }
}

fn clean_provider_checkout(provider_dir: &Path) -> GeneratorResult<()> {
    if command_output(
        "git",
        &[
            "-C",
            path_arg(provider_dir)?,
            "rev-parse",
            "--verify",
            "--quiet",
            "HEAD",
        ],
        None,
    )
    .is_ok()
    {
        command_output(
            "git",
            &["-C", path_arg(provider_dir)?, "reset", "--hard", "--quiet"],
            None,
        )?;
    }
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
    analyzer_path: &Path,
    module_cache: &Path,
) -> GeneratorResult<Vec<TerraformProviderAwsMethodMappingRow>> {
    let output = command_output_with_env(
        path_arg(analyzer_path)?,
        &[
            "--provider-dir",
            path_arg(provider_dir)?,
            "--sdk-map-json",
            path_arg(sdk_map_path)?,
        ],
        None,
        Some(("GOMODCACHE", path_arg(module_cache)?)),
    )?;
    let stdout = String::from_utf8(output.stdout)?;
    serde_json::from_str(&stdout)
        .map_err(|error| format!("failed to parse analyzer output as JSON: {error}").into())
}
fn command_output(
    command: &str,
    args: &[&str],
    current_dir: Option<&Path>,
) -> GeneratorResult<Output> {
    command_output_with_env(command, args, current_dir, None)
}

fn command_output_with_env(
    command: &str,
    args: &[&str],
    current_dir: Option<&Path>,
    environment: Option<(&str, &str)>,
) -> GeneratorResult<Output> {
    let mut process = Command::new(command);
    process.args(args);
    if let Some(current_dir) = current_dir {
        process.current_dir(current_dir);
    }
    if let Some((key, value)) = environment {
        process.env(key, value);
    }
    let output = process
        .output()
        .map_err(|error| format!("{command}: {error}"))?;
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
    fn publish_transaction_replaces_output_directory_as_a_unit() -> Result<(), Box<dyn Error>> {
        let root = TempDir::new()?;
        let output = root.path().join("data");
        fs::create_dir(&output)?;
        fs::write(output.join("old.json"), "old")?;
        let publish = root.path().join("publish");
        let staged = publish.join("data");
        fs::create_dir_all(&staged)?;
        fs::write(staged.join("new.json"), "new")?;

        publish_transaction(&publish, &output)?;

        assert_eq!(fs::read_to_string(output.join("new.json"))?, "new");
        assert!(!output.join("old.json").exists());
        assert!(!publish.exists());
        Ok(())
    }

    #[test]
    fn failed_swap_restore_moves_backup_back_to_output() -> Result<(), Box<dyn Error>> {
        let root = TempDir::new()?;
        let output = root.path().join("data");
        let backup = root.path().join(".data.publish-backup-test");
        fs::create_dir(&backup)?;
        fs::write(backup.join("state"), "old")?;

        restore_output_from_backup(&backup, &output, root.path())?;

        assert_eq!(fs::read_to_string(output.join("state"))?, "old");
        assert!(!backup.exists());
        Ok(())
    }

    #[test]
    fn recovery_resolves_interrupted_output_transactions() -> Result<(), Box<dyn Error>> {
        let root = TempDir::new()?;
        let work = root.path().join("work");
        fs::create_dir(&work)?;

        let promoted = root.path().join("promoted");
        let candidate = root.path().join(".promoted.publish-candidate-test");
        let backup = root.path().join(".promoted.publish-backup-test");
        fs::create_dir(&candidate)?;
        fs::write(candidate.join("state"), "new")?;
        fs::create_dir(&backup)?;
        fs::write(backup.join("state"), "old")?;
        recover_pending_publish(&work, &promoted)?;
        assert_eq!(fs::read_to_string(promoted.join("state"))?, "new");
        assert!(!candidate.exists());
        assert!(!backup.exists());

        let restored = root.path().join("restored");
        let backup = root.path().join(".restored.publish-backup-test");
        fs::create_dir(&backup)?;
        fs::write(backup.join("state"), "old")?;
        recover_pending_publish(&work, &restored)?;
        assert_eq!(fs::read_to_string(restored.join("state"))?, "old");
        assert!(!backup.exists());

        let retained = root.path().join("retained");
        fs::create_dir(&retained)?;
        fs::write(retained.join("state"), "current")?;
        let candidate = root.path().join(".retained.publish-candidate-test");
        let backup = root.path().join(".retained.publish-backup-test");
        fs::create_dir(&candidate)?;
        fs::create_dir(&backup)?;
        recover_pending_publish(&work, &retained)?;
        assert_eq!(fs::read_to_string(retained.join("state"))?, "current");
        assert!(!candidate.exists());
        assert!(!backup.exists());
        Ok(())
    }

    #[test]
    fn historical_replacement_repairs_successor_delta() -> Result<(), Box<dyn Error>> {
        let first_version = Version::parse("1.57.0")?;
        let replaced_version = Version::parse("1.58.0")?;
        let successor_version = Version::parse("1.59.0")?;
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

        let work = TempDir::new()?;
        let output = TempDir::new()?;
        let snapshot_dir = work.path().join("snapshots");
        fs::create_dir_all(&snapshot_dir)?;
        let snapshot_path = snapshot_dir.join(format!("{replaced_version}.json"));
        persist_snapshot(&snapshot_dir, &replaced_version, &replacement)?;
        let publish_dir = prepare_publish(
            work.path(),
            existing_files,
            &BTreeMap::from([(replaced_version.clone(), snapshot_path)]),
        )?;
        publish_transaction(&publish_dir, output.path())?;

        let files = load_existing_files(output.path(), &BTreeSet::new())?;
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

    #[test]
    fn full_forced_replacement_defers_legacy_policy_validation() -> Result<(), Box<dyn Error>> {
        let output = TempDir::new()?;
        let work = TempDir::new()?;
        let version = Version::parse("1.57.0")?;
        let legacy_file = PermissionDataFile {
            provider_version: version.to_string(),
            unsupported_reason: None,
            remove: Vec::new(),
            upsert: vec![(
                "resource".to_owned(),
                "aws_test".to_owned(),
                "read".to_owned(),
                vec![("test".to_owned(), "String".to_owned())],
            )],
        };
        fs::write(
            output.path().join(format!("{version}.json")),
            serde_json::to_vec(&legacy_file)?,
        )?;

        let existing_files =
            load_existing_files(output.path(), &BTreeSet::from([version.clone()]))?;
        let snapshot_dir = work.path().join("snapshots");
        fs::create_dir(&snapshot_dir)?;
        let replacement = mapping_state("DescribeThings");
        persist_snapshot(&snapshot_dir, &version, &replacement)?;
        let publish_dir = prepare_publish(
            work.path(),
            existing_files,
            &BTreeMap::from([(
                version.clone(),
                snapshot_dir.join(format!("{version}.json")),
            )]),
        )?;

        load_existing_files(&publish_dir.join("data"), &BTreeSet::new())?;
        Ok(())
    }

    #[test]
    fn unsupported_snapshot_round_trips() -> Result<(), Box<dyn Error>> {
        let work = TempDir::new()?;
        let version = Version::parse("0.1.0")?;
        let snapshot_dir = work.path().join("snapshots");
        fs::create_dir(&snapshot_dir)?;

        persist_unsupported_snapshot(
            &snapshot_dir,
            &version,
            TerraformProviderAwsUnsupportedReason::AwsSdkGoV1,
        )?;
        let snapshot = load_snapshot(&snapshot_dir.join(format!("{version}.json")), &version)?;

        assert!(snapshot.state.is_empty());
        assert_eq!(
            snapshot.unsupported_reason,
            Some(TerraformProviderAwsUnsupportedReason::AwsSdkGoV1)
        );
        Ok(())
    }

    #[test]
    fn partial_forced_replacement_rejects_unselected_legacy_policy_data()
    -> Result<(), Box<dyn Error>> {
        let output = TempDir::new()?;
        let legacy_version = Version::parse("1.57.0")?;
        let selected_version = Version::parse("1.58.0")?;
        let legacy_file = PermissionDataFile {
            provider_version: legacy_version.to_string(),
            unsupported_reason: None,
            remove: Vec::new(),
            upsert: vec![(
                "resource".to_owned(),
                "aws_test".to_owned(),
                "read".to_owned(),
                vec![("test".to_owned(), "String".to_owned())],
            )],
        };
        fs::write(
            output.path().join(format!("{legacy_version}.json")),
            serde_json::to_vec(&legacy_file)?,
        )?;

        let error =
            load_existing_files(output.path(), &BTreeSet::from([selected_version])).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("forbidden helper API operation test.String"),
            "{error}"
        );
        Ok(())
    }

    #[test]
    fn publish_rejects_policy_invalid_replacement() -> Result<(), Box<dyn Error>> {
        let work = TempDir::new()?;
        let version = Version::parse("1.57.0")?;
        let replacement_path = work.path().join(format!("{version}.json"));
        let replacement = PermissionDataFile {
            provider_version: version.to_string(),
            unsupported_reason: None,
            remove: Vec::new(),
            upsert: vec![(
                "resource".to_owned(),
                "aws_test".to_owned(),
                "read".to_owned(),
                vec![("test".to_owned(), "String".to_owned())],
            )],
        };
        fs::write(&replacement_path, serde_json::to_vec(&replacement)?)?;

        let error = prepare_publish(
            work.path(),
            BTreeMap::new(),
            &BTreeMap::from([(version, replacement_path)]),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("forbidden helper API operation test.String"),
            "{error}"
        );
        Ok(())
    }

    #[test]
    fn sdk_data_version_selects_supported_release() -> Result<(), Box<dyn Error>> {
        const MODULE_PATH: &str = "github.com/aws/aws-sdk-go-v2/service/s3";

        let versions = cloudcover_aws_sdk_go_v2::service_versions(MODULE_PATH)
            .expect("S3 service versions must be indexed");
        assert!(
            versions.len() >= 2,
            "S3 must have enough indexed releases to test compatible selection"
        );
        let latest = *versions.last().expect("S3 versions must not be empty");
        let (compatible, requested) = versions
            .windows(2)
            .rev()
            .find_map(|pair| {
                let mut requested = Version::parse(pair[0]).ok()?;
                requested.patch = requested.patch.checked_add(1)?;
                let next = Version::parse(pair[1]).ok()?;
                (requested < next).then_some((pair[0], requested.to_string()))
            })
            .expect("S3 indexed versions must contain a release gap");

        assert_eq!(
            sdk_data_version(MODULE_PATH, &requested)?,
            compatible,
            "must select the newest indexed snapshot not newer than the provider requirement"
        );
        assert_eq!(
            sdk_data_version(MODULE_PATH, latest)?,
            latest,
            "must select the newest compatible indexed snapshot"
        );
        let error = sdk_data_version(MODULE_PATH, "0.0.0").unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "AWS Go SDK service module {MODULE_PATH} has no compatible snapshot for requested version 0.0.0"
            )
        );
        Ok(())
    }

    #[test]
    fn mapping_gates_reject_zero_rows_and_zero_operations() {
        assert_eq!(
            validate_mapping_rows(&[]),
            Err("provider analysis has 0 total rows (requires more than 0)".to_owned())
        );
        let error = validate_mapping_rows(&[mapping_row(0, None)]).unwrap_err();
        assert_eq!(
            error,
            "provider analysis has 1 total rows but 0 API references (requires more than 0)"
        );
    }

    #[test]
    fn mapping_gate_accepts_exact_empty_handler_ratio_boundary() {
        let mut rows = (0..16)
            .map(|index| mapping_row(index, Some("DescribeThings")))
            .collect::<Vec<_>>();
        rows.extend((16..20).map(|index| mapping_row(index, None)));
        assert!(validate_mapping_rows(&rows).is_ok());
    }

    #[test]
    fn mapping_gate_rejects_empty_handler_ratio_above_boundary() {
        let mut rows = (0..15)
            .map(|index| mapping_row(index, Some("DescribeThings")))
            .collect::<Vec<_>>();
        rows.extend((15..20).map(|index| mapping_row(index, None)));
        let error = validate_mapping_rows(&rows).unwrap_err();
        assert_eq!(
            error,
            "provider analysis has 5 empty-handler rows out of 20 total rows (5/20 = 25.00%; maximum 20%)"
        );
    }

    #[test]
    fn mapping_gate_accepts_request_suffixed_api_operation() {
        let row = TerraformProviderAwsMethodMappingRow {
            kind: "resource".to_owned(),
            type_name: "aws_spot_fleet_request".to_owned(),
            action: "update".to_owned(),
            api_methods: vec![ApiMethodRefRow {
                service: "ec2".to_owned(),
                name: "ModifySpotFleetRequest".to_owned(),
            }],
        };

        assert!(validate_mapping_rows(&[row]).is_ok());
    }

    #[test]
    fn mapping_gate_accepts_provenance_valid_suffix_api_operations() {
        let row = TerraformProviderAwsMethodMappingRow {
            kind: "resource".to_owned(),
            type_name: "aws_test".to_owned(),
            action: "read".to_owned(),
            api_methods: vec![
                ApiMethodRefRow {
                    service: "pricing".to_owned(),
                    name: "ListProductPages".to_owned(),
                },
                ApiMethodRefRow {
                    service: "pricing".to_owned(),
                    name: "ListProductRestEndpointPages".to_owned(),
                },
                ApiMethodRefRow {
                    service: "workspaces".to_owned(),
                    name: "ListWorkspacePages".to_owned(),
                },
                ApiMethodRefRow {
                    service: "example".to_owned(),
                    name: "HypotheticalWithContext".to_owned(),
                },
            ],
        };

        assert!(validate_mapping_rows(&[row]).is_ok());
    }

    #[test]
    fn mapping_gate_accepts_known_set_api_operation() {
        let row = TerraformProviderAwsMethodMappingRow {
            kind: "resource".to_owned(),
            type_name: "aws_lb".to_owned(),
            action: "update".to_owned(),
            api_methods: vec![ApiMethodRefRow {
                service: "elasticloadbalancingv2".to_owned(),
                name: "SetIpAddressType".to_owned(),
            }],
        };

        assert!(validate_mapping_rows(&[row]).is_ok());
    }

    #[test]
    fn mapping_gate_rejects_helper_api_operations() {
        for helper in [
            "String",
            "Validate",
            "New",
            "newClient",
            "NormalizeBucketLocation",
            "LogType_Values",
            "SetEncryptionContextEquals",
            "SetFilter",
            "SetSAMLOptions",
        ] {
            let error = validate_mapping_rows(&[mapping_row(0, Some(helper))]).unwrap_err();
            assert_eq!(
                error,
                format!("provider analysis emits forbidden helper API operation test.{helper}")
            );
        }
    }

    #[test]
    fn cached_snapshots_cannot_bypass_mapping_gates() -> Result<(), Box<dyn Error>> {
        let directory = TempDir::new()?;
        let version = Version::parse("1.57.0")?;
        let path = directory.path().join("1.57.0.json");
        let file = PermissionDataFile {
            provider_version: version.to_string(),
            unsupported_reason: None,
            remove: Vec::new(),
            upsert: vec![(
                "resource".to_owned(),
                "aws_test".to_owned(),
                "read".to_owned(),
                vec![("test".to_owned(), "String".to_owned())],
            )],
        };
        fs::write(&path, serde_json::to_vec(&file)?)?;
        let error = match load_snapshot(&path, &version) {
            Ok(_) => panic!("invalid cached snapshot passed mapping validation"),
            Err(error) => error.to_string(),
        };
        assert!(
            error.contains("forbidden helper API operation test.String"),
            "{error}"
        );
        Ok(())
    }

    #[test]
    fn sdk_paginator_mappings_require_matching_client_operations() {
        let api_methods = vec![ApiMethodRef {
            service: "s3",
            name: "ListBuckets",
        }];
        let paginator = sdk_mapping("ListBucketsPaginator", "NextPage", api_methods.clone());
        let missing = validate_sdk_paginator_parity(&[paginator.clone()])
            .unwrap_err()
            .to_string();
        assert!(missing.contains("has no matching Client.ListBuckets mapping"));

        let mismatch = validate_sdk_paginator_parity(&[
            paginator.clone(),
            sdk_mapping(
                "Client",
                "ListBuckets",
                vec![ApiMethodRef {
                    service: "s3",
                    name: "ListObjectsV2",
                }],
            ),
        ])
        .unwrap_err()
        .to_string();
        assert!(mismatch.contains("does not match Client.ListBuckets API references"));

        assert!(
            validate_sdk_paginator_parity(&[
                paginator,
                sdk_mapping("Client", "ListBuckets", api_methods),
            ])
            .is_ok()
        );
    }

    #[test]
    fn provider_checkout_is_reused_and_source_cleanup_preserves_module_cache()
    -> Result<(), Box<dyn Error>> {
        let root = TempDir::new()?;
        let provider_dir = root.path().join("provider");
        let module_cache = root.path().join("go-mod-cache");
        initialize_provider_checkout(&provider_dir)?;
        fs::write(provider_dir.join(".git/persistent"), "keep")?;
        initialize_provider_checkout(&provider_dir)?;
        assert_eq!(
            fs::read_to_string(provider_dir.join(".git/persistent"))?,
            "keep"
        );
        fs::write(provider_dir.join("tracked"), "clean")?;
        command_output(
            "git",
            &["-C", path_arg(&provider_dir)?, "add", "tracked"],
            None,
        )?;
        command_output(
            "git",
            &[
                "-C",
                path_arg(&provider_dir)?,
                "-c",
                "user.name=CloudCover Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "-m",
                "initial",
            ],
            None,
        )?;
        fs::write(provider_dir.join("tracked"), "dirty")?;
        fs::write(provider_dir.join("generated"), "dirty")?;
        fs::create_dir_all(&module_cache)?;
        fs::write(module_cache.join("cached-module"), "cached")?;

        with_clean_provider_checkout(&provider_dir, || Ok(()))?;

        assert!(provider_dir.join(".git").is_dir());
        assert!(!provider_dir.join("generated").exists());
        assert_eq!(fs::read_to_string(provider_dir.join("tracked"))?, "clean");
        assert_eq!(
            fs::read_to_string(module_cache.join("cached-module"))?,
            "cached"
        );
        Ok(())
    }

    #[test]
    fn provider_checkout_cleanup_runs_after_analysis_failure() -> Result<(), Box<dyn Error>> {
        let root = TempDir::new()?;
        let provider_dir = root.path().join("provider");
        initialize_provider_checkout(&provider_dir)?;
        fs::write(provider_dir.join("generated"), "dirty")?;

        let error =
            with_clean_provider_checkout(&provider_dir, || Err::<(), _>("analysis failed".into()))
                .unwrap_err();

        assert!(error.to_string().contains("analysis failed"));
        assert!(provider_dir.join(".git").is_dir());
        assert!(!provider_dir.join("generated").exists());
        Ok(())
    }

    fn mapping_row(
        index: usize,
        api_method_name: Option<&str>,
    ) -> TerraformProviderAwsMethodMappingRow {
        TerraformProviderAwsMethodMappingRow {
            kind: "resource".to_owned(),
            type_name: format!("aws_test_{index}"),
            action: "read".to_owned(),
            api_methods: api_method_name
                .into_iter()
                .map(|name| ApiMethodRefRow {
                    service: "test".to_owned(),
                    name: name.to_owned(),
                })
                .collect(),
        }
    }

    fn sdk_mapping(
        receiver: &'static str,
        method: &'static str,
        api_methods: Vec<ApiMethodRef>,
    ) -> AwsSdkGoMapping {
        AwsSdkGoMapping {
            package: "github.com/aws/aws-sdk-go-v2/service/s3",
            receiver,
            method,
            api_methods,
        }
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
