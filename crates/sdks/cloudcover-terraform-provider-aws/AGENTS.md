# cloudcover-terraform-provider-aws

This crate provides mappings from `terraform-provider-aws` entrypoints to AWS API operations.

Entrypoint model:
- kind: Terraform provider entrypoint category such as `resource`, `data_source`, `list_resource`, `ephemeral_resource`, or `action`
- type name: Terraform type name such as `aws_s3_bucket`
- action: lifecycle action such as `create`, `read`, `update`, `delete`, `list`, `open`, `renew`, `close`, or `invoke`

Structure:
- `build.rs`: shared cache management, provider checkout refresh, AWS SDK Go v2 map handoff, generator execution, and emitted Rust static data.
- `generator/`: Go analyzer that reads provider service-package metadata, resolves handlers, walks reachable code, and reuses `cloudcover-aws-sdk-go-v2` method mappings.
- `src/lib.rs`: public generated mapping types and tests.

Scope:
- This is an SDK-layer integration for Terraform AWS provider internals.
- It does not analyze user Terraform source files.

## Generate permission data

Install `git` and `go` before you run the generator. The generator discovers provider tags and writes checked-in files under `data/`.

From the repository root, generate one provider version:

```sh
cargo run -p cloudcover-terraform-provider-aws --features generator \
  --bin generate-terraform-provider-aws-data -- --version 6.64.0
```

Generate every stable provider version, or limit `--all` to an inclusive version range:

```sh
cargo run -p cloudcover-terraform-provider-aws --features generator \
  --bin generate-terraform-provider-aws-data -- --all
cargo run -p cloudcover-terraform-provider-aws --features generator \
  --bin generate-terraform-provider-aws-data -- --all --from 6.64.0 --to 6.65.0
```

Pass `--force` to regenerate selected existing versions. Pass `--refresh-cache` with `--force` to recompute cached provider analyses. Use `--output-dir PATH` or `--work-dir PATH` to change the default data or cache paths.
