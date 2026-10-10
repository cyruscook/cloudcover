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

## Usage: Terraform

Terraform analysis requires an initialized root module with
`.terraform.lock.hcl` and `.terraform/modules/modules.json`:

```sh
cloudcover policy --language terraform ./path/to/root-module
```

CloudCover analyzes the root module and every module recorded in the
initialized Terraform module manifest, including remote modules. The command
writes an AWS IAM policy as JSON to standard output by default.

Permission mappings cover possible API and lifecycle paths, including S3
multipart uploads and KMS operations performed by S3 and Secrets Manager on
the caller's behalf. KMS requirements depend on encryption settings, key
changes, and checksum options. The generated policy includes these possible
requirements because analysis does not evaluate request arguments or live
bucket/key configuration. Review and restrict the generated ARN templates
to the resources and KMS keys your deployment uses.

Use `--format terraform` to write a Terraform `aws_iam_policy_document` data
source in HCL:

```sh
cloudcover policy --format terraform --language go ./path/to/project
```

## Usage: Go

Run `cloudcover` with the path to a Go project:

```sh
cloudcover policy --language go ./path/to/project
```

## Usage: JavaScript/TypeScript

Run `cloudcover` with a JavaScript or TypeScript project that has a local
`typescript` package and installed AWS SDK v3 client packages:

```sh
cloudcover policy --language javascript ./path/to/project
cloudcover policy --language typescript ./path/to/project
```

JavaScript and TypeScript analysis requires Node.js on `PATH`.
When present, `tsconfig.json` or `jsconfig.json` sets the project entrypoints.
