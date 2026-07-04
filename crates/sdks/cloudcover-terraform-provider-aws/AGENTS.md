# cloudcover-terraform-provider-aws

This crate provides mappings from `terraform-provider-aws` entrypoints to AWS API operations.

Entrypoint model:
- kind: Terraform provider entrypoint category such as `resource`, `data_source`, `list_resource`, `ephemeral_resource`, or `action`
- type name: Terraform type name such as `aws_s3_bucket`
- action: lifecycle action such as `create`, `read`, `update`, `delete`, `list`, `open`, `renew`, `close`, or `invoke`

Structure:
- `build.rs`: shared cache management, provider checkout refresh, aws-sdk-go-v2 map handoff, generator execution, and emitted Rust static data.
- `generator/`: Go analyzer that reads provider service-package metadata, resolves handlers, walks reachable code, and reuses `cloudcover-aws-sdk-go-v2` method mappings.
- `src/lib.rs`: public generated mapping types and tests.

Scope:
- This is an SDK-layer integration for Terraform AWS provider internals.
- It does not analyze user Terraform source files.
