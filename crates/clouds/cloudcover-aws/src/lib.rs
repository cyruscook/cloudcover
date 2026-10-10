mod generated;
mod model;
mod policy;
mod provider;

pub use provider::{AwsError, AwsProvider};

pub use model::{IamContextKey, IamPermission, IamResource};

/// Returns IAM permissions and their Service Authorization Reference metadata.
#[must_use]
pub fn iam_permission_catalog() -> &'static [IamPermission] {
    generated::IAM_PERMISSIONS
}
