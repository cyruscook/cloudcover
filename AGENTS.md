# CloudCover

A project to process data on API methods, associated permissions, and corresponding SDK methods across cloud providers.

Users should be able to find the required permissions for the API methods they're calling by providing their source code. CloudCover will analyse the source code, discover references to SDK methods, and find the corresponding API methods and required permissions. CloudCover can then output a permissions policy.

Current workspace layout:
- `crates/cloudcover-cli`: user-facing CLI entrypoint.
- `crates/cloudcover-core`: shared model types such as languages, SDKs, method references, API methods, and mappings.
- `crates/clouds/`: cloud-provider implementations.
- `crates/langs/`: source-language analyzers.
- `crates/sdks/`: SDK analyzers and generated SDK-to-API mapping crates.

ALWAYS run `make check` after any changes.

