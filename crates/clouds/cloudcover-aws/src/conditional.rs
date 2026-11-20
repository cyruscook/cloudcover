//! Exact reviewed direct links whose applicability requires request knowledge.
//! Raw operation links and the IAM metadata catalog remain evidence, not grants.
use cloudcover_core::ApiMethod;

use crate::model::AwsAction;

/// A direct authorization link requiring review of request-specific behavior.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConditionalPermission {
    pub service: &'static str,
    pub operation: &'static str,
    pub permission: &'static str,
    pub applicability: &'static str,
    pub source: &'static str,
    pub reviewed: &'static str,
    pub passed_to_service: Option<&'static str>,
    /// A required manual policy step; this link is never rendered automatically.
    pub manual_step: Option<&'static str>,
}

/// Policy output together with direct links excluded or explicitly opted into.
/// Consumers must present these diagnostics and review resource placeholders.
#[derive(Debug)]
#[must_use]
pub struct ReviewedPolicy<T> {
    pub policy: T,
    pub conditional_permissions: Vec<ConditionalPermission>,
}

pub(crate) fn classify(method: &ApiMethod, action: &AwsAction) -> Option<ConditionalPermission> {
    let (applicability, source, passed_to_service) = match (
        method.service(),
        method.name(),
        action.service,
        action.name,
    ) {
        (
            "appstream",
            "CreateFleet" | "CreateImageBuilder" | "CreateStack" | "CreateUpdatedImage",
            "appstream",
            "TagResource",
        ) => (
            "Required when the request supplies tags; ordinary untagged requests do not need this grant.",
            "https://docs.aws.amazon.com/service-authorization/latest/reference/list_appstream.html",
            None,
        ),
        ("appstream", "CreateFleet" | "UpdateFleet" | "CreateImageBuilder", "iam", "PassRole") => (
            "Required when passing a role to AppStream; review the role ARN and request parameters.",
            "https://docs.aws.amazon.com/service-authorization/latest/reference/list_appstream.html",
            Some("appstream.amazonaws.com"),
        ),
        ("dynamodb", "Scan", "dynamodb", "ReadDataForReplication") => (
            "Replication-specific authorization for multi-account global tables, not ordinary Scan; review the replication service principal and table resource policy.",
            "https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/globaltables_MA_security.html",
            None,
        ),
        (
            "lambda",
            "CreateFunction" | "UpdateFunctionConfiguration",
            "lambda",
            "AssociateRuntimeAlias" | "AssociateRuntimeVersion",
        ) => (
            "Applies to customer-managed runtimes; review alias versus version, runtime ARN and cross-account resource-policy authorization.",
            "https://docs.aws.amazon.com/lambda/latest/dg/cmr-permissions.html",
            None,
        ),
        ("secretsmanager", "CreateSecret", "secretsmanager", "PutResourcePolicy") => (
            "Required when creating a cross-account replica with its managed resource policy, not an ordinary secret.",
            "https://docs.aws.amazon.com/secretsmanager/latest/userguide/auth-and-access_cross-account-replication.html",
            None,
        ),
        _ => return None,
    };
    Some(ConditionalPermission {
        service: match method.service() {
            "appstream" => "appstream",
            "dynamodb" => "dynamodb",
            "lambda" => "lambda",
            "secretsmanager" => "secretsmanager",
            _ => return None,
        },
        // All accepted operation names are static generated catalog identities.
        operation: crate::policy::find_operation(method).ok()?.name,
        permission: action.permission,
        applicability,
        source,
        reviewed: "2026-11-20",
        passed_to_service,
        manual_step: passed_to_service.map(|_| "Add iam:PassRole manually only for the concrete, validated requested role ARN, with StringEquals iam:PassedToService = appstream.amazonaws.com. CloudCover does not infer the role ARN and never renders this grant."),
    })
}
