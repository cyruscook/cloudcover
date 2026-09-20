use std::collections::{BTreeMap, BTreeSet};

use super::data;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct InternedMappingRow {
    key: (u16, u16, u16),
    api_methods: Vec<(u16, u16)>,
}

#[derive(Debug)]
pub(crate) struct EncodedMappings {
    pub(crate) strings: Vec<String>,
    pub(crate) version_string_ids: Vec<u16>,
    pub(crate) version_index: Vec<(u32, u32)>,
    pub(crate) binary: Vec<u8>,
    pub(crate) rows_offset: usize,
    #[allow(dead_code)]
    pub(crate) api_methods_offset: usize,
    pub(crate) api_lists_offset: usize,
    pub(crate) row_ids_offset: usize,
}
#[derive(Debug)]
struct EncodedBinary {
    binary: Vec<u8>,
    version_index: Vec<(u32, u32)>,
    api_methods_offset: usize,
    api_lists_offset: usize,
    row_ids_offset: usize,
}

#[derive(Default)]
pub(crate) struct IndexBuilder {
    strings: StringInterner,
    rows: BTreeMap<InternedMappingRow, u16>,
    version_string_ids: Vec<u16>,
    snapshot_row_ids: Vec<Vec<u16>>,
}

#[derive(Default)]
struct StringInterner {
    indexes: BTreeMap<String, u16>,
}

impl StringInterner {
    fn intern(&mut self, value: &str) -> Result<u16, String> {
        if let Some(index) = self.indexes.get(value) {
            return Ok(*index);
        }
        if self.indexes.len() > usize::from(u16::MAX) {
            return Err("too many strings for u16 string indexes".into());
        }
        let index = u16::try_from(self.indexes.len())
            .map_err(|_| "too many strings for u16 string indexes")?;
        self.indexes.insert(value.to_owned(), index);
        Ok(index)
    }

    fn into_sorted(self) -> Result<(Vec<String>, Vec<u16>), String> {
        let mut remap = vec![0; self.indexes.len()];
        for (new_index, old_index) in self.indexes.values().enumerate() {
            remap[usize::from(*old_index)] =
                u16::try_from(new_index).map_err(|_| "too many strings for u16 string indexes")?;
        }
        Ok((self.indexes.into_keys().collect(), remap))
    }
}

impl IndexBuilder {
    pub(crate) fn from_releases(releases: &mut [data::SdkDataRelease]) -> Result<Self, String> {
        let mut builder = Self::default();
        let mut state = data::MappingState::new();
        for release in releases {
            let version = data::validate_and_apply_release(release, &mut state)?;
            builder
                .version_string_ids
                .push(builder.strings.intern(&version.to_string())?);
            builder.capture_state(&state)?;
        }
        Ok(builder)
    }

    fn capture_state(&mut self, state: &data::MappingState) -> Result<(), String> {
        let mut snapshot = Vec::with_capacity(state.len());
        for ((package, receiver, method), api_methods) in state {
            let row = InternedMappingRow {
                key: (
                    self.strings.intern(package)?,
                    self.strings.intern(receiver)?,
                    self.strings.intern(method)?,
                ),
                api_methods: api_methods
                    .iter()
                    .map(|api_method| {
                        Ok((
                            self.strings.intern(&api_method.service)?,
                            self.strings.intern(&api_method.name)?,
                        ))
                    })
                    .collect::<Result<_, String>>()?,
            };
            snapshot.push(self.intern_row(row)?);
        }
        self.snapshot_row_ids.push(snapshot);
        Ok(())
    }

    fn intern_row(&mut self, row: InternedMappingRow) -> Result<u16, String> {
        if let Some(index) = self.rows.get(&row) {
            return Ok(*index);
        }
        if self.rows.len() > usize::from(u16::MAX) {
            return Err("too many mapping rows for u16 row indexes".into());
        }
        let index = u16::try_from(self.rows.len())
            .map_err(|_| "too many mapping rows for u16 row indexes")?;
        self.rows.insert(row, index);
        Ok(index)
    }

