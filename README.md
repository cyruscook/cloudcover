# CloudCover

A project to process data on API methods, associated permissions, and corresponding SDK methods across cloud providers.

## Installation

Install the command with cargo-binstall:

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
