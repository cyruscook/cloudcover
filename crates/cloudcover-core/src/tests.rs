use crate::{
    ApiMethod, GoMethodReference, Language, MethodReference, PythonMethodReference, Sdk,
    SdkMethodMapping,
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
    let sdk = Sdk::new("boto3", Language::Python);

    assert_eq!(sdk.name(), "boto3");
    assert_eq!(sdk.language(), Language::Python);
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
