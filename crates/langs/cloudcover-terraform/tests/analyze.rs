use std::{error::Error, path::PathBuf};

use cloudcover_core::TerraformMethodReference;
use cloudcover_terraform::analyze_dir;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../cloudcover-cli/tests/fixtures/aws-terraform")
}

#[test]
fn analyzes_reachable_initialized_modules_and_ignores_stale_manifest_entries()
-> Result<(), Box<dyn Error>> {
    let analysis = analyze_dir(fixture_dir())?;

    assert_eq!(analysis.provider_version(), "6.64.0");
    for action in ["create", "read", "update", "delete"] {
        assert!(analysis.methods().contains(&TerraformMethodReference::new(
            "resource",
            "aws_s3_bucket",
            action,
        )));
    }
    for action in ["create", "read", "update", "delete"] {
        assert!(analysis.methods().contains(&TerraformMethodReference::new(
            "resource",
            "aws_acm_certificate_validation",
            action,
        )));
    }
    assert!(analysis.methods().contains(&TerraformMethodReference::new(
        "data_source",
        "aws_ecr_image",
        "read",
    )));
    assert!(analysis.methods().contains(&TerraformMethodReference::new(
        "data_source",
        "aws_caller_identity",
        "read",
    )));

    Ok(())
}
