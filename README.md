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

## Scheduled mapping updates

GitHub Actions refreshes Terraform AWS provider data and the AWS SDK mappings
daily. Each workflow tests changed data and creates or updates a dedicated
automation PR; runs with no data changes do not create a commit or PR.

| Mapping | Daily schedule (UTC) | Manual generation |
| --- | --- | --- |
| Terraform AWS provider | 06:17 | Exact `version`, inclusive `from`/`to` range, or all stable releases |
| AWS SDK Go v1 | 06:37 | All stable v1 releases from a fresh bare SDK clone |
| AWS SDK Go v2 | 06:47 | Exact service `tag` (for example, `service/s3/v1.104.0`), or all stable service releases |
| AWS SDK JavaScript/TypeScript v3 | 06:57 | All stable current and historical client package releases |

Run these workflows from the repository's Actions tab using **Run workflow**.
Go v2 skips already recorded releases. Go v1 and JavaScript/TypeScript regenerate
their complete histories because their generators replace the output file.
JavaScript and TypeScript share the same SDK mapping data and update workflow.
