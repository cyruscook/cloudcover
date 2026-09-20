# cloudcover-aws-sdk-go-v2

This crate exposes generated static mappings from `aws-sdk-go-v2` Go method references to AWS API operations.

Structure:
- `build.rs`: manages shared build cache, SDK checkout refresh, generator execution, and emitted Rust static data.
- `generator/`: Go analyzer using `golang.org/x/tools/go/packages`, SSA, and static callgraph analysis over `aws-sdk-go-v2` source.
- `src/lib.rs`: public generated mapping types and tests.

Role in the workspace:
- Supplies Go SDK method-to-API mappings to `cloudcover-aws`.
- Supplies the AWS SDK operation map reused by `cloudcover-terraform-provider-aws`.

## Generate mapping data

Install `git` and `go` before you run the generator. The generator writes checked-in service files under `data/`.

From the repository root, generate one latest stable release for each service:

```sh
cargo run -p cloudcover-aws-sdk-go-v2 --features generator \
  --bin generate-aws-sdk-go-v2-data -- --latest
```

Generate every stable service release with `--all`, or generate selected releases with one or more `--tag` arguments:

```sh
cargo run -p cloudcover-aws-sdk-go-v2 --features generator \
  --bin generate-aws-sdk-go-v2-data -- --tag service/s3/v1.104.0
cargo run -p cloudcover-aws-sdk-go-v2 --features generator \
  --bin generate-aws-sdk-go-v2-data -- --all
```

The generator uses Go module downloads by default. Pass `--repository-dir PATH` to use a local `aws-sdk-go-v2` checkout. Pass `--force` to regenerate existing releases. Use `--output-dir PATH` or `--work-dir PATH` to change the default data or cache paths.
