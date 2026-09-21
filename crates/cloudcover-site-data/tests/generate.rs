use std::{error::Error, fs, path::Path};

use cloudcover_site_data::generate;
use serde_json::Value;

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, Box<dyn Error>> {
    value
        .get(name)
        .ok_or_else(|| format!("missing JSON field {name:?}").into())
}

fn array_item(value: &Value, index: usize) -> Result<&Value, Box<dyn Error>> {
    value
        .get(index)
        .ok_or_else(|| format!("missing JSON array item {index}").into())
}

fn string_value(value: &Value, context: &str) -> Result<String, Box<dyn Error>> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("{context} is not a string").into())
}

fn usize_value(value: &Value, context: &str) -> Result<usize, Box<dyn Error>> {
    let number = value
        .as_u64()
        .ok_or_else(|| format!("{context} is not an unsigned integer"))?;
    usize::try_from(number).map_err(|_| format!("{context} does not fit usize").into())
}

fn read_json(path: &Path) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

#[test]
#[allow(clippy::too_many_lines)]
fn generated_catalogs_are_consistent() -> Result<(), Box<dyn Error>> {
    let temporary_directory = tempfile::tempdir()?;
    let output = temporary_directory.path().join("data");
    generate(&output)?;

    let api = read_json(&output.join("api.json"))?;
    assert_eq!(field(&api, "schema_version")?.as_u64(), Some(1));
    let iam_values = field(&api, "iam_permissions")?
        .as_array()
        .ok_or("iam_permissions is not an array")?;
    let iam_permissions = iam_values
        .iter()
        .enumerate()
        .map(|(index, value)| string_value(value, &format!("IAM permission {index}")))
        .collect::<Result<Vec<_>, _>>()?;
    assert!(
        iam_permissions
            .windows(2)
            .all(|window| window[0] < window[1])
    );

    let api_values = field(&api, "api_methods")?
        .as_array()
        .ok_or("api_methods is not an array")?;
    let mut api_rows = Vec::with_capacity(api_values.len());
    for (index, value) in api_values.iter().enumerate() {
        let row = value.as_array().ok_or("API method row is not an array")?;
        assert_eq!(row.len(), 3, "API method row {index} has the wrong shape");
        let service = string_value(array_item(value, 0)?, "API service")?;
        let name = string_value(array_item(value, 1)?, "API operation")?;
        let permission_ids = array_item(value, 2)?
            .as_array()
            .ok_or("API permission IDs are not an array")?
            .iter()
            .map(|value| {
                let permission_id = usize_value(value, "API permission ID")?;
                if permission_id >= iam_permissions.len() {
                    return Err(format!("API permission ID {permission_id} is dangling").into());
                }
                Ok(permission_id)
            })
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        assert!(
            permission_ids
                .windows(2)
                .all(|window| window[0] < window[1])
        );
        api_rows.push((service, name, permission_ids));
    }
    assert!(api_rows.windows(2).all(|window| {
        (window[0].0.as_str(), window[0].1.as_str()) < (window[1].0.as_str(), window[1].1.as_str())
    }));

    let api_create_bucket_id = api_rows
        .iter()
        .position(|row| row.0 == "s3" && row.1 == "CreateBucket")
        .ok_or("missing s3:CreateBucket API method")?;
    let iam_create_bucket_id = iam_permissions
        .iter()
        .position(|permission| permission == "s3:CreateBucket")
        .ok_or("missing s3:CreateBucket IAM permission")?;
    assert!(
        api_rows[api_create_bucket_id]
            .2
            .contains(&iam_create_bucket_id)
    );

    let terraform_index = read_json(&output.join("terraform/index.json"))?;
    assert_eq!(field(&terraform_index, "schema_version")?.as_u64(), Some(1));
    let versions = field(&terraform_index, "versions")?
        .as_array()
        .ok_or("Terraform versions is not an array")?
        .iter()
        .enumerate()
        .map(|(index, value)| string_value(value, &format!("Terraform version {index}")))
        .collect::<Result<Vec<_>, _>>()?;
    assert!(!versions.is_empty());
    assert!(versions.windows(2).all(|window| window[0] != window[1]));
    let latest = string_value(field(&terraform_index, "latest")?, "Terraform latest")?;
    assert_eq!(versions.last(), Some(&latest));

    let terraform_rows_document = read_json(&output.join("terraform/rows.json"))?;
    assert_eq!(
        field(&terraform_rows_document, "schema_version")?.as_u64(),
        Some(1)
    );
    let row_values = field(&terraform_rows_document, "rows")?
        .as_array()
        .ok_or("Terraform rows is not an array")?;
    let mut terraform_rows = Vec::with_capacity(row_values.len());
    for value in row_values {
        let row = value.as_array().ok_or("Terraform row is not an array")?;
        assert_eq!(row.len(), 3, "Terraform row has the wrong shape");
        let resource = string_value(array_item(value, 0)?, "Terraform resource")?;
        let lifecycle = string_value(array_item(value, 1)?, "Terraform lifecycle")?;
        let api_ids = array_item(value, 2)?
            .as_array()
            .ok_or("Terraform API IDs are not an array")?
            .iter()
            .map(|value| {
                let api_id = usize_value(value, "Terraform API ID")?;
                if api_id >= api_rows.len() {
                    return Err(format!("Terraform API ID {api_id} is dangling").into());
                }
                Ok(api_id)
            })
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        assert!(api_ids.windows(2).all(|window| window[0] < window[1]));
        terraform_rows.push((resource, lifecycle, api_ids));
    }
    assert!(
        terraform_rows
            .windows(2)
            .all(|window| window[0] < window[1])
    );

    for version in &versions {
        let snapshot = read_json(&output.join(format!("terraform/versions/{version}.json")))?;
        assert_eq!(field(&snapshot, "schema_version")?.as_u64(), Some(1));
        assert_eq!(
            string_value(field(&snapshot, "version")?, "snapshot version")?,
            *version
        );
        let row_ids = field(&snapshot, "row_ids")?
            .as_array()
            .ok_or("snapshot row IDs are not an array")?
            .iter()
            .map(|value| {
                let row_id = usize_value(value, "snapshot row ID")?;
                if row_id >= terraform_rows.len() {
                    return Err(format!("snapshot row ID {row_id} is dangling").into());
                }
                Ok(row_id)
            })
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        assert!(row_ids.windows(2).all(|window| window[0] < window[1]));
    }

    let snapshot = read_json(&output.join("terraform/versions/6.64.0.json"))?;
    let snapshot_row_ids = field(&snapshot, "row_ids")?
        .as_array()
        .ok_or("6.64.0 row IDs are not an array")?;
    let bucket_create = snapshot_row_ids
        .iter()
        .map(|value| usize_value(value, "6.64.0 row ID"))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|row_id| &terraform_rows[row_id])
        .find(|row| row.0 == "aws_s3_bucket" && row.1 == "create")
        .ok_or("missing aws_s3_bucket create row in 6.64.0")?;
    assert!(bucket_create.2.contains(&api_create_bucket_id));
    assert!(
        api_rows[api_create_bucket_id]
            .2
            .contains(&iam_create_bucket_id)
    );

    Ok(())
}
