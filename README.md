# CloudCover

A project to process data on API methods, associated permissions, and corresponding SDK methods across cloud providers.

Users should be able to find the required permissions for the API methods they're calling by providing their source code. CloudCover will analyse the source code, discover references to SDK methods, and find the corresponding API methods and required permissions. CloudCover can then output a permissions policy.

Directories:
* `crates/cloudcover-cli`: CLI, for user facing operations utilising CloudCover, such as outputting permissions policy for given source code
* `crates/cloudcover-core`: shared definitions and functionality
* `crates/clouds/`: support for cloud providers
* `crates/langs/`: support for analysing source code languages
* `crates/sdks/`: analyses SDKs and provides maps of SDK methods -> API methods
