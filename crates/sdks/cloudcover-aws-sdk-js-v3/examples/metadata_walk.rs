//! Walk every recorded release while keeping the complete metadata graph linked.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut total = 0usize;
    for package in cloudcover_aws_sdk_js_v3::service_modules() {
        for version in cloudcover_aws_sdk_js_v3::service_versions(package).ok_or("package")? {
            let rows = cloudcover_aws_sdk_js_v3::service_method_mappings(package, version)
                .ok_or("exact release")?;
            for row in rows {
                total = total.wrapping_add(row.method.len() + row.api_methods.len());
            }
        }
    }
    println!("{total}");
    Ok(())
}
