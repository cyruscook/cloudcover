# CloudCover

CloudCover takes your source code (Lambdas, CDK, IaC...) and outputs the exact least-privilege AWS IAM policy required.

In paticular, for Terraform IaC, because no plan, apply, or state file is required, you will know the exact IAM policy you need before you've even touched a live AWS account. No more asking your admin to add that one missing action to the permission set.

## Installation

Install the command with [Homebrew](https://brew.sh/) on macOS or Linux:

```sh
brew install cyruscook/tap/cloudcover
```

Alternatively, install with [cargo-binstall](https://github.com/cargo-bins/cargo-binstall#installation):

```sh
cargo binstall cloudcover-cli
```

## CLI usage

For a Go project:

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
writes an AWS IAM policy as JSON to standard output by default.

Use `--format terraform` to write a Terraform `aws_iam_policy_document` data source
in HCL:

```sh
cloudcover policy --format terraform --language go ./path/to/project
```
