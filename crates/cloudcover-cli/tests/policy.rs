use std::{error::Error, io, path::PathBuf, process::Command};

use serde_json::Value;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/aws-s3")
}

fn terraform_fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/aws-terraform")
}

fn run_policy(arguments: &[&str]) -> Result<std::process::Output, Box<dyn Error>> {
    Ok(Command::new(env!("CARGO_BIN_EXE_cloudcover"))
        .args(arguments)
        .output()?)
}

fn parse_stdout_json(output: &std::process::Output) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn collect_actions(policy: &Value) -> Result<Vec<String>, Box<dyn Error>> {
    let statements = policy["Statement"]
        .as_array()
        .ok_or_else(|| io::Error::other("policy statements should be an array"))?;
    let mut actions = statements
        .iter()
        .flat_map(|statement| match &statement["Action"] {
            Value::Array(actions) => actions.iter().collect::<Vec<_>>(),
            value => vec![value],
        })
        .map(|action| {
            action
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| io::Error::other("policy actions should be strings"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    actions.sort();
    actions.dedup();
    Ok(actions)
}

#[test]
fn emits_expected_s3_read_policy() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_dir().to_string_lossy().into_owned();
    let output = run_policy(&["policy", "--language", "go", &fixture])?;

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty(), "stderr should be empty");

    let policy = parse_stdout_json(&output)?;
    let actions = collect_actions(&policy)?;
    assert!(actions.contains(&"s3:GetObject".to_owned()));
    assert!(actions.contains(&"s3:GetObjectLegalHold".to_owned()));
    assert!(actions.contains(&"s3:GetObjectRetention".to_owned()));
    assert!(actions.contains(&"s3:GetObjectTagging".to_owned()));
    assert!(actions.contains(&"s3:GetObjectVersion".to_owned()));
    assert!(actions.contains(&"s3-object-lambda:GetObject".to_owned()));
    assert!(!actions.contains(&"s3:PutObject".to_owned()));

    Ok(())
}

#[test]
fn shorthand_matches_explicit_language_output() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_dir().to_string_lossy().into_owned();
    let explicit = run_policy(&["policy", "--language", "go", &fixture])?;
    let shorthand = run_policy(&["policy", &fixture])?;

    assert!(
        explicit.status.success(),
        "explicit stderr: {}",
        String::from_utf8_lossy(&explicit.stderr)
    );
    assert!(
        shorthand.status.success(),
        "shorthand stderr: {}",
        String::from_utf8_lossy(&shorthand.stderr)
    );
    assert_eq!(
        parse_stdout_json(&explicit)?,
        parse_stdout_json(&shorthand)?
    );

    Ok(())
}

#[test]
fn emits_full_lifecycle_policy_for_initialized_terraform_modules() -> Result<(), Box<dyn Error>> {
    let fixture = terraform_fixture_dir().to_string_lossy().into_owned();
    let output = run_policy(&["policy", "--language", "terraform", &fixture])?;

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let actions = collect_actions(&parse_stdout_json(&output)?)?;
    for action in [
        "ecr:DescribeImages",
        "ecr:DescribeRepositories",
        "s3:CreateBucket",
        "s3:DeleteBucket",
        "s3:PutBucketPolicy",
    ] {
        assert!(actions.contains(&action.to_owned()), "missing {action}");
    }
    assert!(!actions.contains(&"sts:GetCallerIdentity".to_owned()));
    assert!(
        !actions
            .iter()
            .any(|action| action == "*" || action.ends_with(":*"))
    );

    Ok(())
}

#[test]
fn unsupported_language_is_usage_error() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_dir().to_string_lossy().into_owned();
    let output = run_policy(&["policy", "--language", "python", &fixture])?;

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("Usage: cloudcover policy [--language go|terraform] <PATH>"));

    Ok(())
}
