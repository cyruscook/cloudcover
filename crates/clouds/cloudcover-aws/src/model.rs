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
