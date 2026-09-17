# CloudCover

A project to process data on API methods, associated permissions, and corresponding SDK methods across cloud providers.

## CLI usage

Run `cloudcover` with the path to a Go project:

```sh
cargo run -p cloudcover-cli -- policy --language go ./path/to/project
```

Terraform analysis requires an initialized root module with
`.terraform.lock.hcl` and `.terraform/modules/modules.json`:

```sh
cargo run -p cloudcover-cli -- policy --language terraform ./path/to/root-module
```

CloudCover analyzes the root module and every module recorded in the
initialized Terraform module manifest, including remote modules. The command
writes an AWS IAM policy as JSON to standard output.
