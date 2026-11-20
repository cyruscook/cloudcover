mod authorization;
mod conditional;
pub mod fas;
mod generated;
mod model;
mod policy;
mod provider;

pub use authorization::AuthorizationCoverage;
pub use conditional::{ConditionalPermission, ReviewedPolicy};
pub use provider::{AwsError, AwsProvider};

pub use model::{IamContextKey, IamPermission, IamResource};

/// Returns IAM permissions and their Service Authorization Reference metadata.
#[must_use]
pub fn iam_permission_catalog() -> &'static [IamPermission] {
    generated::IAM_PERMISSIONS
}
