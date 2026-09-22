#[path = "../build.rs"]
mod build_script;

use std::{env, error::Error, fs, process::Command};

const FIXTURE: &str = r#"
{
  "packages": [
    {
      "package": "@aws-sdk/client-bedrock-runtime",
      "version": "3.1137.0",
      "mappings": [{
        "receiver": "BedrockRuntimeClient",
        "method": "sendConverse",
        "api_methods": [{"service": "bedrock", "name": "Converse"}]
      }]
    },
    {
      "package": "@aws-sdk/client-s3",
      "version": "3.1137.0",
      "mappings": [{
        "receiver": "S3Client",
        "method": "sendPutObject",
        "api_methods": [{"service": "s3", "name": "PutObject"}]
      }]
    },
    {
      "package": "@aws-sdk/client-bedrock-runtime",
      "version": "3.1136.0",
      "mappings": [{
        "receiver": "BedrockRuntimeClient",
        "method": "sendInvokeModel",
        "api_methods": [{"service": "bedrock", "name": "InvokeModel"}]
      }]
    }
  ]
}
"#;

#[test]
fn generated_lookup_supports_arbitrary_packages_and_exact_releases() -> Result<(), Box<dyn Error>> {
    let generated = build_script::generate(FIXTURE)?;
    let temp_dir = env::temp_dir().join(format!(
        "cloudcover-js-v3-release-lookup-{}",
        std::process::id()
    ));
    fs::create_dir_all(&temp_dir)?;
    fs::write(temp_dir.join("generated.rs"), generated)?;

    let library = env::var("CARGO_MANIFEST_DIR")?;
    let harness = format!(
        r#"
include!({library:?});

fn main() {{
    const BEDROCK: &str = "@aws-sdk/client-bedrock-runtime";
    assert_eq!(service_modules(), &[BEDROCK, "@aws-sdk/client-s3"]);
    assert_eq!(service_versions(BEDROCK), Some(&["3.1136.0", "3.1137.0"][..]));
    assert_eq!(service_method_mappings(BEDROCK, "3.1136.0").unwrap()[0].method, "sendInvokeModel");
    assert_eq!(service_method_mappings(BEDROCK, "3.1137.0").unwrap()[0].method, "sendConverse");
    assert!(service_method_mappings(BEDROCK, "3.1135.0").is_none());
    assert!(service_method_mappings("@aws-sdk/client-not-present", "3.1137.0").is_none());
    assert!(service_versions("@aws-sdk/client-not-present").is_none());
}}
"#,
        library = format!("{library}/src/lib.rs")
    );
    let harness_path = temp_dir.join("harness.rs");
    let executable_path = temp_dir.join("harness");
    fs::write(&harness_path, harness)?;

    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let compilation = Command::new(rustc)
        .arg("--edition=2024")
        .arg(&harness_path)
        .arg("-o")
        .arg(&executable_path)
        .env("OUT_DIR", &temp_dir)
        .output()?;
    assert!(
        compilation.status.success(),
        "fixture compilation failed: {}",
        String::from_utf8_lossy(&compilation.stderr)
    );
    let execution = Command::new(&executable_path).output()?;
    assert!(
        execution.status.success(),
        "fixture lookup failed: {}",
        String::from_utf8_lossy(&execution.stderr)
    );
    fs::remove_dir_all(temp_dir)?;
    Ok(())
}

#[test]
fn duplicate_exact_package_release_is_rejected() {
    let duplicate = r#"
    {
      "packages": [
        {"package": "@aws-sdk/client-s3", "version": "3.1137.0", "mappings": []},
        {"package": "@aws-sdk/client-s3", "version": "3.1137.0", "mappings": []}
      ]
    }
    "#;
    assert!(build_script::generate(duplicate).is_err());
}
