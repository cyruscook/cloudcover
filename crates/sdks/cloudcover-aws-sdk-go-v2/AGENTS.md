# cloudcover-aws-sdk-go-v2

This crate exposes generated static mappings from aws-sdk-go-v2 Go method references to AWS API operations.

Structure:
- `build.rs`: manages shared build cache, SDK checkout refresh, generator execution, and emitted Rust static data.
- `generator/`: Go analyzer using `golang.org/x/tools/go/packages`, SSA, and static callgraph analysis over aws-sdk-go-v2 source.
- `src/lib.rs`: public generated mapping types and tests.

Role in the workspace:
- Supplies Go SDK method-to-API mappings to `cloudcover-aws`.
- Supplies the AWS SDK operation map reused by `cloudcover-terraform-provider-aws`
