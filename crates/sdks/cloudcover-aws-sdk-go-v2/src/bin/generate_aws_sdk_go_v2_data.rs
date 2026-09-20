use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::mpsc,
    thread,
};

use clap::{ArgGroup, Parser};
use semver::Version;
use serde::Deserialize;
use tempfile::NamedTempFile;

#[path = "../data.rs"]
mod data;

use data::{ApiMethodRefRow, MappingState, SdkDataFile, SdkDataRelease};

const SDK_REPOSITORY: &str = "https://github.com/aws/aws-sdk-go-v2";
const SDK_MODULE_PREFIX: &str = "github.com/aws/aws-sdk-go-v2/service/";

#[derive(Debug, Parser)]
#[command(
    name = "generate-aws-sdk-go-v2-data",
    about = "Generate checked-in per-service aws-sdk-go-v2 mapping data",
    disable_version_flag = true,
    group = ArgGroup::new("selection").required(true).args(["tags", "all", "latest"])
)]
struct Args {
    #[arg(
        long = "tag",
        value_name = "SERVICE_TAG",
        action = clap::ArgAction::Append,
        help = "AWS tag such as service/s3/v1.104.0"
    )]
    tags: Vec<String>,

    #[arg(long, conflicts_with = "all")]
    latest: bool,
    #[arg(long)]
    all: bool,

    #[arg(
        long,
        value_name = "PATH",
        default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/data")
    )]
    output_dir: PathBuf,

    #[arg(
        long,
        value_name = "PATH",
        help = "Persistent analyzer and bounded Go module cache",
        default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../target/aws-sdk-go-v2-data")
    )]
    work_dir: PathBuf,

    #[arg(
        long,
        value_name = "PATH",
        help = "Local aws-sdk-go-v2 git repository used instead of the module proxy"
    )]
    repository_dir: Option<PathBuf>,
    #[arg(long)]
    force: bool,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ServiceRelease {
    service: String,
    module_path: String,
    version: Version,
    tag: String,
}

#[derive(Clone, Debug)]
enum ExistingRelease {
    Delta(SdkDataRelease),
    Snapshot(MappingState),
}

#[derive(Clone, Default)]
struct ExistingService {
    module_path: String,
    releases: BTreeMap<Version, ExistingRelease>,
}

#[derive(Clone, Debug, Deserialize)]
struct MappingRow {
    package: String,
    receiver: String,
    method: String,
    api_methods: Vec<ApiMethodRefRow>,
}

#[derive(Deserialize)]
struct LegacySdkDataFile {
    module_path: String,
    module_version: String,
    remove: Vec<(String, String, String)>,
    upsert: Vec<(String, String, String, Vec<(String, String)>)>,
}

#[derive(Deserialize)]
struct GoModuleDownload {
    #[serde(rename = "Dir")]
    dir: Option<PathBuf>,
    #[serde(rename = "Error")]
    error: Option<String>,
}

type GeneratorResult<T> = Result<T, Box<dyn Error + Send + Sync>>;
type ExistingServices = BTreeMap<String, ExistingService>;

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
    args.repository_dir = args
        .repository_dir
        .map(|path| absolute_path(&path))
        .transpose()?;
    fs::create_dir_all(&args.output_dir)?;
    fs::create_dir_all(&args.work_dir)?;

    ensure_command_available("git")?;
    ensure_command_available("go")?;
    let selected = select_releases(&args)?;
    let existing = load_existing(&args.output_dir)?;
    let mut releases_by_service = BTreeMap::<String, BTreeSet<ServiceRelease>>::new();
    for release in selected {
        releases_by_service
            .entry(release.module_path.clone())
            .or_default()
            .insert(release);
    }
    for service in existing.values() {
        for version in service.releases.keys() {
            releases_by_service
                .entry(service.module_path.clone())
                .or_default()
                .insert(ServiceRelease {
                    service: service_name(&service.module_path)?,
                    module_path: service.module_path.clone(),
                    version: version.clone(),
                    tag: format!("service/{}/v{version}", service_name(&service.module_path)?),
                });
        }
    }

    let analyzer_path = args.work_dir.join("aws-sdk-go-v2-analyzer");
    let generator_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("generator");
    command(
        "go",
        &["build", "-o", path_arg(&analyzer_path)?, "./main.go"],
        Some(&generator_dir),
    )?;
    let module_cache = args.work_dir.join("go-mod-cache");
    fs::create_dir_all(&module_cache)?;

    for (module_path, releases) in releases_by_service {
        let service = service_name(&module_path)?;
        let existing_service = existing.get(&module_path);
        let mut analyzed = analyze_missing_releases(
            &releases,
            existing_service,
            args.force,
            &analyzer_path,
            &module_cache,
            args.repository_dir.as_deref(),
        )?;
        let mut state = MappingState::new();
        let mut output_releases = Vec::with_capacity(releases.len());
        for release in releases {
            let previous = state.clone();
            let current = match (!args.force).then(|| {
                existing_service.and_then(|service| service.releases.get(&release.version))
            }) {
                Some(Some(ExistingRelease::Delta(existing_release))) => {
                    let mut candidate = state.clone();
                    let mut existing_release = existing_release.clone();
                    data::validate_and_apply_release(&mut existing_release, &mut candidate)
                        .map_err(|error| format!("{}: {error}", release.tag))?;
                    candidate
                }
                Some(Some(ExistingRelease::Snapshot(snapshot))) => snapshot.clone(),
                _ => analyzed
                    .remove(&release.version)
                    .ok_or_else(|| format!("missing analyzed mapping for {}", release.tag))?,
            };
            let delta = delta_release(&release.version.to_string(), &previous, &current);
            let mut validated = delta.clone();
            data::validate_and_apply_release(&mut validated, &mut state)
                .map_err(|error| format!("{}: generated invalid data: {error}", release.tag))?;
            output_releases.push(delta);
        }
        let file = SdkDataFile {
            module_path,
            releases: output_releases,
        };
        persist_data_file(&args.output_dir, &service, &file)?;
        let legacy_dir = args.output_dir.join(&service);
        if legacy_dir.is_dir() {
            fs::remove_dir_all(legacy_dir)?;
        }
    }
    Ok(())
}

