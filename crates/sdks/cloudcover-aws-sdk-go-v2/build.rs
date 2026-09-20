use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

#[allow(dead_code)]
#[path = "src/data.rs"]
mod data;

use data::MappingState;

type BuildResult<T> = Result<T, Box<dyn Error>>;
type EncodedApiMethod = (u16, u16);
type EncodedRow = (u16, u16, u16, u16);

const MAGIC: &[u8; 8] = b"CCGO001\0";
const HEADER_LEN: usize = 32;

#[derive(Clone, Copy)]
struct VersionIndex {
    start: u32,
    len: u32,
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
    snapshot_ids: BTreeMap<Vec<u16>, VersionIndex>,
}

fn main() -> BuildResult<()> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/data.rs");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let data_dir = manifest_dir.join("data");
    println!("cargo:rerun-if-changed={}", data_dir.display());

    let paths = data_paths(&data_dir)?;
    let mut builder = IndexBuilder::default();
    let mut versions = Vec::<(String, String, VersionIndex)>::new();

    for (module_path, path) in paths {
        println!("cargo:rerun-if-changed={}", path.display());
        let contents = fs::read_to_string(&path)?;
        let mut file: data::SdkDataFile = serde_json::from_str(&contents)
            .map_err(|error| format!("{}: invalid JSON: {error}", path.display()))?;
        process_file(&mut file, &module_path, &mut builder, &mut versions)
            .map_err(|error| format!("{}: {error}", path.display()))?;
    }

    let (binary, offsets) = builder.encode(versions.len())?;
    let generated = generate_code(&builder.strings, &versions, offsets)?;
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    write_bytes_if_changed(&out_dir.join("sdk_mappings.bin"), &binary)?;
    write_string_if_changed(&out_dir.join("sdk_mappings.rs"), &generated)?;
    Ok(())
}

fn data_paths(data_dir: &Path) -> BuildResult<Vec<(String, PathBuf)>> {
    let allow_empty = env::var_os("CARGO_FEATURE_GENERATOR").is_some();
    if !data_dir.is_dir() {
        if allow_empty {
            return Ok(Vec::new());
        }
        return Err(format!("SDK data directory is missing: {}", data_dir.display()).into());
    }

    let mut paths = Vec::new();
    for entry in fs::read_dir(data_dir)? {
        let path = entry?.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let service = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| format!("SDK data filename is not valid UTF-8: {}", path.display()))?;
        paths.push((
            format!("github.com/aws/aws-sdk-go-v2/service/{service}"),
            path,
        ));
    }
    if paths.is_empty() && !allow_empty {
        return Err(format!(
            "SDK data directory has no service JSON files: {}",
            data_dir.display()
        )
        .into());
    }
    paths.sort();
    Ok(paths)
}

fn process_file(
    file: &mut data::SdkDataFile,
    expected_module_path: &str,
    builder: &mut IndexBuilder,
    versions: &mut Vec<(String, String, VersionIndex)>,
) -> BuildResult<()> {
    if file.module_path.is_empty() {
        return Err("SDK module path must not be empty".into());
    }
    if file.module_path != expected_module_path {
        return Err(format!(
            "SDK module path {:?} does not match expected {:?}",
            file.module_path, expected_module_path
        )
        .into());
    }
    if file.releases.is_empty() {
        return Err(format!(
            "SDK module {:?} contains no stable releases",
            file.module_path
        )
        .into());
    }

    let mut state = MappingState::new();
    let mut previous_version = None;
    for release in &mut file.releases {
        let version = data::validate_and_apply_release(release, &mut state)
            .map_err(|error| -> Box<dyn Error> { error.into() })?;
        if previous_version
            .as_ref()
            .is_some_and(|previous| previous >= &version)
        {
            return Err(format!(
                "SDK module {:?} releases are not strictly increasing at {}",
                file.module_path, release.module_version
            )
            .into());
        }
        let index = builder.add_snapshot(&state)?;
        versions.push((expected_module_path.to_owned(), version.to_string(), index));
        previous_version = Some(version);
    }
    Ok(())
}

