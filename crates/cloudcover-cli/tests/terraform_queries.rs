use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

// Controlled initialized-module fixtures; no Terraform/provider execution or AWS.
struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Result<Self, Box<dyn Error>> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "cloudcover terraform queries-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    let fixture = Self(path);
                    fixture.write(".terraform.lock.hcl", "provider \"registry.terraform.io/hashicorp/aws\" {\n version = \"6.64.0\"\n hashes = []\n}\n")?;
                    return Ok(fixture);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn write(&self, name: &str, content: &str) -> Result<(), Box<dyn Error>> {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().ok_or("fixture has no parent")?)?;
        fs::write(path, content)?;
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn policy(path: &Path, format: &str, conditional: bool) -> Result<Output, Box<dyn Error>> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cloudcover"));
    command.args(["policy", "--language", "terraform", "--format", format]);
    if conditional {
        command.arg("--include-conditional");
    }
    Ok(command.arg(path).output()?)
}

fn assert_rejected(path: &Path, expected_file: &str) -> Result<(), Box<dyn Error>> {
    for format in ["json", "terraform"] {
        for conditional in [false, true] {
            let output = policy(path, format, conditional)?;
            let stderr = String::from_utf8(output.stderr)?;
            assert_eq!(output.status.code(), Some(1), "{stderr}");
            assert_eq!(
                output.stdout,
                Vec::<u8>::new(),
                "partial {format} policy escaped"
            );
            assert!(
                stderr.contains("unsupported Terraform list queries"),
                "{stderr}"
            );
            assert!(stderr.contains(expected_file), "{stderr}");
            assert!(stderr.contains("no policy was generated"), "{stderr}");
        }
    }
    Ok(())
}

#[test]
fn query_files_and_list_blocks_reject_empty_and_partial_cli_output() -> Result<(), Box<dyn Error>> {
    for (filename, query) in [
        (
            "users.tfquery.hcl",
            "list \"aws_iam_user\" \"users\" { provider = aws }",
        ),
        (
            "users.tfquery.json",
            r#"{"list":{"aws_iam_user":{"users":{"provider":"aws"}}}}"#,
        ),
        (
            "users.tf",
            "list \"aws_iam_user\" \"users\" { provider = aws }",
        ),
        ("users.tf.json", r#"{"list":{"aws_iam_user":{"users":{}}}}"#),
    ] {
        for mixed in [false, true] {
            let fixture = Fixture::new()?;
            if mixed {
                fixture.write("main.tf", "resource \"aws_s3_bucket\" \"root\" {}\n")?;
            }
            fixture.write(filename, query)?;
            assert_rejected(&fixture.0, filename)?;
            if filename.contains(".tfquery.") {
                assert_rejected(&fixture.0.join(filename), filename)?;
            }
        }
    }
    Ok(())
}

#[test]
fn reachable_child_query_rejects_the_whole_cli_request() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    fixture.write(
        "main.tf",
        "resource \"aws_s3_bucket\" \"root\" {}\nmodule \"child\" { source = \"example/child/aws\" }\n",
    )?;
    fixture.write(
        ".terraform/modules/child/users.tfquery.json",
        r#"{"list":{"aws_iam_user":{"users":{}}}}"#,
    )?;
    fixture.write(
        ".terraform/modules/modules.json",
        r#"{"Modules":[{"Key":"child","Dir":".terraform/modules/child"}]}"#,
    )?;
    assert_rejected(&fixture.0, "users.tfquery.json")?;
    Ok(())
}

#[test]
fn managed_only_json_and_ignored_queries_preserve_exact_cli_output() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    fixture.write("main.tf", "resource \"aws_s3_bucket\" \"root\" {}\n")?;
    let mut baseline = Vec::new();
    for format in ["json", "terraform"] {
        for conditional in [false, true] {
            let output = policy(&fixture.0, format, conditional)?;
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains("s3:CreateBucket"));
            baseline.push((format, conditional, output.stdout));
        }
    }
    fs::remove_file(fixture.0.join("main.tf"))?;
    fixture.write(
        "main.tf.json",
        r#"{"resource":{"aws_s3_bucket":{"root":{}}},"locals":{"list":"ordinary attribute"}}"#,
    )?;
    fixture.write(".hidden.tfquery.json", "{}")?;
    fixture.write("backup.tfquery.hcl~", "{}")?;
    fixture.write("stale/users.tfquery.json", "{}")?;
    fixture.write(
        ".terraform/modules/modules.json",
        r#"{"Modules":[{"Key":"stale","Dir":"stale"}]}"#,
    )?;
    for (format, conditional, expected) in baseline {
        let output = policy(&fixture.0, format, conditional)?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, expected);
    }
    Ok(())
}

#[test]
fn default_and_explicit_go_paths_keep_ignoring_terraform_query_files() -> Result<(), Box<dyn Error>>
{
    let fixture = Fixture::new()?;
    fixture.write("go.mod", "module example.com/plain\n\ngo 1.24.0\n")?;
    fixture.write("main.go", "package plain\nfunc Example() {}\n")?;
    for explicit in [false, true] {
        let run = || -> Result<Output, std::io::Error> {
            let mut command = Command::new(env!("CARGO_BIN_EXE_cloudcover"));
            command.arg("policy");
            if explicit {
                command.args(["--language", "go"]);
            }
            command.arg(&fixture.0).output()
        };
        let before = run()?;
        fixture.write("users.tfquery.json", "{}")?;
        let after = run()?;
        assert_eq!(after.status.code(), before.status.code());
        assert_eq!(after.stdout, before.stdout);
        assert_eq!(after.stderr, before.stderr);
        assert!(
            !String::from_utf8_lossy(&after.stderr).contains("unsupported Terraform list queries")
        );
        fs::remove_file(fixture.0.join("users.tfquery.json"))?;
    }
    Ok(())
}

#[test]
fn ordinary_file_arguments_still_require_a_module_directory() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    fixture.write("main.tf", "resource \"aws_s3_bucket\" \"root\" {}\n")?;
    for format in ["json", "terraform"] {
        let output = policy(&fixture.0.join("main.tf"), format, false)?;
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(output.stdout, Vec::<u8>::new());
        let stderr = String::from_utf8(output.stderr)?;
        assert!(stderr.contains("path is not a directory"), "{stderr}");
        assert!(
            !stderr.contains("unsupported Terraform list queries"),
            "{stderr}"
        );
    }
    Ok(())
}