fn select_releases(args: &Args) -> GeneratorResult<Vec<ServiceRelease>> {
    let mut releases = BTreeSet::new();
    if args.all || args.latest {
        let output = command(
            "git",
            &[
                "ls-remote",
                "--tags",
                "--refs",
                SDK_REPOSITORY,
                "refs/tags/service/*/v*",
            ],
            None,
        )?;
        let stdout = String::from_utf8(output.stdout)?;
        let discovered = stdout
            .lines()
            .filter_map(|line| line.split_once('\t').map(|(_, reference)| reference))
            .filter_map(|reference| reference.strip_prefix("refs/tags/"))
            .filter_map(|tag| parse_service_tag(tag).ok())
            .collect::<Vec<_>>();
        if args.latest {
            let mut latest = BTreeMap::<String, ServiceRelease>::new();
            for release in discovered {
                match latest.get(&release.module_path) {
                    Some(existing) if existing.version >= release.version => {}
                    Some(_) | None => {
                        latest.insert(release.module_path.clone(), release);
                    }
                }
            }
            releases.extend(latest.into_values());
        } else {
            releases.extend(discovered);
        }
    } else {
        for tag in &args.tags {
            releases.insert(parse_service_tag(tag)?);
        }
    }
    if releases.is_empty() {
        return Err("no stable AWS service releases matched the selection".into());
    }
    Ok(releases.into_iter().collect())
}

fn parse_service_tag(tag: &str) -> GeneratorResult<ServiceRelease> {
    let raw = tag
        .strip_prefix("service/")
        .ok_or_else(|| format!("unsupported AWS service tag {tag:?}"))?;
    let (service, raw_version) = raw
        .rsplit_once("/v")
        .ok_or_else(|| format!("unsupported AWS service tag {tag:?}"))?;
    if service.is_empty() || service.contains('/') {
        return Err(format!("tag {tag:?} does not identify a public service module").into());
    }
    let version = data::parse_canonical_version(raw_version)?;
    Ok(ServiceRelease {
        service: service.to_owned(),
        module_path: format!("{SDK_MODULE_PREFIX}{service}"),
        version,
        tag: tag.to_owned(),
    })
}

