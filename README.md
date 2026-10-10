# CloudCover

CloudCover takes your source code (Lambdas, CDK, IaC...) and outputs an AWS IAM policy baseline for the operations it discovers.

For Terraform IaC, no plan, apply, or state file is required. CloudCover can identify
permissions before deployment; review the generated policy against your resource
configuration and authorization requirements.

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

## Conditional downstream permissions (FAS)

AWS's operation mappings exclude forward access session permissions. CloudCover
supplements them with a [checked-in, primary-source FAS catalog](docs/fas.md).
Covered operations include possible requirements for unknown request/resource
configuration, such as S3 SSE-KMS and Secrets Manager customer-managed keys.
JSON and HCL preserve KMS service conditions and distinct key resource roles.
Replace ARN/Region placeholders and review applicability before deployment.
CLI notes report possible requirements and operations with unknown FAS coverage.
The website exposes applicability and AWS citations; copied action names omit
policy conditions. Unlisted operations are not a guarantee of complete coverage.

## Conditional downstream permissions (FAS)

AWS's operation mappings exclude forward access session permissions. CloudCover
supplements them with a [checked-in, primary-source FAS catalog](docs/fas.md).
Covered operations include possible requirements for unknown request/resource
configuration, such as S3 SSE-KMS and Secrets Manager customer-managed keys.
JSON and HCL preserve KMS service conditions and distinct key resource roles.
Replace ARN/Region placeholders and review applicability before deployment.
CLI notes report possible requirements and operations with unknown FAS coverage.
The website exposes applicability and AWS citations; copied action names omit
policy conditions. Unlisted operations are not a guarantee of complete coverage.

## Conditional downstream permissions (FAS)

AWS's operation mappings exclude forward access session permissions. CloudCover
supplements them with a [checked-in, primary-source FAS catalog](docs/fas.md).
Covered operations include possible requirements for unknown request/resource
configuration, such as S3 SSE-KMS and Secrets Manager customer-managed keys.
JSON and HCL preserve KMS service conditions and distinct key resource roles.
Replace ARN/Region placeholders and review applicability before deployment.
CLI notes report possible requirements and operations with unknown FAS coverage.
The website exposes applicability and AWS citations; copied action names omit
policy conditions. Unlisted operations are not a guarantee of complete coverage.

## Conditional downstream permissions (FAS)

AWS's operation mappings exclude forward access session permissions. CloudCover
supplements them with a [checked-in, primary-source FAS catalog](docs/fas.md).
Covered operations include possible requirements for unknown request/resource
configuration, such as S3 SSE-KMS and Secrets Manager customer-managed keys.
JSON and HCL preserve KMS service conditions and distinct key resource roles.
Replace ARN/Region placeholders and review applicability before deployment.
CLI notes report possible requirements and operations with unknown FAS coverage.
The website exposes applicability and AWS citations; copied action names omit
policy conditions. Unlisted operations are not a guarantee of complete coverage.
