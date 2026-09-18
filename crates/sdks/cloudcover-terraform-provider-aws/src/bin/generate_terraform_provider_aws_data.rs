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
use serde::Serialize;
use tempfile::NamedTempFile;
#[cfg(test)]
use tempfile::TempDir;

#[path = "../data.rs"]
mod data;

use data::{MappingState, PermissionDataFile, TerraformProviderAwsMethodMappingRow};

const PROVIDER_REPOSITORY: &str = "https://github.com/hashicorp/terraform-provider-aws";
const SNAPSHOT_FORMAT_VERSION: &str = "1";
const ANALYZER_SOURCE: &[u8] = include_bytes!("../../generator/main.go");
const ANALYZER_MODULE: &[u8] = include_bytes!("../../generator/go.mod");
const ANALYZER_SUMS: &[u8] = include_bytes!("../../generator/go.sum");

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
    let mut args = Args::parse();
    args.output_dir = absolute_path(&args.output_dir)?;
    args.work_dir = absolute_path(&args.work_dir)?;
    let output_dir = args.output_dir.clone();
    fs::create_dir_all(&args.work_dir)?;
    recover_pending_publish(&args.work_dir, &output_dir)?;

    let selected_versions = select_versions(&args)?;
    let existing_files = load_existing_files(&output_dir)?;
    let pending_versions = select_pending_versions(&existing_files, &selected_versions, args.force);
    if pending_versions.is_empty() {
        return Ok(());
    }

    fs::create_dir_all(&output_dir)?;
    let sdk_map_path = args.work_dir.join("cloudcover-aws-sdk-go-v2-mappings.json");
    write_sdk_map(&sdk_map_path)?;
    let sdk_map = fs::read(&sdk_map_path)?;
    let fingerprint = analyzer_fingerprint(&sdk_map);
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
    let mut analyzed_count = 0;
    let mut replacements = BTreeMap::<Version, PathBuf>::new();
    for version in pending_versions {
        let snapshot_path = snapshot_dir.join(format!("{version}.json"));
        if !args.refresh_cache && load_snapshot(&snapshot_path, &version).is_ok() {
            eprintln!("Using cached Terraform AWS provider v{version} analysis");
            replacements.insert(version, snapshot_path);
            continue;
        }
        fs::create_dir_all(provider_dir.parent().ok_or("provider path has no parent")?)?;
        drop_provider_checkout(&provider_dir)?;
        initialize_provider_checkout(&provider_dir)?;
        eprintln!("Analyzing Terraform AWS provider v{version}");
        checkout_provider_version(&provider_dir, &version)?;
        let mut mappings =
            analyze_provider(&provider_dir, &sdk_map_path, &analyzer_path, &module_cache)?;
        let state =
            state_from_rows(&mut mappings).map_err(|error| format!("v{version}: {error}"))?;
        persist_snapshot(&snapshot_dir, &version, &state)?;
        replacements.insert(version, snapshot_path);
        drop_provider_checkout(&provider_dir)?;
        analyzed_count += 1;
        if analyzed_count % 16 == 0 {
            reset_module_cache(&module_cache)?;
        }
    }

    reset_module_cache(&module_cache)?;
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
        let replacement_state = replacements
            .get(&version)
            .map(|path| load_snapshot(path, &version))
            .transpose()?;
        let target = replacement_state.as_ref().unwrap_or(&original_state);
        let mut delta = create_delta(version.to_string(), &rewritten_state, target);
        data::validate_and_apply_file(&mut delta, Some(&version.to_string()), &mut rewritten_state)
            .map_err(|error| format!("v{version}: generated invalid delta: {error}"))?;
        persist_data_file(&staged_data_dir, &version, &delta)?;
    }
    Ok(publish_dir)
}

fn publish_transaction(publish_dir: &Path, output_dir: &Path) -> GeneratorResult<()> {
    let staged_data_dir = publish_dir.join("data");
    fs::create_dir_all(output_dir)?;
    let mut paths = fs::read_dir(&staged_data_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.sort();
    for staged_path in paths {
        let file_name = staged_path
            .file_name()
            .ok_or("staged data path has no filename")?;
        let target = output_dir.join(file_name);
        let mut temporary = NamedTempFile::new_in(output_dir)?;
        let mut source = fs::File::open(&staged_path)?;
        io::copy(&mut source, temporary.as_file_mut())?;
        temporary.as_file().sync_all()?;
        temporary.persist(&target).map_err(|error| {
            format!("failed to atomically persist {}: {error}", target.display())
        })?;
    }
    fs::remove_dir_all(publish_dir)?;
    Ok(())
}

fn recover_pending_publish(work_dir: &Path, _output_dir: &Path) -> GeneratorResult<()> {
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

fn persist_snapshot(
    snapshot_dir: &Path,
    version: &Version,
    state: &MappingState,
) -> GeneratorResult<()> {
    let file = PermissionDataFile {
        provider_version: version.to_string(),
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

fn load_snapshot(path: &Path, version: &Version) -> GeneratorResult<MappingState> {
    let contents = fs::read_to_string(path)?;
    let mut file: PermissionDataFile = serde_json::from_str(&contents)?;
    let mut state = MappingState::new();
    data::validate_and_apply_file(&mut file, Some(&version.to_string()), &mut state)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(state)
}

fn analyzer_fingerprint(sdk_map: &[u8]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    SNAPSHOT_FORMAT_VERSION.hash(&mut hasher);
    ANALYZER_SOURCE.hash(&mut hasher);
    ANALYZER_MODULE.hash(&mut hasher);
    ANALYZER_SUMS.hash(&mut hasher);
    sdk_map.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
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

fn build_analyzer(analyzer_path: &Path) -> GeneratorResult<()> {
    let generator_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("generator");
    command_output(
        "go",
        &["build", "-o", path_arg(analyzer_path)?, "./main.go"],
        Some(&generator_dir),
    )?;
    Ok(())
}

fn drop_provider_checkout(provider_dir: &Path) -> GeneratorResult<()> {
    if provider_dir.exists() {
        fs::remove_dir_all(provider_dir)?;
    }
    Ok(())
}

fn reset_module_cache(module_cache: &Path) -> GeneratorResult<()> {
    command_output_with_env(
        "go",
        &["clean", "-modcache"],
        None,
        Some(("GOMODCACHE", path_arg(module_cache)?)),
    )?;
    fs::create_dir_all(module_cache)?;
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