fn load_existing(output_dir: &Path) -> GeneratorResult<ExistingServices> {
    let mut services = ExistingServices::new();
    if !output_dir.exists() {
        return Ok(services);
    }
    for entry in fs::read_dir(output_dir)? {
        let path = entry?.path();
        if path.is_dir() {
            let service = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| format!("invalid service data directory: {}", path.display()))?;
            let module_path = format!("{SDK_MODULE_PREFIX}{service}");
            let target = services
                .entry(module_path.clone())
                .or_insert_with(|| ExistingService {
                    module_path: module_path.clone(),
                    ..ExistingService::default()
                });
            for version_entry in fs::read_dir(path)? {
                let version_path = version_entry?.path();
                if version_path
                    .extension()
                    .is_none_or(|extension| extension != "json")
                {
                    continue;
                }
                let file: LegacySdkDataFile =
                    serde_json::from_str(&fs::read_to_string(&version_path)?).map_err(|error| {
                        format!("{}: invalid JSON: {error}", version_path.display())
                    })?;
                if file.module_path != module_path {
                    return Err(format!("{}: wrong module path", version_path.display()).into());
                }
                let version = data::parse_canonical_version(&file.module_version)?;
                target.releases.insert(
                    version,
                    ExistingRelease::Delta(SdkDataRelease {
                        module_version: file.module_version,
                        remove: file.remove,
                        upsert: file.upsert,
                    }),
                );
            }
            continue;
        }
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let file: SdkDataFile = serde_json::from_str(&fs::read_to_string(&path)?)
            .map_err(|error| format!("{}: invalid JSON: {error}", path.display()))?;
        let module_path = file.module_path.clone();
        let mut validated_file = file.clone();
        let mut validation_state = MappingState::new();
        let versions = data::validate_and_apply_file(
            &mut validated_file,
            Some(&module_path),
            &mut validation_state,
        )
        .map_err(|error| format!("{}: {error}", path.display()))?;
        let target = services
            .entry(module_path.clone())
            .or_insert_with(|| ExistingService {
                module_path,
                ..ExistingService::default()
            });
        let mut releases = file.releases.into_iter();
        let mut snapshot = releases
            .next()
            .ok_or("validated SDK file has no releases")?;
        let mut state = MappingState::new();
        data::validate_and_apply_release(&mut snapshot, &mut state)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        target
            .releases
            .insert(versions[0].clone(), ExistingRelease::Snapshot(state));
        for (release, version) in releases.zip(versions.into_iter().skip(1)) {
            target
                .releases
                .insert(version, ExistingRelease::Delta(release));
        }
    }
    Ok(services)
}

fn rows_to_state(rows: Vec<MappingRow>) -> GeneratorResult<MappingState> {
    let mut state = MappingState::new();
    for row in rows {
        let key = (row.package, row.receiver, row.method);
        let mut api_methods = row.api_methods;
        api_methods.sort();
        api_methods.dedup();
        if api_methods.is_empty() {
            return Err(format!("mapping {} has no API methods", key.2).into());
        }
        if let Some(existing) = state.get(&key) {
            if existing != &api_methods {
                return Err(
                    format!("mapping rows disagree for {} {}.{}", key.0, key.1, key.2).into(),
                );
            }
        } else {
            state.insert(key, api_methods);
        }
    }
    Ok(state)
}

fn delta_release(
    module_version: &str,
    previous: &MappingState,
    current: &MappingState,
) -> SdkDataRelease {
    SdkDataRelease {
        module_version: module_version.to_owned(),
        remove: previous
            .keys()
            .filter(|key| !current.contains_key(*key))
            .cloned()
            .collect(),
        upsert: current
            .iter()
            .filter(|(key, api_methods)| {
                previous
                    .get(*key)
                    .is_none_or(|existing| existing.as_slice() != api_methods.as_slice())
            })
            .map(|((package, receiver, method), api_methods)| {
                (
                    package.clone(),
                    receiver.clone(),
                    method.clone(),
                    api_methods
                        .iter()
                        .map(|api_method| (api_method.service.clone(), api_method.name.clone()))
                        .collect(),
                )
            })
            .collect(),
    }
}
fn analyze_missing_releases(
    releases: &BTreeSet<ServiceRelease>,
    existing_service: Option<&ExistingService>,
    force: bool,
    analyzer_path: &Path,
    module_cache: &Path,
    repository_dir: Option<&Path>,
) -> GeneratorResult<BTreeMap<Version, MappingState>> {
    let pending = releases
        .iter()
        .filter(|release| {
            force
                || existing_service
                    .is_none_or(|service| !service.releases.contains_key(&release.version))
        })
        .cloned()
        .collect::<Vec<_>>();
    if pending.is_empty() {
        return Ok(BTreeMap::new());
    }

    let worker_count = thread::available_parallelism()
        .map_or(1, |parallelism| parallelism.get())
        .min(pending.len());
    let chunk_size = pending.len().div_ceil(worker_count);
    let (sender, receiver) = mpsc::channel();
    let mut analyzed = BTreeMap::new();
    let mut first_error = None;

    thread::scope(|scope| {
        for chunk in pending.chunks(chunk_size) {
            let sender = sender.clone();
            scope.spawn(move || {
                for release in chunk {
                    let result =
                        analyze_release(release, analyzer_path, module_cache, repository_dir);
                    if sender.send((release.version.clone(), result)).is_err() {
                        return;
                    }
                }
            });
        }
        drop(sender);
        for (version, result) in receiver {
            match result {
                Ok(state) => {
                    analyzed.insert(version, state);
                }
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(_) => {}
            }
        }
    });

    if let Some(error) = first_error {
        return Err(error);
    }
    if analyzed.len() != pending.len() {
        return Err(format!(
            "analyzed {} of {} pending releases",
            analyzed.len(),
            pending.len()
        )
        .into());
    }
    Ok(analyzed)
}

