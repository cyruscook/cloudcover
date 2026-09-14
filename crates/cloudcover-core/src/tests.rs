use crate::{
    ApiMethod, GoMethodReference, Language, MethodReference, PythonMethodReference, Sdk,
    SdkMethodMapping, TerraformMethodReference,
};

#[test]
fn api_method_constructor_and_accessors() {
    let method = ApiMethod::new("s3", "GetObject");

    assert_eq!(method.service(), "s3");
    assert_eq!(method.name(), "GetObject");
}

#[test]
fn api_methods_sort_by_service_then_name() {
    let mut methods = vec![
        ApiMethod::new("s3", "PutObject"),
        ApiMethod::new("ec2", "DescribeInstances"),
        ApiMethod::new("s3", "GetObject"),
    ];

    methods.sort();

    assert_eq!(
        methods,
        vec![
            ApiMethod::new("ec2", "DescribeInstances"),
            ApiMethod::new("s3", "GetObject"),
            ApiMethod::new("s3", "PutObject"),
        ]
    );
}

#[test]
fn sdk_constructor_and_accessors() {
    let sdk = Sdk::new("terraform-provider-aws", Language::Terraform);
    let versioned = Sdk::new("terraform-provider-aws", Language::Terraform).with_version("6.64.0");

    assert_eq!(sdk.name(), "terraform-provider-aws");
    assert_eq!(sdk.language(), Language::Terraform);
    assert_eq!(sdk.version(), None);
    assert_eq!(versioned.version(), Some("6.64.0"));
    assert_ne!(sdk, versioned);
    assert_eq!(sdk, Sdk::new("terraform-provider-aws", Language::Terraform));
}

#[test]
fn sdk_serialization_omits_only_unversioned_version() -> Result<(), serde_json::Error> {
    let unversioned = serde_json::to_value(Sdk::new("boto3", Language::Python))?;
    let versioned = serde_json::to_value(
        Sdk::new("terraform-provider-aws", Language::Terraform).with_version("6.64.0"),
    )?;
    assert_eq!(
        unversioned,
        serde_json::json!({"name": "boto3", "language": "Python"})
    );
    assert_eq!(
        versioned,
        serde_json::json!({
            "name": "terraform-provider-aws",
            "language": "Terraform",
            "version": "6.64.0",
        })
    );
    Ok(())
}

#[test]
fn python_method_reference_constructor_and_accessors() {
    let method = PythonMethodReference::new("boto3", Some("s3".to_owned()), "get_object");

    assert_eq!(method.module(), "boto3");
    assert_eq!(method.receiver(), Some("s3"));
    assert_eq!(method.name(), "get_object");
}

#[test]
fn go_method_reference_constructor_and_accessors() {
    let method = GoMethodReference::new("example.com/myapp", Some("MyType".to_owned()), "MyMethod");

    assert_eq!(method.package(), "example.com/myapp");
    assert_eq!(method.receiver(), Some("MyType"));
    assert_eq!(method.name(), "MyMethod");
}

#[test]
fn terraform_method_reference_constructor_and_accessors() {
    let method = TerraformMethodReference::new("resource", "aws_s3_bucket", "create");

    assert_eq!(method.kind(), "resource");
    assert_eq!(method.type_name(), "aws_s3_bucket");
    assert_eq!(method.action(), "create");
}

#[test]
fn sdk_method_mapping_constructor_and_accessors() {
    let mapping = SdkMethodMapping::new(
        Sdk::new("boto3", Language::Python),
        MethodReference::Python(PythonMethodReference::new(
            "boto3",
            Some("s3".to_owned()),
            "get_object",
        )),
        vec![ApiMethod::new("s3", "GetObject")],
    );

    assert_eq!(mapping.sdk(), &Sdk::new("boto3", Language::Python));
    assert_eq!(
        mapping.method(),
        &MethodReference::Python(PythonMethodReference::new(
            "boto3",
            Some("s3".to_owned()),
            "get_object",
        ))
    );
    assert_eq!(mapping.api_methods(), &[ApiMethod::new("s3", "GetObject")]);
}