impl IndexBuilder {
    fn add_snapshot(&mut self, state: &MappingState) -> BuildResult<VersionIndex> {
        let mut snapshot = Vec::with_capacity(state.len());
        for ((package, receiver, method), api_methods) in state {
            let package = self.intern_string(package)?;
            let receiver = self.intern_string(receiver)?;
            let method = self.intern_string(method)?;
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
            let row = self.intern_row((package, receiver, method, api_list))?;
            snapshot.push(row);
        }
        if let Some(index) = self.snapshot_ids.get(&snapshot) {
            return Ok(*index);
        }
        let index = VersionIndex {
            start: to_u32(self.snapshot_rows.len(), "snapshot row offset")?,
            len: to_u32(snapshot.len(), "snapshot row count")?,
        };
        self.snapshot_rows.extend_from_slice(&snapshot);
        self.snapshot_ids.insert(snapshot, index);
        Ok(index)
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

    fn encode(&self, version_count: usize) -> BuildResult<(Vec<u8>, BinaryOffsets)> {
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
        push_u32(&mut binary, to_u32(version_count, "version count")?);

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
        for &(package, receiver, method, api_list) in &self.rows {
            push_u16(&mut binary, package);
            push_u16(&mut binary, receiver);
            push_u16(&mut binary, method);
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
    versions: &[(String, String, VersionIndex)],
    offsets: BinaryOffsets,
) -> Result<String, std::fmt::Error> {
    let mut generated = String::new();
    generated.push_str("const STRINGS: &[&str] = &[\n");
    for value in strings {
        writeln!(generated, "    {value:?},")?;
    }

    generated.push_str("];\n\nconst SERVICE_MODULES: &[&str] = &[\n");
    let mut module_ranges = Vec::<(&str, usize, usize)>::new();
    let mut offset = 0;
    while offset < versions.len() {
        let module_path = versions[offset].0.as_str();
        let start = offset;
        while offset < versions.len() && versions[offset].0 == module_path {
            offset += 1;
        }
        writeln!(generated, "    {module_path:?},")?;
        module_ranges.push((module_path, start, offset - start));
    }

    generated.push_str("];\n\nconst MODULE_VERSIONS: &[&str] = &[\n");
    for (_, version, _) in versions {
        writeln!(generated, "    {version:?},")?;
    }
    generated.push_str("];\n\nconst MODULE_VERSION_RANGES: &[(&str, usize, usize)] = &[\n");
    for (module_path, start, len) in module_ranges {
        writeln!(generated, "    ({module_path:?}, {start}, {len}),")?;
    }

    generated.push_str("];\n\nconst VERSION_INDEX: &[VersionIndex] = &[\n");
    for (_, _, index) in versions {
        writeln!(
            generated,
            "    VersionIndex {{ start: {}, len: {} }},",
            format_number(&index.start),
            format_number(&index.len)
        )?;
    }
    generated.push_str("];\n\nconst VERSION_LOOKUP: &[(&str, &str, usize)] = &[\n");
    let mut lookup = versions
        .iter()
        .enumerate()
        .map(|(index, (module_path, version, _))| (module_path.as_str(), version.as_str(), index))
        .collect::<Vec<_>>();
    lookup.sort_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
    for (module_path, version, index) in lookup {
        writeln!(generated, "    ({module_path:?}, {version:?}, {index}),")?;
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

#[cfg(test)]
mod tests {
    use super::*;

    const MODULE_PATH: &str = "github.com/aws/aws-sdk-go-v2/service/example";

    fn release(
        version: &str,
        remove: Vec<data::MappingKey>,
        upsert: Vec<data::CompactMappingRow>,
    ) -> data::SdkDataRelease {
        data::SdkDataRelease {
            module_version: version.into(),
            remove,
            upsert,
        }
    }

    fn synthetic_file() -> data::SdkDataFile {
        data::SdkDataFile {
            module_path: MODULE_PATH.into(),
            releases: vec![
                release(
                    "1.0.0",
                    vec![],
                    vec![(
                        "example".into(),
                        "Client".into(),
                        "First".into(),
                        vec![
                            ("Example".into(), "Zebra".into()),
                            ("Example".into(), "First".into()),
                        ],
                    )],
                ),
                release(
                    "1.1.0",
                    vec![("example".into(), "Client".into(), "First".into())],
                    vec![(
                        "example".into(),
                        "Client".into(),
                        "Second".into(),
                        vec![("Example".into(), "Second".into())],
                    )],
                ),
            ],
        }
    }

    #[test]
    fn processes_service_releases_once_while_validating_module_and_versions() {
        let mut builder = IndexBuilder::default();
        let mut versions = Vec::new();
        let mut file = synthetic_file();

        process_file(&mut file, MODULE_PATH, &mut builder, &mut versions).unwrap();

        assert_eq!(
            versions
                .iter()
                .map(|(_, version, _)| version.as_str())
                .collect::<Vec<_>>(),
            ["1.0.0", "1.1.0"]
        );
        assert_eq!(
            file.releases[0].upsert,
            vec![(
                "example".into(),
                "Client".into(),
                "First".into(),
                vec![
                    ("Example".into(), "First".into()),
                    ("Example".into(), "Zebra".into()),
                ],
            )]
        );
        assert_eq!(builder.snapshot_ids.len(), 2);

        let mut invalid_module = synthetic_file();
        invalid_module.module_path = "github.com/aws/aws-sdk-go-v2/service/other".into();
        assert_eq!(
            process_file(
                &mut invalid_module,
                MODULE_PATH,
                &mut IndexBuilder::default(),
                &mut Vec::new(),
            )
            .unwrap_err()
            .to_string(),
            format!(
                "SDK module path {:?} does not match expected {:?}",
                invalid_module.module_path, MODULE_PATH
            )
        );

        let mut invalid_version = synthetic_file();
        invalid_version.releases[0].module_version = "1.0".into();
        assert_eq!(
            process_file(
                &mut invalid_version,
                MODULE_PATH,
                &mut IndexBuilder::default(),
                &mut Vec::new(),
            )
            .unwrap_err()
            .to_string(),
            "invalid semantic version \"1.0\": unexpected end of input while parsing minor version number"
        );

        let mut invalid_versions = synthetic_file();
        invalid_versions.releases[1].module_version = "1.0.0".into();
        assert_eq!(
            process_file(
                &mut invalid_versions,
                MODULE_PATH,
                &mut IndexBuilder::default(),
                &mut Vec::new(),
            )
            .unwrap_err()
            .to_string(),
            format!(
                "SDK module {:?} releases are not strictly increasing at 1.0.0",
                MODULE_PATH
            )
        );
    }
}