fn analyze_release(
    release: &ServiceRelease,
    analyzer_path: &Path,
    module_cache: &Path,
    repository_dir: Option<&Path>,
) -> GeneratorResult<MappingState> {
    let service_dir = download_service_module(release, module_cache, repository_dir)?;
    let analyzed = analyze(analyzer_path, &service_dir, &release.module_path);
    if repository_dir.is_some() {
        remove_extracted_service(&service_dir)?;
    }
    rows_to_state(analyzed?)
}

fn download_service_module(
    release: &ServiceRelease,
    module_cache: &Path,
    repository_dir: Option<&Path>,
) -> GeneratorResult<PathBuf> {
    if let Some(repository_dir) = repository_dir {
        return extract_service_from_repository(release, module_cache, repository_dir);
    }
    let query = format!("{}@v{}", release.module_path, release.version);
    let output = Command::new("go")
        .args(["mod", "download", "-json", &query])
        .env("GOMODCACHE", module_cache)
        .output()?;
    if !output.status.success() {
        return command_failure("go mod download", &output);
    }
    let download: GoModuleDownload = serde_json::from_slice(&output.stdout)?;
    if let Some(error) = download.error {
        return Err(format!("failed to download {query}: {error}").into());
    }
    download
        .dir
        .ok_or_else(|| format!("go mod download returned no directory for {query}").into())
}

fn extract_service_from_repository(
    release: &ServiceRelease,
    module_cache: &Path,
    repository_dir: &Path,
) -> GeneratorResult<PathBuf> {
    let source_dir = module_cache
        .join("sources")
        .join(&release.service)
        .join(release.version.to_string());
    let service_dir = source_dir.join("service").join(&release.service);
    if service_dir.is_dir() {
        return Ok(service_dir);
    }
    fs::create_dir_all(&source_dir)?;
    let archive = source_dir.join("source.tar");
    let service_path = format!("service/{}/*.go", release.service);
    command(
        "git",
        &[
            "-C",
            path_arg(repository_dir)?,
            "archive",
            "--format=tar",
            "--output",
            path_arg(&archive)?,
            &release.tag,
            &service_path,
        ],
        None,
    )?;
    command(
        "tar",
        &["-xf", path_arg(&archive)?, "-C", path_arg(&source_dir)?],
        None,
    )?;
    fs::remove_file(archive)?;
    Ok(service_dir)
}

fn analyze(
    analyzer_path: &Path,
    service_dir: &Path,
    module_path: &str,
) -> GeneratorResult<Vec<MappingRow>> {
    let output = command(
        path_arg(analyzer_path)?,
        &[
            "--fast",
            "--service-dir",
            path_arg(service_dir)?,
            "--module-path",
            module_path,
        ],
        None,
    )?;
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn remove_extracted_service(service_dir: &Path) -> GeneratorResult<()> {
    let source_dir = service_dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| format!("invalid extracted service path: {}", service_dir.display()))?;
    if source_dir.exists() {
        fs::remove_dir_all(source_dir)?;
    }
    Ok(())
}

fn persist_data_file(output_dir: &Path, service: &str, file: &SdkDataFile) -> GeneratorResult<()> {
    let path = output_dir.join(format!("{service}.json"));
    let mut temporary = NamedTempFile::new_in(output_dir)?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), file)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(&path)
        .map_err(|error| format!("failed to persist {}: {error}", path.display()))?;
    Ok(())
}

fn service_name(module_path: &str) -> GeneratorResult<String> {
    module_path
        .strip_prefix(SDK_MODULE_PREFIX)
        .filter(|service| !service.is_empty() && !service.contains('/'))
        .map(str::to_owned)
        .ok_or_else(|| format!("unsupported AWS service module {module_path:?}").into())
}

fn ensure_command_available(command_name: &str) -> GeneratorResult<()> {
    let args = if command_name == "go" {
        &["version"][..]
    } else {
        &["--version"][..]
    };
    command(command_name, args, None).map(|_| ())
}

fn command(command_name: &str, args: &[&str], cwd: Option<&Path>) -> GeneratorResult<Output> {
    let mut command = Command::new(command_name);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command.output()?;
    if output.status.success() {
        return Ok(output);
    }
    command_failure(command_name, &output)
}

fn command_failure<T>(command_name: &str, output: &Output) -> GeneratorResult<T> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "{command_name} failed with status {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status
    )
    .into())
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
