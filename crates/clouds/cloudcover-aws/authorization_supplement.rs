//! Reviewed operation-to-action links absent from the upstream SAR operation table.
//! This never changes raw SAR documents or substitutes FAS for direct authorization.
use std::collections::BTreeSet;

use super::{BuildResult, OperationRow};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Supplement {
    schema_version: u8,
    mappings: Vec<Mapping>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Mapping {
    service: String,
    operation: String,
    authorized_actions: Vec<Action>,
    sources: Vec<Source>,
    reviewed: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Action {
    service: String,
    name: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    url: String,
    evidence: String,
}

pub(super) fn apply(
    operations: &mut [OperationRow],
    actions: &BTreeSet<(String, String)>,
) -> BuildResult<()> {
    apply_document(operations, actions, include_str!("data/authorization.json"))
}

fn apply_document(
    operations: &mut [OperationRow],
    actions: &BTreeSet<(String, String)>,
    document: &str,
) -> BuildResult<()> {
    let supplement: Supplement = serde_json::from_str(document)?;
    if supplement.schema_version != 1 {
        return Err("unsupported authorization supplement schema".into());
    }
    let mut seen = BTreeSet::new();
    for mapping in supplement.mappings {
        let identity = (&mapping.service, &mapping.operation);
        if !seen.insert((mapping.service.clone(), mapping.operation.clone()))
            || mapping.authorized_actions.is_empty()
            || mapping.sources.is_empty()
            || mapping.reviewed.len() != 10
            || mapping.sources.iter().any(|source| {
                !source.url.starts_with("https://docs.aws.amazon.com/")
                    || source.evidence.trim().is_empty()
            })
        {
            return Err(format!("invalid authorization supplement for {identity:?}").into());
        }
        let mut expected = mapping
            .authorized_actions
            .into_iter()
            .map(|action| (action.service, action.name))
            .collect::<Vec<_>>();
        expected.sort_unstable();
        if expected.windows(2).any(|pair| pair[0] == pair[1])
            || expected.iter().any(|action| !actions.contains(action))
        {
            return Err(format!("missing or duplicate authorized action for {identity:?}").into());
        }
        let row = operations
            .iter_mut()
            .find(|row| (&row.service, &row.name) == identity)
            .ok_or_else(|| format!("missing supplemented operation {identity:?}"))?;
        // An identical upstream fix is harmless. A changed upstream mapping must
        // be reviewed, never silently overwritten or unioned with a stale rule.
        if !row.authorized_actions.is_empty() && row.authorized_actions != expected {
            return Err(
                format!("authorization supplement conflicts with SAR for {identity:?}").into(),
            );
        }
        row.authorized_actions = expected;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<OperationRow> {
        vec![OperationRow {
            service: "s3".into(),
            name: "CreateSession".into(),
            authorized_actions: vec![],
        }]
    }

    fn actions() -> BTreeSet<(String, String)> {
        BTreeSet::from([("s3express".into(), "CreateSession".into())])
    }

    #[test]
    fn applies_documented_bridge_and_accepts_identical_upstream_fix() -> BuildResult<()> {
        let mut rows = rows();
        apply(&mut rows, &actions())?;
        assert_eq!(rows[0].authorized_actions, Vec::from_iter(actions()));
        apply(&mut rows, &actions())?;
        Ok(())
    }

    #[test]
    fn rejects_missing_references_and_conflicting_upstream_authorization() {
        assert!(apply(&mut [], &actions()).is_err());
        assert!(apply(&mut rows(), &BTreeSet::new()).is_err());
        let mut rows = rows();
        rows[0].authorized_actions = vec![("s3".into(), "GetObject".into())];
        assert!(apply(&mut rows, &actions()).is_err());
    }

    #[test]
    fn rejects_invalid_schema_evidence_and_duplicate_mappings() -> BuildResult<()> {
        let document: serde_json::Value =
            serde_json::from_str(include_str!("data/authorization.json"))?;
        for field in ["sources", "authorized_actions"] {
            let mut invalid = document.clone();
            invalid["mappings"][0][field] = serde_json::json!([]);
            assert!(apply_document(&mut rows(), &actions(), &invalid.to_string()).is_err());
        }
        let mut invalid = document.clone();
        invalid["schema_version"] = serde_json::json!(2);
        assert!(apply_document(&mut rows(), &actions(), &invalid.to_string()).is_err());
        let mut invalid = document.clone();
        invalid["mappings"] = serde_json::json!([document["mappings"][0], document["mappings"][0]]);
        assert!(apply_document(&mut rows(), &actions(), &invalid.to_string()).is_err());
        Ok(())
    }
}
