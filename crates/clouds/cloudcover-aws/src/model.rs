#[derive(Clone, Copy, Debug)]
pub(crate) struct AwsAction {
    pub(crate) service: &'static str,
    pub(crate) name: &'static str,
    pub(crate) permission: &'static str,
    pub(crate) resource_types: &'static [&'static str],
    pub(crate) resource_templates: &'static [&'static str],
    pub(crate) has_complete_resource_templates: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AwsApiMethodRef {
    pub(crate) service: &'static str,
    pub(crate) name: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AwsOperation {
    pub(crate) service: &'static str,
    pub(crate) name: &'static str,
    pub(crate) authorized_actions: &'static [AwsApiMethodRef],
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AwsSdkMethodMapping {
    pub(crate) sdk_package: &'static str,
    pub(crate) sdk_name: &'static str,
    pub(crate) sdk_method: &'static str,
    pub(crate) api_service: &'static str,
    pub(crate) api_name: &'static str,
}

/// Service Authorization Reference metadata for an IAM permission.
#[derive(Clone, Copy, Debug)]
pub struct IamPermission {
    pub permission: &'static str,
    pub resources: &'static [IamResource],
    pub context_keys: &'static [IamContextKey],
}

/// A resource type applicable to a permission and its ARN patterns.
#[derive(Clone, Copy, Debug)]
pub struct IamResource {
    pub name: &'static str,
    pub arn_formats: &'static [&'static str],
}

/// A context key with its SAR value types and applicability.
#[derive(Clone, Copy, Debug)]
pub struct IamContextKey {
    pub name: &'static str,
    pub types: &'static [&'static str],
    pub permission: bool,
    pub permission_resources: &'static [&'static str],
    pub resource_types: &'static [&'static str],
}
