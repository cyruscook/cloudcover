use cloudcover_core::ApiMethod;

use crate::{AwsError, generated};

/// A reviewed lifecycle or manual authorization diagnostic.
/// Acknowledgment never means that caller IAM permissions are unnecessary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct OperationDisposition {
    pub service: &'static str,
    pub operation: &'static str,
    /// `retired`, `deprecated_sdk_api`, or `manual_review`.
    pub kind: &'static str,
    pub message: &'static str,
    pub source: &'static str,
}

impl OperationDisposition {
    /// Whether this diagnostic blocks policy output for the compiled snapshot.
    /// Retained, exactly reviewed Network Firewall mappings remain usable with
    /// deprecation warnings. An absent raw mapping is never revived by a name
    /// match or a generated fallback operation.
    #[must_use]
    pub fn blocks_policy(&self) -> bool {
        self.kind != "deprecated_sdk_api"
            || !generated::REVIEWED_LEGACY_OPERATIONS.contains(&(self.service, self.operation))
    }
}

impl std::fmt::Display for OperationDisposition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{} ({}): {} Evidence: {}",
            self.service, self.operation, self.kind, self.message, self.source
        )?;
        if self.kind == "deprecated_sdk_api" {
            if self.blocks_policy() {
                f.write_str(" The pinned SAR omits the operation; policy generation is blocked.")?;
            } else {
                f.write_str(" The pinned SAR retains the exact reviewed mapping; historical permissions are preserved with this warning.")?;
            }
        }
        Ok(())
    }
}

pub(crate) fn lookup(method: &ApiMethod) -> Option<&'static OperationDisposition> {
    generated::AUTHORIZATION_DISPOSITIONS
        .iter()
        .find(|d| d.service == method.service() && d.operation == method.name())
}

pub(crate) fn ensure_policy_supported(method: &ApiMethod) -> Result<(), AwsError> {
    if let Some(diagnostic) = lookup(method)
        && diagnostic.blocks_policy()
    {
        return Err(AwsError::UnresolvedAuthorization {
            service: method.service().to_owned(),
            name: method.name().to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn unresolved_reason(service: &str, name: &str) -> String {
    if let Some(diagnostic) = lookup(&ApiMethod::new(service, name))
        && diagnostic.blocks_policy()
    {
        return format!(
            "policy generation blocked by reviewed authorization disposition: {diagnostic}"
        );
    }
    "the catalog does not establish caller IAM permission requirements".to_owned()
}
