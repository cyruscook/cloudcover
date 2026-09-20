use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::Path,
};

#[path = "src/data.rs"]
mod data;

const MODULE_PATH: &str = "github.com/aws/aws-sdk-go";

fn main() {
    if let Err(error) = run() {
        eprintln!("failed to build AWS SDK Go v1 mappings: {error}");
        std::process::exit(1);
    }
}

#[allow(clippy::too_many_lines)]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let input = Path::new("data/aws-sdk-go.json");
    println!("cargo:rerun-if-changed={}", input.display());
    let contents = fs::read_to_string(input)?;
    let mut file: data::SdkDataFile = serde_json::from_str(&contents)?;
    let mut state = data::MappingState::new();
    let mut snapshots = Vec::with_capacity(file.releases.len());
    for release in &mut file.releases {
        let version = data::validate_and_apply_release(release, &mut state)?;
        snapshots.push((version.to_string(), state.clone()));
    }
    if file.module_path != MODULE_PATH {
        return Err(format!("unexpected module path {:?}", file.module_path).into());
    }

    let mut identities = BTreeSet::new();
    for (_, snapshot) in &snapshots {
        for (key, api_methods) in snapshot {
            identities.insert((
                key.clone(),
                api_methods
                    .iter()
                    .map(|method| (method.service.clone(), method.name.clone()))
                    .collect::<Vec<_>>(),
            ));
        }
    }
    if identities.len() > usize::from(u16::MAX) + 1 {
        return Err("too many mapping rows for u16 row indexes".into());
    }

    let mut strings = BTreeSet::new();
    for (version, snapshot) in &snapshots {
        strings.insert(version.clone());
        for (key, api_methods) in snapshot {
            strings.extend([key.0.clone(), key.1.clone(), key.2.clone()]);
            for method in api_methods {
                strings.insert(method.service.clone());
                strings.insert(method.name.clone());
            }
        }
    }
    if strings.len() > usize::from(u16::MAX) + 1 {
        return Err("too many strings for u16 string indexes".into());
    }
    let strings = strings.into_iter().collect::<Vec<_>>();
    let string_index = strings
        .iter()
        .enumerate()
        .map(|(index, value)| Ok((value.clone(), u16::try_from(index)?)))
        .collect::<Result<BTreeMap<_, _>, std::num::TryFromIntError>>()?;

    let identities = identities.into_iter().collect::<Vec<_>>();
    let row_index = identities
        .iter()
        .enumerate()
        .map(|(index, identity)| Ok((identity.clone(), u16::try_from(index)?)))
        .collect::<Result<BTreeMap<_, _>, std::num::TryFromIntError>>()?;

    let mut api_lists = BTreeSet::<Vec<(u16, u16)>>::new();
    let mut snapshot_rows = Vec::with_capacity(snapshots.len());
    for (_, snapshot) in &snapshots {
        let mut rows = Vec::with_capacity(snapshot.len());
        for (key, api_methods) in snapshot {
            let compact_methods = api_methods
                .iter()
                .map(|method| {
                    Ok((
                        *string_index
                            .get(&method.service)
                            .ok_or("missing API service string")?,
                        *string_index
                            .get(&method.name)
                            .ok_or("missing API method string")?,
                    ))
                })
                .collect::<Result<Vec<_>, &str>>()?;
            api_lists.insert(compact_methods.clone());
            rows.push((
                row_index
                    .get(&(
                        key.clone(),
                        api_methods
                            .iter()
                            .map(|method| (method.service.clone(), method.name.clone()))
                            .collect(),
                    ))
                    .copied()
                    .ok_or("missing mapping row")?,
                compact_methods,
            ));
        }
        rows.sort_unstable_by_key(|(row_id, _)| *row_id);
        snapshot_rows.push(rows);
    }
    let api_lists = api_lists.into_iter().collect::<Vec<_>>();
    if api_lists.len() > usize::from(u16::MAX) + 1 {
        return Err("too many API lists for u16 indexes".into());
    }
    let api_list_index = api_lists
        .iter()
        .enumerate()
        .map(|(index, methods)| Ok((methods.clone(), u16::try_from(index)?)))
        .collect::<Result<BTreeMap<_, _>, std::num::TryFromIntError>>()?;
    let mut api_methods_bytes = Vec::new();
    let mut api_list_records = Vec::with_capacity(api_lists.len());

    let mut rows_bytes = Vec::with_capacity(identities.len() * 8);
    for ((package, receiver, method), api_methods) in &identities {
        push_u16(
            &mut rows_bytes,
            *string_index.get(package).ok_or("missing package string")?,
        );
        push_u16(
            &mut rows_bytes,
            *string_index
                .get(receiver)
                .ok_or("missing receiver string")?,
        );
        push_u16(
            &mut rows_bytes,
            *string_index.get(method).ok_or("missing method string")?,
        );
        let compact_methods = api_methods
            .iter()
            .map(|(service, name)| {
                Ok((
                    *string_index
                        .get(service)
                        .ok_or("missing API service string")?,
                    *string_index.get(name).ok_or("missing API method string")?,
                ))
            })
            .collect::<Result<Vec<_>, &str>>()?;
        push_u16(
            &mut rows_bytes,
            *api_list_index
                .get(&compact_methods)
                .ok_or("missing API list")?,
        );
    }
    for methods in &api_lists {
        let start = u32::try_from(api_methods_bytes.len() / 4)?;
        for (service, name) in methods {
            push_u16(&mut api_methods_bytes, *service);
            push_u16(&mut api_methods_bytes, *name);
        }
        api_list_records.push((start, u32::try_from(methods.len())?));
    }
    let mut api_lists_bytes = Vec::with_capacity(api_list_records.len() * 8);
    let mut row_ids_bytes = Vec::new();
    let mut version_index = Vec::with_capacity(snapshot_rows.len());
    for (start, len) in api_list_records {
        push_u32(&mut api_lists_bytes, start);
        push_u32(&mut api_lists_bytes, len);
    }
    for rows in snapshot_rows {
        let start = u32::try_from(row_ids_bytes.len() / 2)?;
        for (row_id, _) in rows {
            push_u16(&mut row_ids_bytes, row_id);
        }
        version_index.push((start, u32::try_from(row_ids_bytes.len() / 2)? - start));
    }

    let rows_offset = 0;
    let api_methods_offset = rows_bytes.len();
    let api_lists_offset = api_methods_offset + api_methods_bytes.len();
    let row_ids_offset = api_lists_offset + api_lists_bytes.len();
    let mut binary = rows_bytes;
    binary.extend(api_methods_bytes);
    binary.extend(api_lists_bytes);
    binary.extend(row_ids_bytes);

    let out_dir_value = env::var("OUT_DIR")?;
    let out_dir = Path::new(&out_dir_value);
    fs::write(out_dir.join("sdk_mappings.bin"), binary)?;
    let generated = generate_source(
        &strings,
        &snapshots,
        &version_index,
        rows_offset,
        api_methods_offset,
        api_lists_offset,
        row_ids_offset,
    );
    fs::write(out_dir.join("sdk_mappings.rs"), generated)?;
    Ok(())
}

#[allow(clippy::format_collect, clippy::uninlined_format_args)]
fn generate_source(
    strings: &[String],
    snapshots: &[(String, data::MappingState)],
    version_index: &[(u32, u32)],
    rows_offset: usize,
    api_methods_offset: usize,
    api_lists_offset: usize,
    row_ids_offset: usize,
) -> String {
    let strings = strings
        .iter()
        .map(|value| format!("    {:?},", value))
        .collect::<String>();
    let versions = snapshots
        .iter()
        .map(|(version, _)| format!("    {:?},", version))
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

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend(value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend(value.to_le_bytes());
}
