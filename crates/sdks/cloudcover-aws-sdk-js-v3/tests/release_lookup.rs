#[path = "../build.rs"]
mod build_script;

use std::{env, error::Error, fs, process::Command};

const FIXTURE: &str = r#"
{
  "packages": [
    {
      "package": "@aws-sdk/client-bedrock-runtime",
      "releases": [
        {
          "version": "3.1136.0",
          "remove": [],
          "upsert": [{
            "receiver": "BedrockRuntimeClient",
            "method": "sendInvokeModel",
            "api_methods": [{"service": "bedrock", "name": "InvokeModel"}]
          }]
        },
        {
          "version": "3.1137.0",
          "remove": [["BedrockRuntimeClient", "sendInvokeModel"]],
          "upsert": [{
            "receiver": "BedrockRuntimeClient",
            "method": "sendConverse",
            "api_methods": [{"service": "bedrock", "name": "Converse"}]
          }]
        }
      ]
    },
    {
      "package": "@aws-sdk/client-s3",
      "releases": [{
        "version": "3.2.0",
        "remove": [],
        "upsert": [{
          "receiver": "S3Client",
          "method": "sendPutObject",
          "api_methods": [{"service": "s3", "name": "PutObject"}]
        }]
      },
      {"version": "3.10.0", "remove": [], "upsert": []},
      {"version": "3.100.0", "remove": [], "upsert": []},
      {"version": "3.1137.0", "remove": [], "upsert": []}]
    }
  ]
}
"#;

#[test]
fn generated_lookup_supports_arbitrary_packages_and_exact_releases() -> Result<(), Box<dyn Error>> {
    let generated = build_script::generate(FIXTURE)?;
    assert_eq!(generated.matches("static RELEASE_").count(), 3);
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
    for version in ["3.2.0", "3.10.0", "3.100.0", "3.1137.0"] {{
        assert_eq!(service_method_mappings("@aws-sdk/client-s3", version).unwrap()[0].method, "sendPutObject");
    }}
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
fn every_generated_release_is_indexed_at_its_exact_version() -> Result<(), Box<dyn Error>> {
    for package in cloudcover_aws_sdk_js_v3::service_modules() {
        let versions = cloudcover_aws_sdk_js_v3::service_versions(package)
            .ok_or_else(|| format!("{package} has no indexed versions"))?;
        assert_ne!(
            versions,
            &[] as &[&str],
            "{package} has no generated versions"
        );

        for version in versions {
            let mappings = cloudcover_aws_sdk_js_v3::service_method_mappings(package, version)
                .ok_or_else(|| format!("{package}@{version} has no indexed mappings"))?;
            assert!(
                !mappings.is_empty(),
                "{package}@{version} has no generated mappings"
            );
        }
    }
    Ok(())
}

#[test]
fn historical_releases_preserve_operation_availability_and_retired_packages()
-> Result<(), Box<dyn Error>> {
    const S3: &str = "@aws-sdk/client-s3";
    let oldest = cloudcover_aws_sdk_js_v3::service_method_mappings(S3, "3.0.0")
        .ok_or("missing first stable S3 release")?;
    let latest_version = cloudcover_aws_sdk_js_v3::service_versions(S3)
        .and_then(|versions| versions.last())
        .ok_or("missing S3 versions")?;
    let latest = cloudcover_aws_sdk_js_v3::service_method_mappings(S3, latest_version)
        .ok_or("missing latest S3 release")?;
    assert!(
        oldest
            .iter()
            .any(|mapping| mapping.method == "GetObjectCommand")
    );
    assert!(
        !oldest
            .iter()
            .any(|mapping| mapping.method == "GetObjectAttributesCommand")
    );
    assert!(
        latest
            .iter()
            .any(|mapping| mapping.method == "GetObjectAttributesCommand")
    );
    assert!(cloudcover_aws_sdk_js_v3::service_versions("@aws-sdk/client-codestar").is_some());
    Ok(())
}

#[test]
fn duplicate_exact_package_release_is_rejected() {
    let duplicate = r#"
    {
      "packages": [{
        "package": "@aws-sdk/client-s3",
        "releases": [
          {"version": "3.1137.0", "remove": [], "upsert": []},
          {"version": "3.1137.0", "remove": [], "upsert": []}
        ]
      }]
    }
    "#;
    assert!(build_script::generate(duplicate).is_err());
}
