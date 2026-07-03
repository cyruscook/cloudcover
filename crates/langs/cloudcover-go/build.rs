use std::{
    env,
    error::Error,
    path::PathBuf,
    process::{Command, Output},
};

const MINIMUM_GO_MINOR: u32 = 24;

type BuildResult<T> = Result<T, Box<dyn Error>>;

fn main() -> BuildResult<()> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=analyzer/go.mod");
    println!("cargo:rerun-if-changed=analyzer/go.sum");
    println!("cargo:rerun-if-changed=analyzer/main.go");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-lib=resolv");
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-lib=dylib=pthread");
        println!("cargo:rustc-link-lib=dylib=dl");
        println!("cargo:rustc-link-lib=dylib=m");
    }

    ensure_go_available()?;

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let output = Command::new("go")
        .args(["build", "-mod=readonly", "-buildmode=c-archive", "-o"])
        .arg(out_dir.join("libcloudcover_go_analyzer.a"))
        .arg(".")
        .current_dir(manifest_dir.join("analyzer"))
        .output()?;
    if !output.status.success() {
        return Err(format_command_failure(
            "go build",
            &output,
            &["failed to build cloudcover-go analyzer"],
        )
        .into());
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=cloudcover_go_analyzer");

    Ok(())
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
