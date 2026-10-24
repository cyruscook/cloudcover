use cloudcover_core::{
    ApiMethod, GoMethodReference, Language, MethodReference, Sdk, SdkMethodMapping,
};

#[test]
fn unresolved_operations_round_trip_and_legacy_mappings_remain_readable()
-> Result<(), Box<dyn std::error::Error>> {
    let mapping = SdkMethodMapping::new(
        Sdk::new("test", Language::Go),
        MethodReference::Go(GoMethodReference::new("pkg", Some("Client".into()), "Call")),
        vec![ApiMethod::new("s3", "GetObject")],
    );
    let legacy = serde_json::to_value(&mapping)?;
    assert!(legacy.get("unresolved_api_methods").is_none());
    assert!(serde_json::from_value::<SdkMethodMapping>(legacy)?.is_complete());
    let partial = mapping.with_unresolved_api_methods(vec![ApiMethod::new("missing", "Call")]);
    let decoded: SdkMethodMapping = serde_json::from_slice(&serde_json::to_vec(&partial)?)?;
    assert_eq!(partial, decoded);
    assert!(!decoded.is_complete());
    Ok(())
}
