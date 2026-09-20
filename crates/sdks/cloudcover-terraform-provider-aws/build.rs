use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use semver::Version;

#[path = "src/data.rs"]
mod data;

use data::{MappingState, PermissionDataFile, TerraformProviderAwsUnsupportedReason};

type BuildResult<T> = Result<T, Box<dyn Error>>;
type EncodedApiMethod = (u16, u16);
type EncodedRow = (u16, u16, u16, u16);

const MAGIC: &[u8; 8] = b"CCAWS001";
const HEADER_LEN: usize = 32;

#[derive(Clone, Copy)]
struct VersionIndex {
    start: u32,
    len: u32,
    unsupported_reason: Option<TerraformProviderAwsUnsupportedReason>,
}

#[derive(Default)]
struct IndexBuilder {
    string_ids: BTreeMap<String, u16>,
    strings: Vec<String>,
    api_list_ids: BTreeMap<Vec<EncodedApiMethod>, u16>,
    api_lists: Vec<Vec<EncodedApiMethod>>,
    row_ids: BTreeMap<EncodedRow, u16>,
    rows: Vec<EncodedRow>,
    snapshot_rows: Vec<u16>,
}

fn main() -> BuildResult<()> {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let data_dir = manifest_dir.join("data");
    println!("cargo:rerun-if-changed={}", data_dir.display());
    println!("cargo:rerun-if-changed=src/data.rs");

    let paths = data_paths(&data_dir)?;
    let mut state = MappingState::new();
    let mut builder = IndexBuilder::default();
    let mut versions = Vec::<(Version, VersionIndex)>::with_capacity(paths.len());

    for (version, path) in paths {
        println!("cargo:rerun-if-changed={}", path.display());
        let contents = fs::read_to_string(&path)?;
        let mut file: PermissionDataFile = serde_json::from_str(&contents)
            .map_err(|error| format!("{}: invalid JSON: {error}", path.display()))?;
        let unsupported_reason = file.unsupported_reason;
        let validated =
            data::validate_and_apply_file(&mut file, Some(&version.to_string()), &mut state)
                .map_err(|error| format!("{}: {error}", path.display()))?;
        if validated != version {
            return Err(format!(
                "{}: parsed version {validated} does not match filename version {version}",
                path.display()
            )
            .into());
        }
        let index = match unsupported_reason {
            Some(reason) => VersionIndex {
                start: 0,
                len: 0,
                unsupported_reason: Some(reason),
            },
            None => builder.add_snapshot(&state)?,
        };
        versions.push((version, index));
    }

    let (binary, offsets) = builder.encode(&versions)?;
    let generated = generate_code(&builder.strings, &versions, offsets)?;
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    write_bytes_if_changed(
        &out_dir.join("terraform_provider_aws_mappings.bin"),
        &binary,
    )?;
    write_string_if_changed(
        &out_dir.join("terraform_provider_aws_mappings.rs"),
        &generated,
    )?;
    Ok(())
}

fn data_paths(data_dir: &Path) -> BuildResult<Vec<(Version, PathBuf)>> {
    if !data_dir.is_dir() {
        return Err(format!(
            "permission data directory is missing: {}",
            data_dir.display()
        )
        .into());
    }

    let mut paths = BTreeMap::<Version, PathBuf>::new();
    for entry in fs::read_dir(data_dir)? {
        let path = entry?.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| {
                format!(
                    "permission data filename is not valid UTF-8: {}",
                    path.display()
                )
            })?;
        let version = data::parse_canonical_version(stem)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if paths.insert(version.clone(), path).is_some() {
            return Err(format!("duplicate permission data version {version}").into());
        }
    }
    if paths.is_empty() {
        return Err(format!(
            "permission data directory has no JSON files: {}",
            data_dir.display()
        )
        .into());
    }
    Ok(paths.into_iter().collect())
}

