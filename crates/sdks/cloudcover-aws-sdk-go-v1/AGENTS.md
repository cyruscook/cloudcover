# cloudcover-aws-sdk-go-v1

This crate exposes generated static mappings from `aws-sdk-go` method references to AWS API operations.

Structure:
- `build.rs`: validates the checked-in mapping data and emits the Rust lookup index.
- `generator/`: Go analyzer that reads the `aws-sdk-go` source history.
- `src/lib.rs`: public generated mapping types and tests.
- `data/aws-sdk-go.json`: checked-in mapping data for every stable `v1.*` release.

Role in the workspace:
- Supplies Go SDK v1 method-to-API mappings to `cloudcover-aws`.
- Supplies legacy SDK mappings to `cloudcover-terraform-provider-aws`.

## Generate mapping data

Install `git` and `go` before you run the generator. The generator reads a bare `aws-sdk-go` repository and replaces `data/aws-sdk-go.json`.

From the repository root, create the default repository and run:

```sh
mkdir -p target/aws-sdk-go-v1-data
git clone --bare https://github.com/aws/aws-sdk-go.git \
  target/aws-sdk-go-v1-data/repository.git
cargo run -p cloudcover-aws-sdk-go-v1 --features generator \
  --bin generate-aws-sdk-go-v1-data -- --force
```

The generator scans every stable `v1.*` tag. Refresh the bare repository with `git -C target/aws-sdk-go-v1-data/repository.git fetch --tags` before a later run. Pass `--repository PATH` and `--output PATH` to use different paths. The `--force` flag is required when the output file already exists.
