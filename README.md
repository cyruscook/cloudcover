# CloudCover

A project to process data on API methods, associated permissions, and corresponding SDK methods across cloud providers.

## CLI usage

Install the command with [cargo-binstall](https://github.com/cargo-bins/cargo-binstall#installation):

```sh
cargo binstall cloudcover-cli
```

Run `cloudcover` with the path to a Go project:

```sh
cloudcover policy --language go ./path/to/project
```

Terraform analysis requires an initialized root module with
`.terraform.lock.hcl` and `.terraform/modules/modules.json`:

```sh
cloudcover policy --language terraform ./path/to/root-module
```

CloudCover analyzes the root module and every module recorded in the
initialized Terraform module manifest, including remote modules. The command
writes an AWS IAM policy as JSON to standard output by default. Use
`--format terraform` to write a Terraform `aws_iam_policy_document` data source
in HCL:

```sh
cloudcover policy --format terraform --language go ./path/to/project
```