impl IndexBuilder {
    fn add_snapshot(&mut self, state: &MappingState) -> BuildResult<VersionIndex> {
        let start = to_u32(self.snapshot_rows.len(), "snapshot row offset")?;
        for ((kind, type_name, action), api_methods) in state {
            let kind = self.intern_string(kind)?;
            let type_name = self.intern_string(type_name)?;
            let action = self.intern_string(action)?;
            let encoded_api_methods = api_methods
                .iter()
                .map(|api_method| {
                    Ok((
                        self.intern_string(&api_method.service)?,
                        self.intern_string(&api_method.name)?,
                    ))
                })
                .collect::<BuildResult<Vec<_>>>()?;
            let api_list = self.intern_api_list(encoded_api_methods)?;
            let row = self.intern_row((kind, type_name, action, api_list))?;
            self.snapshot_rows.push(row);
        }
        Ok(VersionIndex {
            start,
            len: to_u32(state.len(), "snapshot row count")?,
            unsupported_reason: None,
        })
    }

    fn intern_string(&mut self, value: &str) -> BuildResult<u16> {
        if let Some(id) = self.string_ids.get(value) {
            return Ok(*id);
        }
        let id = to_u16(self.strings.len(), "string count")?;
        let owned = value.to_owned();
        self.strings.push(owned.clone());
        self.string_ids.insert(owned, id);
        Ok(id)
    }

    fn intern_api_list(&mut self, value: Vec<EncodedApiMethod>) -> BuildResult<u16> {
        if let Some(id) = self.api_list_ids.get(&value) {
            return Ok(*id);
        }
        let id = to_u16(self.api_lists.len(), "API method list count")?;
        self.api_lists.push(value.clone());
        self.api_list_ids.insert(value, id);
        Ok(id)
    }

    fn intern_row(&mut self, value: EncodedRow) -> BuildResult<u16> {
        if let Some(id) = self.row_ids.get(&value) {
            return Ok(*id);
        }
        let id = to_u16(self.rows.len(), "mapping row count")?;
        self.rows.push(value);
        self.row_ids.insert(value, id);
        Ok(id)
    }

    fn encode(
        &self,
        versions: &[(Version, VersionIndex)],
    ) -> BuildResult<(Vec<u8>, BinaryOffsets)> {
        let api_method_count = self.api_lists.iter().map(Vec::len).sum::<usize>();
        let api_methods_offset = HEADER_LEN;
        let api_lists_offset = checked_section_end(api_methods_offset, api_method_count, 4)?;
        let rows_offset = checked_section_end(api_lists_offset, self.api_lists.len(), 8)?;
        let row_ids_offset = checked_section_end(rows_offset, self.rows.len(), 8)?;
        let binary_len = checked_section_end(row_ids_offset, self.snapshot_rows.len(), 2)?;
        let mut binary = Vec::with_capacity(binary_len);

        binary.extend_from_slice(MAGIC);
        push_u32(&mut binary, to_u32(self.strings.len(), "string count")?);
        push_u32(&mut binary, to_u32(api_method_count, "API method count")?);
        push_u32(
            &mut binary,
            to_u32(self.api_lists.len(), "API method list count")?,
        );
        push_u32(&mut binary, to_u32(self.rows.len(), "mapping row count")?);
        push_u32(
            &mut binary,
            to_u32(self.snapshot_rows.len(), "snapshot row count")?,
        );
        push_u32(&mut binary, to_u32(versions.len(), "version count")?);

        let mut api_start = 0_u32;
        for list in &self.api_lists {
            for &(service, name) in list {
                push_u16(&mut binary, service);
                push_u16(&mut binary, name);
            }
        }
        for list in &self.api_lists {
            push_u32(&mut binary, api_start);
            let len = to_u32(list.len(), "API method list length")?;
            push_u32(&mut binary, len);
            api_start = api_start
                .checked_add(len)
                .ok_or("API method offset exceeds u32")?;
        }
        for &(kind, type_name, action, api_list) in &self.rows {
            push_u16(&mut binary, kind);
            push_u16(&mut binary, type_name);
            push_u16(&mut binary, action);
            push_u16(&mut binary, api_list);
        }
        for &row in &self.snapshot_rows {
            push_u16(&mut binary, row);
        }
        if binary.len() != binary_len {
            return Err(format!(
                "encoded binary length {} does not match calculated length {binary_len}",
                binary.len()
            )
            .into());
        }

        Ok((
            binary,
            BinaryOffsets {
                api_methods: api_methods_offset,
                api_lists: api_lists_offset,
                rows: rows_offset,
                row_ids: row_ids_offset,
            },
        ))
    }
}

