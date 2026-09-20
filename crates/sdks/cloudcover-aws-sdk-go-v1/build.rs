use std::{env, fs, path::Path};

#[path = "src/build_index.rs"]
mod build_index;
#[path = "src/data.rs"]
mod data;

const MODULE_PATH: &str = "github.com/aws/aws-sdk-go";

fn main() {
    if let Err(error) = run() {
        eprintln!("failed to build AWS SDK Go v1 mappings: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let input = Path::new("data/aws-sdk-go.json");
    println!("cargo:rerun-if-changed={}", input.display());
    let contents = fs::read_to_string(input)?;
    let mut file: data::SdkDataFile = serde_json::from_str(&contents)?;
    let encoded = build_index::IndexBuilder::from_releases(&mut file.releases)?.encode()?;
    if file.module_path != MODULE_PATH {
        return Err(format!("unexpected module path {:?}", file.module_path).into());
    }

    let out_dir_value = env::var("OUT_DIR")?;
    let out_dir = Path::new(&out_dir_value);
    fs::write(out_dir.join("sdk_mappings.bin"), encoded.binary)?;
    let generated = generate_source(
        &encoded.strings,
        &encoded.version_string_ids,
        &encoded.version_index,
        encoded.rows_offset,
        encoded.api_methods_offset,
        encoded.api_lists_offset,
        encoded.row_ids_offset,
    );
    fs::write(out_dir.join("sdk_mappings.rs"), generated)?;
    Ok(())
}

#[allow(clippy::format_collect, clippy::uninlined_format_args)]
fn generate_source(
    strings: &[String],
    version_string_ids: &[u16],
    version_index: &[(u32, u32)],
    rows_offset: usize,
    api_methods_offset: usize,
    api_lists_offset: usize,
    row_ids_offset: usize,
) -> String {
    let versions = version_string_ids
        .iter()
        .map(|index| format!("    {:?},", strings[usize::from(*index)]))
        .collect::<String>();
    let strings = strings
        .iter()
        .map(|value| format!("    {:?},", value))
        .collect::<String>();
    let index = version_index
        .iter()
        .map(|(start, len)| format!("    VersionIndex {{ start: {start}, len: {len} }},"))
        .collect::<String>();
    format!(
        "#[allow(clippy::unreadable_literal)]\n\
const ROWS_OFFSET: usize = {rows_offset};\n\
#[allow(clippy::unreadable_literal)]\n\
const API_METHODS_OFFSET: usize = {api_methods_offset};\n\
#[allow(clippy::unreadable_literal)]\n\
const API_LISTS_OFFSET: usize = {api_lists_offset};\n\
#[allow(clippy::unreadable_literal)]\n\
const ROW_IDS_OFFSET: usize = {row_ids_offset};\n\
const STRINGS: &[&str] = &[\n{strings}];\n\
const SDK_VERSIONS: &[&str] = &[\n{versions}];\n\
#[allow(clippy::unreadable_literal)]\n\
const VERSION_INDEX: &[VersionIndex] = &[\n{index}];\n"
    )
}
