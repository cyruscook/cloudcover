use std::{
    collections::hash_map::DefaultHasher,
    env,
    error::Error,
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    process::{Command, Output},
};

const MINIMUM_GO_MINOR: u32 = 24;
const ARCHIVE_NAME: &str = "libcloudcover_go_analyzer.a";
const CACHE_DIR_NAME: &str = "cloudcover-build-cache";

type BuildResult<T> = Result<T, Box<dyn Error>>;

fn main() -> BuildResult<()> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=analyzer/go.mod");
    println!("cargo:rerun-if-changed=analyzer/go.sum");
    println!("cargo:rerun-if-changed=analyzer/main.go");
    println!("cargo:rerun-if-env-changed=CC");

    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-lib=legacy_stdio_definitions");
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-lib=resolv");
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-lib=dylib=pthread");
        println!("cargo:rustc-link-lib=dylib=dl");
        println!("cargo:rustc-link-lib=dylib=m");
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let cache_dir = shared_cache_dir(&out_dir)?.join("cloudcover-go");
    fs::create_dir_all(&cache_dir)?;

    let cache_key = format!(
        "{}-{}",
        env::var("TARGET")?,
        analyzer_fingerprint(&manifest_dir)?
    );
    let cached_archive_path = cache_dir.join(format!("{cache_key}-{ARCHIVE_NAME}"));
    if !cached_archive_path.exists() {
        ensure_go_available()?;
        build_go_analyzer(&manifest_dir, &cached_archive_path)?;
    }

    copy_if_changed(&cached_archive_path, &out_dir.join(ARCHIVE_NAME))?;

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=cloudcover_go_analyzer");

    Ok(())
}

fn build_go_analyzer(manifest_dir: &Path, output_path: &Path) -> BuildResult<()> {
    let output = Command::new("go")
        .args(["build", "-mod=readonly", "-buildmode=c-archive", "-o"])
        .arg(output_path)
        .arg(".")
        .current_dir(manifest_dir.join("analyzer"))
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format_command_failure(
            "go build",
            &output,
            &["failed to build cloudcover-go analyzer"],
        )
        .into())
    }
}

fn analyzer_fingerprint(manifest_dir: &Path) -> BuildResult<String> {
    let mut hasher = DefaultHasher::new();
    env::var("TARGET")?.hash(&mut hasher);
    env::var_os("CC").hash(&mut hasher);
    for relative_path in [
        "build.rs",
        "analyzer/go.mod",
        "analyzer/go.sum",
        "analyzer/main.go",
    ] {
        relative_path.hash(&mut hasher);
        fs::read(manifest_dir.join(relative_path))?.hash(&mut hasher);
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

fn copy_if_changed(source: &Path, destination: &Path) -> BuildResult<()> {
    match (fs::read(source), fs::read(destination)) {
        (Ok(source_bytes), Ok(destination_bytes)) if source_bytes == destination_bytes => Ok(()),
        (Ok(_), Ok(_) | Err(_)) => {
            fs::copy(source, destination)?;
            Ok(())
        }
        (Err(error), _) => Err(error.into()),
    }
}

fn ensure_command_available(command: &str, missing_help: &[&str]) -> BuildResult<Output> {
    let version_flag = if command == "go" {
        "version"
    } else {
        "--version"
    };
    match Command::new(command).arg(version_flag).output() {
        Ok(output) if output.status.success() => Ok(output),
        Ok(output) => Err(format_command_failure(command, &output, missing_help).into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(format_missing_executable(command, missing_help).into())
        }
        Err(error) => Err(format!("failed to execute {command}: {error}").into()),
    }
}

fn ensure_go_available() -> BuildResult<()> {
    let help = [
        "go is required to build the cloudcover-go analyzer.",
        "Go 1.24+ is required because the analyzer module declares go 1.24.",
    ];
    let output = ensure_command_available("go", &help)?;

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
        "go 1.{MINIMUM_GO_MINOR}+ is required because the analyzer module declares go 1.24; found go{version}"
    )
    .into())
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
