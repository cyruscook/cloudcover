use std::{error::Error, io, path::PathBuf, process::Command};

use serde_json::Value;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/aws-s3-resolved")
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
fn emits_expected_s3_read_policy_for_pre_session_sdk() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_dir().to_string_lossy().into_owned();
    let output = run_policy(&["policy", "--language", "go", &fixture])?;

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("FAS: included possible downstream requirements")
    );

    let policy = parse_stdout_json(&output)?;
    let actions = collect_actions(&policy)?;
    assert!(actions.contains(&"s3:GetObject".to_owned()));
    assert!(actions.contains(&"kms:Decrypt".to_owned()));
    assert!(actions.contains(&"kms:GenerateDataKey".to_owned()));
    assert!(
        policy["Statement"]
            .as_array()
            .ok_or("missing statements")?
            .iter()
            .any(|statement| {
                statement["Condition"]["StringEquals"]["kms:ViaService"]
                    == "s3.${Region}.amazonaws.com"
            })
    );
    assert!(actions.contains(&"s3:GetObjectLegalHold".to_owned()));
    assert!(actions.contains(&"s3:GetObjectRetention".to_owned()));
    assert!(actions.contains(&"s3:GetObjectTagging".to_owned()));
    assert!(actions.contains(&"s3:GetObjectVersion".to_owned()));
    assert!(actions.contains(&"s3-object-lambda:GetObject".to_owned()));
    assert!(!actions.contains(&"s3:PutObject".to_owned()));
    assert!(!actions.contains(&"s3express:CreateSession".to_owned()));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("S3 Express:"));

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
        "bedrock-agentcore:CreateGateway",
        "cognito-idp:CreateUserPool",
        "cognito-identity:CreateIdentityPool",
        "apigateway:POST",
        "events:PutRule",
        "scheduler:CreateSchedule",
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
fn emits_terraform_hcl_policy_document() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_dir().to_string_lossy().into_owned();
    let output = run_policy(&[
        "policy",
        "--format",
        "terraform",
        "--language",
        "go",
        &fixture,
    ])?;

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("FAS: included possible downstream requirements")
    );
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains(r#"data "aws_iam_policy_document" "cloudcover""#));
    assert!(stdout.contains("statement {"));
    assert!(stdout.contains(r#""s3:GetObject""#));
    assert!(stdout.contains(r#""s3-object-lambda:GetObject""#));
    assert!(stdout.contains(r#""arn:$${Partition}:s3:::$${BucketName}/$${ObjectName}""#));

    Ok(())
}

#[test]
fn unsupported_language_is_usage_error() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_dir().to_string_lossy().into_owned();
    let output = run_policy(&["policy", "--language", "python", &fixture])?;

    assert_eq!(output.status.code(), Some(2));
    Ok(())
}

#[test]
fn missing_lifecycle_mapping_reports_incomplete_coverage() -> Result<(), Box<dyn Error>> {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/aws-terraform-missing");
    let output = run_policy(&[
        "policy",
        "--language",
        "terraform",
        &fixture.to_string_lossy(),
    ])?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Terraform reference is not mapped"));
    assert_eq!(output.stdout, Vec::<u8>::new());
    Ok(())
}

#[test]
fn modern_s3_helpers_resolve_documented_session_authorization_with_warning()
-> Result<(), Box<dyn Error>> {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/aws-s3")
        .to_string_lossy()
        .into_owned();
    for format in ["json", "terraform"] {
        let output = run_policy(&["policy", "--format", format, "--language", "go", &fixture])?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let policy = String::from_utf8(output.stdout)?;
        assert!(policy.contains("s3:GetObject"));
        assert!(policy.contains("s3express:CreateSession"));
        assert!(!policy.contains("\"s3:CreateSession\""));
        assert!(policy.contains("s3express:"));
        let warning = String::from_utf8(output.stderr)?;
        assert!(warning.contains("S3 Express: this conservative policy"));
        assert!(warning.contains("ordinary general-purpose GetObject does not require this grant"));
        assert!(warning.contains("Bucket, endpoint and session mode are not inferred"));
    }
    Ok(())
}

#[test]
fn unrelated_missing_authorization_still_rejects_mixed_modern_s3_output()
-> Result<(), Box<dyn Error>> {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/aws-s3-unresolved")
        .to_string_lossy()
        .into_owned();
    for format in ["json", "terraform"] {
        let output = run_policy(&["policy", "--format", format, "--language", "go", &fixture])?;
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(output.stdout, Vec::<u8>::new());
        assert!(
            String::from_utf8(output.stderr)?
                .contains("unresolved AWS authorization coverage for s3:RenameObject")
        );
    }
    Ok(())
}

#[test]
fn document_client_response_properties_preserve_existing_dynamodb_policy()
-> Result<(), Box<dyn Error>> {
    use cloudcover_core::{ApiMethod, CloudProvider};

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../langs/cloudcover-javascript/analyzer/fixtures/dynamodb");
    let expected = cloudcover_aws::AwsProvider::new().permissions_policy(&[
        ApiMethod::new("dynamodb", "GetItem"),
        ApiMethod::new("dynamodb", "DescribeTable"),
    ])?;
    for language in ["javascript", "typescript"] {
        let output = run_policy(&["policy", "--language", language, &fixture.to_string_lossy()])?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let policy = parse_stdout_json(&output)?;
        assert_eq!(policy, expected);
        assert!(collect_actions(&policy)?.contains(&"dynamodb:GetItem".to_owned()));
    }
    Ok(())
}

#[test]
#[cfg(unix)]
fn conditional_grants_have_diagnostics_and_explicit_opt_in_in_both_formats()
-> Result<(), Box<dyn Error>> {
    use std::{fs, os::unix::fs::symlink};
    let root = std::env::temp_dir().join(format!("cloudcover-conditional-{}", std::process::id()));
    fs::create_dir_all(root.join("node_modules/@aws-sdk/client-appstream"))?;
    let result = (|| -> Result<(), Box<dyn Error>> {
        symlink(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
                "../langs/cloudcover-javascript/analyzer/fixtures/dynamodb/node_modules/typescript",
            ),
            root.join("node_modules/typescript"),
        )?;
        fs::write(
            root.join("package.json"),
            r#"{"private":true,"type":"module"}"#,
        )?;
        fs::write(
            root.join("main.ts"),
            "import { CreateFleetCommand } from '@aws-sdk/client-appstream'; new CreateFleetCommand({});",
        )?;
        let sdk = root.join("node_modules/@aws-sdk/client-appstream");
        fs::write(
            sdk.join("package.json"),
            r#"{"name":"@aws-sdk/client-appstream","version":"3.1172.0","types":"index.d.ts","main":"index.js"}"#,
        )?;
        fs::write(
            sdk.join("index.d.ts"),
            "export declare class CreateFleetCommand { constructor(input: unknown); }",
        )?;
        fs::write(
            sdk.join("index.js"),
            "throw new Error('Analyzed SDK code must never execute');",
        )?;
        for format in ["json", "terraform"] {
            for include in [false, true] {
                let path = root.to_string_lossy();
                let mut args = vec![
                    "policy",
                    "--language",
                    "typescript",
                    "--format",
                    format,
                    &path,
                ];
                if include {
                    args.push("--include-conditional");
                }
                let output = run_policy(&args)?;
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert!(output.status.success(), "{stderr}");
                assert!(stderr.contains("appstream:CreateFleet -> iam:PassRole"));
                assert!(stderr.contains("appstream:CreateFleet -> appstream:TagResource"));
                assert!(stderr.contains("https://docs.aws.amazon.com/"));
                assert!(stderr.contains(if include {
                    "included by explicit opt-in"
                } else {
                    "excluded from ordinary grants"
                }));
                let stdout = String::from_utf8(output.stdout)?;
                assert!(stdout.contains("appstream:CreateFleet"));
                assert!(!stdout.contains("iam:PassRole"));
                assert_eq!(stdout.contains("appstream:TagResource"), include);
                assert!(!stdout.contains("iam:PassedToService"));
                assert!(stderr.contains("manual grant required; not rendered"));
                assert!(stderr.contains("concrete, validated requested role ARN"));
                assert!(stderr.contains("iam:PassedToService = appstream.amazonaws.com"));
            }
        }
        Ok(())
    })();
    fs::remove_dir_all(root)?;
    result
}

#[test]
fn emits_terraform_policy_without_module_manifest() -> Result<(), Box<dyn Error>> {
    let fixture = terraform_fixture_dir().with_file_name("aws-terraform-root-only");
    let output = run_policy(&[
        "policy",
        "--language",
        "terraform",
        &fixture.to_string_lossy(),
    ])?;
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actions = collect_actions(&parse_stdout_json(&output)?)?;
    for action in ["s3:CreateBucket", "s3:DeleteBucket", "ecr:DescribeImages"] {
        assert!(actions.contains(&action.to_owned()), "missing {action}");
    }
    Ok(())
}
