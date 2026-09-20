use std::{
    env,
    error::Error,
    fs,
    path::PathBuf,
    process::{Command, Output},
};

use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "generate-aws-sdk-go-v1-data",
    about = "Generate checked-in aws-sdk-go-v1 mapping data"
)]
struct Args {
    #[arg(long, value_name = "PATH", default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/data/aws-sdk-go.json"))]
    output: PathBuf,
    #[arg(long, value_name = "PATH", default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../target/aws-sdk-go-v1-data/repository.git"))]
    repository: PathBuf,
    #[arg(long)]
    force: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    if args.output.exists() && !args.force {
        return Err(format!(
            "output already exists; pass --force: {}",
            args.output.display()
        )
        .into());
    }
    let generator_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("generator");
    let work_dir = args.repository.with_extension("generator");
    fs::create_dir_all(&work_dir)?;
    let binary = work_dir.join("analyzer");
    command(
        "go",
        &["build", "-o", path_arg(&binary)?, "./main.go"],
        &generator_dir,
    )?;
    command(
        path_arg(&binary)?,
        &[
            "--repository",
            path_arg(&args.repository)?,
            "--output",
            path_arg(&args.output)?,
        ],
        &generator_dir,
    )?;
}

fn command(program: &str, args: &[&str], cwd: &PathBuf) -> Result<Output, Box<dyn Error>> {
    let output = Command::new(program).args(args).current_dir(cwd).output()?;
    if !output.status.success() {
        return Err(format!(
            "{program} failed: {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(output)
}

fn path_arg(path: &PathBuf) -> Result<&str, Box<dyn Error>> {
    path.to_str()
        .ok_or_else(|| format!("path is not UTF-8: {}", path.display()).into())
}
