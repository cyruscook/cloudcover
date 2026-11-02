use crate::model::AwsOperation;

/// Coverage of the operation's direct caller IAM permission requirements.
///
/// Token, trust-policy and resource authorization can still apply when caller
/// IAM permissions are not required. FAS coverage is tracked separately.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizationCoverage {
    /// The catalog supplies one or more authorized IAM actions.
    Mapped,
    /// AWS documents that no caller IAM permission is required.
    NotRequired,
    /// The catalog does not establish the operation's IAM requirements.
    Unresolved,
}

impl AwsOperation {
    pub(crate) fn authorization_coverage(&self) -> AuthorizationCoverage {
        if no_caller_iam_permissions(self.service, self.name) {
            AuthorizationCoverage::NotRequired
        } else if self.authorized_actions.is_empty() {
            // Neither an omitted AuthorizedActions field nor an empty array is
            // affirmative evidence that an operation requires no permissions.
            AuthorizationCoverage::Unresolved
        } else {
            AuthorizationCoverage::Mapped
        }
    }
}

fn no_caller_iam_permissions(service: &str, name: &str) -> bool {
    matches!(
        (service, name),
        // AWS documents that this call succeeds even if its action is denied.
        // https://docs.aws.amazon.com/STS/latest/APIReference/API_GetCallerIdentity.html
        // These federation calls authenticate an identity-provider assertion or
        // token without caller AWS credentials. The role must still trust the IdP.
        // https://docs.aws.amazon.com/STS/latest/APIReference/API_AssumeRoleWithSAML.html
        // https://docs.aws.amazon.com/STS/latest/APIReference/API_AssumeRoleWithWebIdentity.html
        ("sts", "GetCallerIdentity" | "AssumeRoleWithSAML" | "AssumeRoleWithWebIdentity")
        // AWS specifies user access tokens (including their required scope), or
        // app-client credentials, rather than IAM credentials for these calls.
        // https://docs.aws.amazon.com/cognito-user-identity-pools/latest/APIReference/API_CompleteWebAuthnRegistration.html
        // https://docs.aws.amazon.com/cognito-user-identity-pools/latest/APIReference/API_DeleteWebAuthnCredential.html
        // https://docs.aws.amazon.com/cognito-user-identity-pools/latest/APIReference/API_GetClientToken.html
        // https://docs.aws.amazon.com/cognito-user-identity-pools/latest/APIReference/API_GetUserAuthFactors.html
        // https://docs.aws.amazon.com/cognito-user-identity-pools/latest/APIReference/API_ListWebAuthnCredentials.html
        // https://docs.aws.amazon.com/cognito-user-identity-pools/latest/APIReference/API_StartWebAuthnRegistration.html
        | (
            "cognito-idp",
            "CompleteWebAuthnRegistration"
            | "DeleteWebAuthnCredential"
            | "GetClientToken"
            | "GetUserAuthFactors"
            | "ListWebAuthnCredentials"
            | "StartWebAuthnRegistration",
        )
        // The portal API requires the OIDC access token in its SSO bearer header.
        // https://docs.aws.amazon.com/singlesignon/latest/PortalAPIReference/API_GetRoleCredentials.html
        // https://docs.aws.amazon.com/singlesignon/latest/PortalAPIReference/API_ListAccountRoles.html
        // https://docs.aws.amazon.com/singlesignon/latest/PortalAPIReference/API_ListAccounts.html
        // https://docs.aws.amazon.com/singlesignon/latest/PortalAPIReference/API_Logout.html
        | ("sso", "GetRoleCredentials" | "ListAccountRoles" | "ListAccounts" | "Logout")
        // RegisterClient registers a public OAuth client; device authorization
        // uses that client's ID and secret, not caller AWS credentials.
        // https://docs.aws.amazon.com/singlesignon/latest/OIDCAPIReference/API_RegisterClient.html
        // https://docs.aws.amazon.com/singlesignon/latest/OIDCAPIReference/API_StartDeviceAuthorization.html
        | ("sso-oauth", "RegisterClient" | "StartDeviceAuthorization")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AwsApiMethodRef;

    #[test]
    fn actionless_operations_require_affirmative_service_specific_evidence() {
        for (service, name) in [
            ("ec2", "GetCapacityManagerSupportabilityScores"),
            ("neptune-db", "CancelIndexRebuild"),
            ("neptune-db", "GetIndexRebuildStatus"),
            ("neptune-db", "GetStorageInfo"),
            ("neptune-db", "ListIndexRebuilds"),
            ("neptune-db", "StartIndexRebuild"),
            ("opensearch", "FailoverReplication"),
            ("s3", "CreateSession"),
            ("other", "GetCallerIdentity"),
            ("sts", "UnknownOperation"),
        ] {
            let operation = AwsOperation {
                service,
                name,
                authorized_actions: &[],
            };
            assert_eq!(
                operation.authorization_coverage(),
                AuthorizationCoverage::Unresolved,
                "{service}:{name}"
            );
        }
    }

    #[test]
    fn mapped_actions_and_documented_permissionless_calls_remain_distinct() {
        let operation = AwsOperation {
            service: "s3",
            name: "GetObject",
            authorized_actions: &[AwsApiMethodRef {
                service: "s3",
                name: "GetObject",
            }],
        };
        assert_eq!(
            operation.authorization_coverage(),
            AuthorizationCoverage::Mapped
        );
        for name in [
            "GetCallerIdentity",
            "AssumeRoleWithSAML",
            "AssumeRoleWithWebIdentity",
        ] {
            let operation = AwsOperation {
                service: "sts",
                name,
                authorized_actions: &[],
            };
            assert_eq!(
                operation.authorization_coverage(),
                AuthorizationCoverage::NotRequired
            );
        }
    }
}
