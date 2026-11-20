use cloudcover_core::ApiMethod;

use crate::{AwsError, generated};

/// A reviewed lifecycle or manual authorization issue that blocks policy output.
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

impl std::fmt::Display for OperationDisposition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{} ({}): {} Evidence: {}",
            self.service, self.operation, self.kind, self.message, self.source
        )
    }
}

pub(crate) fn lookup(method: &ApiMethod) -> Option<&'static OperationDisposition> {
    generated::AUTHORIZATION_DISPOSITIONS
        .iter()
        .find(|d| d.service == method.service() && d.operation == method.name())
}

pub(crate) fn ensure_policy_supported(method: &ApiMethod) -> Result<(), AwsError> {
    if let Some(diagnostic) = lookup(method) {
        return Err(AwsError::AuthorizationDisposition { diagnostic });
    }
    Ok(())
}
