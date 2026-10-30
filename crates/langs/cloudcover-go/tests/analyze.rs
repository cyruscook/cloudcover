use std::{error::Error, path::PathBuf};

use cloudcover_core::GoMethodReference;
use cloudcover_go::{GoAnalysisError, analyze_dir};

fn basic_fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/basic")
}

#[test]
fn analyzes_interface_calls_in_fixture() -> Result<(), Box<dyn Error>> {
    let analysis = analyze_dir(basic_fixture_dir())?;
    let methods = analysis.methods();

    assert!(methods.contains(&GoMethodReference::new(
        "example.com/cloudcover/basic",
        Some("Client".to_owned()),
        "Do",
    )));
    assert!(methods.contains(&GoMethodReference::new(
        "example.com/cloudcover/basic",
        None,
        "helper",
    )));

    Ok(())
}

#[test]
fn rejects_file_paths_before_ffi() {
    let fixture_file = basic_fixture_dir().join("main.go");
    let err = analyze_dir(fixture_file).err();

    assert!(matches!(err, Some(GoAnalysisError::NotDirectory(_))));
}

#[test]
fn source_roots_keep_library_wrappers_and_skip_unused_dependency_functions()
-> Result<(), Box<dyn Error>> {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/source-roots");
    let analysis = analyze_dir(fixture)?;
    let methods = analysis.methods();
    for name in ["Live", "FromInit", "FromClosure"] {
        assert!(
            methods.contains(&GoMethodReference::new(
                "example.com/cloudcover/dependency",
                Some("Client".to_owned()),
                name,
            )),
            "missing reachable dependency method {name}"
        );
    }
    assert!(methods.contains(&GoMethodReference::new(
        "example.com/cloudcover/dependency",
        None,
        "Wrapper",
    )));
    assert!(!methods.contains(&GoMethodReference::new(
        "example.com/cloudcover/dependency",
        Some("Client".to_owned()),
        "Dead",
    )));
    Ok(())
}