#[derive(Clone, Copy)]
struct BinaryOffsets {
    api_methods: usize,
    api_lists: usize,
    rows: usize,
    row_ids: usize,
}

fn generate_code(
    strings: &[String],
    versions: &[(Version, VersionIndex)],
    offsets: BinaryOffsets,
) -> Result<String, std::fmt::Error> {
    let mut generated = String::new();
    generated.push_str("const STRINGS: &[&str] = &[\n");
    for value in strings {
        writeln!(generated, "    {value:?},")?;
    }
    generated.push_str("];\n\nconst PROVIDER_VERSIONS: &[&str] = &[\n");
    for (version, _) in versions {
        writeln!(generated, "    {:?},", version.to_string())?;
    }
    generated.push_str("];\n\nconst VERSION_INDEX: &[VersionIndex] = &[\n");
    for (_, index) in versions {
        let unsupported_reason = match index.unsupported_reason {
            Some(TerraformProviderAwsUnsupportedReason::AwsSdkGoV1) => {
                "Some(TerraformProviderAwsUnsupportedReason::AwsSdkGoV1)"
            }
            None => "None",
        };
        writeln!(
            generated,
            "    VersionIndex {{ start: {}, len: {}, unsupported_reason: {} }},",
            format_number(&index.start),
            format_number(&index.len),
            unsupported_reason,
        )?;
    }
    generated.push_str("];\n\nconst VERSION_LOOKUP: &[(&str, usize)] = &[\n");
    let mut lookup = versions
        .iter()
        .enumerate()
        .map(|(index, (version, _))| (version.to_string(), index))
        .collect::<Vec<_>>();
    lookup.sort_by(|left, right| left.0.cmp(&right.0));
    for (version, index) in lookup {
        writeln!(generated, "    ({version:?}, {index}),")?;
    }
    generated.push_str("];\n\n");
    writeln!(
        generated,
        "const API_METHODS_OFFSET: usize = {};",
        format_number(&offsets.api_methods)
    )?;
    writeln!(
        generated,
        "const API_LISTS_OFFSET: usize = {};",
        format_number(&offsets.api_lists)
    )?;
    writeln!(
        generated,
        "const ROWS_OFFSET: usize = {};",
        format_number(&offsets.rows)
    )?;
    writeln!(
        generated,
        "const ROW_IDS_OFFSET: usize = {};",
        format_number(&offsets.row_ids)
    )?;
    Ok(generated)
}

fn format_number(value: &impl ToString) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index != 0 && (digits.len() - index).is_multiple_of(3) {
            formatted.push('_');
        }
        formatted.push(character);
    }
    formatted
}

fn checked_section_end(start: usize, count: usize, width: usize) -> BuildResult<usize> {
    count
        .checked_mul(width)
        .and_then(|length| start.checked_add(length))
        .ok_or_else(|| "binary index size exceeds usize".into())
}

fn to_u32(value: usize, description: &str) -> BuildResult<u32> {
    u32::try_from(value).map_err(|_| format!("{description} exceeds u32").into())
}
fn to_u16(value: usize, description: &str) -> BuildResult<u16> {
    u16::try_from(value).map_err(|_| format!("{description} exceeds u16").into())
}

fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}
fn push_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn write_bytes_if_changed(path: &Path, contents: &[u8]) -> BuildResult<()> {
    match fs::read(path) {
        Ok(existing) if existing == contents => Ok(()),
        Ok(_) | Err(_) => {
            fs::write(path, contents)?;
            Ok(())
        }
    }
}

fn write_string_if_changed(path: &Path, contents: &str) -> BuildResult<()> {
    write_bytes_if_changed(path, contents.as_bytes())
}