    pub(crate) fn encode(self) -> Result<EncodedMappings, String> {
        let (strings, string_remap) = self.strings.into_sorted()?;
        let mut rows = self
            .rows
            .into_iter()
            .map(|(row, old_index)| {
                (
                    old_index,
                    InternedMappingRow {
                        key: (
                            string_remap[usize::from(row.key.0)],
                            string_remap[usize::from(row.key.1)],
                            string_remap[usize::from(row.key.2)],
                        ),
                        api_methods: row
                            .api_methods
                            .into_iter()
                            .map(|(service, name)| {
                                (
                                    string_remap[usize::from(service)],
                                    string_remap[usize::from(name)],
                                )
                            })
                            .collect(),
                    },
                )
            })
            .collect::<Vec<_>>();
        rows.sort_unstable_by(|(_, left), (_, right)| left.cmp(right));
        let mut row_remap = vec![0; rows.len()];
        for (new_index, (old_index, _)) in rows.iter().enumerate() {
            row_remap[usize::from(*old_index)] = u16::try_from(new_index)
                .map_err(|_| "too many mapping rows for u16 row indexes")?;
        }
        let rows = rows.into_iter().map(|(_, row)| row).collect::<Vec<_>>();
        let version_string_ids = self
            .version_string_ids
            .into_iter()
            .map(|index| string_remap[usize::from(index)])
            .collect::<Vec<_>>();
        let snapshot_row_ids = self
            .snapshot_row_ids
            .into_iter()
            .map(|snapshot| {
                let mut snapshot = snapshot
                    .into_iter()
                    .map(|index| row_remap[usize::from(index)])
                    .collect::<Vec<_>>();
                snapshot.sort_unstable();
                snapshot
            })
            .collect::<Vec<_>>();

        let mut api_lists = BTreeSet::<Vec<(u16, u16)>>::new();
        for row in &rows {
            api_lists.insert(row.api_methods.clone());
        }
        if api_lists.len() > usize::from(u16::MAX) {
            return Err("too many API lists for u16 indexes".into());
        }
        let api_lists = api_lists.into_iter().collect::<Vec<_>>();
        let api_list_index = api_lists
            .iter()
            .enumerate()
            .map(|(index, methods)| {
                Ok((
                    methods.clone(),
                    u16::try_from(index).map_err(|_| "too many API lists for u16 indexes")?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;

        let EncodedBinary {
            binary,
            version_index,
            api_methods_offset,
            api_lists_offset,
            row_ids_offset,
        } = encode_binary(&rows, &api_lists, &api_list_index, snapshot_row_ids)?;
        let rows_offset = 0;

        Ok(EncodedMappings {
            strings,
            version_string_ids,
            version_index,
            binary,
            rows_offset,
            api_methods_offset,
            api_lists_offset,
            row_ids_offset,
        })
    }
}

fn encode_binary(
    rows: &[InternedMappingRow],
    api_lists: &[Vec<(u16, u16)>],
    api_list_index: &BTreeMap<Vec<(u16, u16)>, u16>,
    snapshot_row_ids: Vec<Vec<u16>>,
) -> Result<EncodedBinary, String> {
    let mut rows_bytes = Vec::with_capacity(rows.len() * 8);
    for row in rows {
        push_u16(&mut rows_bytes, row.key.0);
        push_u16(&mut rows_bytes, row.key.1);
        push_u16(&mut rows_bytes, row.key.2);
        let api_list = api_list_index
            .get(&row.api_methods)
            .ok_or("interned API list missing")?;
        push_u16(&mut rows_bytes, *api_list);
    }

    let mut api_methods_bytes = Vec::new();
    let mut api_list_records = Vec::with_capacity(api_lists.len());
    for methods in api_lists {
        let start =
            u32::try_from(api_methods_bytes.len() / 4).map_err(|error| error.to_string())?;
        for (service, name) in methods {
            push_u16(&mut api_methods_bytes, *service);
            push_u16(&mut api_methods_bytes, *name);
        }
        api_list_records.push((
            start,
            u32::try_from(methods.len()).map_err(|error| error.to_string())?,
        ));
    }
    let mut api_lists_bytes = Vec::with_capacity(api_list_records.len() * 8);
    for (start, len) in api_list_records {
        push_u32(&mut api_lists_bytes, start);
        push_u32(&mut api_lists_bytes, len);
    }

    let mut row_ids_bytes = Vec::new();
    let mut version_index = Vec::with_capacity(snapshot_row_ids.len());
    for snapshot in snapshot_row_ids {
        let start = u32::try_from(row_ids_bytes.len() / 2).map_err(|error| error.to_string())?;
        for row_id in snapshot {
            push_u16(&mut row_ids_bytes, row_id);
        }
        version_index.push((
            start,
            u32::try_from(row_ids_bytes.len() / 2).map_err(|error| error.to_string())? - start,
        ));
    }

    let api_methods_offset = rows_bytes.len();
    let api_lists_offset = api_methods_offset + api_methods_bytes.len();
    let row_ids_offset = api_lists_offset + api_lists_bytes.len();
    let mut binary = rows_bytes;
    binary.extend(api_methods_bytes);
    binary.extend(api_lists_bytes);
    binary.extend(row_ids_bytes);
    Ok(EncodedBinary {
        binary,
        version_index,
        api_methods_offset,
        api_lists_offset,
        row_ids_offset,
    })
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend(value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend(value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_data() -> data::SdkDataFile {
        data::SdkDataFile {
            module_path: "github.com/aws/aws-sdk-go".into(),
            releases: vec![
                data::SdkDataRelease {
                    module_version: "1.0.0".into(),
                    remove: vec![],
                    upsert: vec![(
                        "pkg".into(),
                        "Client".into(),
                        "First".into(),
                        vec![data::ApiMethodTuple::from(("Svc".into(), "Get".into()))],
                    )],
                },
                data::SdkDataRelease {
                    module_version: "1.1.0".into(),
                    remove: vec![("pkg".into(), "Client".into(), "First".into())],
                    upsert: vec![(
                        "pkg".into(),
                        "Client".into(),
                        "Second".into(),
                        vec![data::ApiMethodTuple::from(("Svc".into(), "Put".into()))],
                    )],
                },
                data::SdkDataRelease {
                    module_version: "1.2.0".into(),
                    remove: vec![],
                    upsert: vec![(
                        "pkg".into(),
                        "Client".into(),
                        "Second".into(),
                        vec![data::ApiMethodTuple::from(("Svc".into(), "Delete".into()))],
                    )],
                },
            ],
        }
    }

    fn read_u16(bytes: &[u8], offset: usize) -> u16 {
        u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
    }

    fn read_u32(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ])
    }

    type SnapshotRow = ((String, String, String), Vec<(String, String)>);

    fn snapshot(encoded: &EncodedMappings, version: &str) -> Option<Vec<SnapshotRow>> {
        let version_position = encoded
            .version_string_ids
            .iter()
            .position(|index| encoded.strings[usize::from(*index)] == version)?;
        let (start, len) = encoded.version_index[version_position];
        let rows = (start..start + len)
            .map(|offset| {
                let offset = usize::try_from(offset).ok()?;
                let row_id = usize::from(read_u16(
                    &encoded.binary,
                    encoded.row_ids_offset + offset * 2,
                ));
                let row_offset = encoded.rows_offset + row_id * 8;
                let key = (0..3)
                    .map(|part| {
                        encoded.strings
                            [usize::from(read_u16(&encoded.binary, row_offset + part * 2))]
                        .clone()
                    })
                    .collect::<Vec<_>>();
                let api_list = usize::from(read_u16(&encoded.binary, row_offset + 6));
                let api_list_offset = encoded.api_lists_offset + api_list * 8;
                let api_start = read_u32(&encoded.binary, api_list_offset);
                let api_len = read_u32(&encoded.binary, api_list_offset + 4);
                let api_methods = (api_start..api_start + api_len)
                    .map(|api_offset| {
                        let api_offset =
                            encoded.api_methods_offset + usize::try_from(api_offset).ok()? * 4;
                        Some((
                            encoded.strings[usize::from(read_u16(&encoded.binary, api_offset))]
                                .clone(),
                            encoded.strings[usize::from(read_u16(&encoded.binary, api_offset + 2))]
                                .clone(),
                        ))
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some((
                    (key[0].clone(), key[1].clone(), key[2].clone()),
                    api_methods,
                ))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(rows)
    }

    #[test]
    fn streams_releases_into_compact_encoded_snapshots() -> Result<(), String> {
        let mut data = synthetic_data();
        let builder = IndexBuilder::from_releases(&mut data.releases)?;
        assert_eq!(builder.snapshot_row_ids, [vec![0], vec![1], vec![2]]);
        let encoded = builder.encode()?;
        assert_eq!(
            encoded
                .version_string_ids
                .iter()
                .map(|index| encoded.strings[usize::from(*index)].as_str())
                .collect::<Vec<_>>(),
            ["1.0.0", "1.1.0", "1.2.0"],
        );
        assert_eq!(encoded.version_index, [(0, 1), (1, 1), (2, 1)]);
        assert_eq!(
            snapshot(&encoded, "1.0.0"),
            Some(vec![(
                ("pkg".into(), "Client".into(), "First".into()),
                vec![("Svc".into(), "Get".into())],
            )]),
        );
        assert_eq!(
            snapshot(&encoded, "1.1.0"),
            Some(vec![(
                ("pkg".into(), "Client".into(), "Second".into()),
                vec![("Svc".into(), "Put".into())],
            )]),
        );
        assert_eq!(
            snapshot(&encoded, "1.2.0"),
            Some(vec![(
                ("pkg".into(), "Client".into(), "Second".into()),
                vec![("Svc".into(), "Delete".into())],
            )]),
        );
        assert_eq!(snapshot(&encoded, "v1.3.0"), None);
        Ok(())
    }
}
