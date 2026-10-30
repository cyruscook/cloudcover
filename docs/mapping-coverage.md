# Mapping coverage and incomplete policies

SDK adapters retain every source operation. Resolved operations appear in
`SdkMethodMapping::api_methods()`; catalog gaps remain in
`unresolved_api_methods()`. Callers must check `is_complete()` before using a
mapping to build a complete policy. A mapping can be partly resolved. Unsupported
SDK releases remain explicit provider errors.

An empty API list with no unresolved operations preserves the existing Terraform
known-no-call meaning. An empty list with unresolved source operations is
incomplete. CLI policy commands fail on either partial or wholly unresolved
requested mappings and missing Terraform lifecycle mappings. Unused catalog gaps
do not prevent policy generation. Go references absent from the SDK extractor remain excluded: the analyzer also
returns transitive SDK helper and waiter references, so absence alone is not proof
of a missing API mapping. Source-operation catalog gaps are always retained.

Migration: the additional serialized `unresolved_api_methods` field is optional
and defaults to empty for older values. Existing constructors and resolved lists
are unchanged. Consumers using provider enumeration should handle the retained
unresolved rows, rather than assume enumeration contains resolved-only entries.

## AWS caller IAM authorization

Resolving an SDK call to an API operation does not establish that operation's
authorization requirements. `AwsProvider::authorization_coverage()` distinguishes
mapped IAM actions, documented absence of caller IAM permissions, and unresolved
authorization. Missing or empty `AuthorizedActions` metadata is unresolved unless
a reviewed AWS source explicitly establishes a different authentication model.
The reviewed exceptions and source URLs are enumerated in
`crates/clouds/cloudcover-aws/src/authorization.rs`; token, role trust, and resource
authorization can still be required for those calls.

JSON and HCL policy generation, including `iam_permissions()`, reject requested
operations with unresolved direct or downstream authorization. Mixed requests
produce an error rather than a partial policy. FAS evidence does not fill a
missing direct authorization mapping. Empty requests and Terraform known-no-call
mappings remain valid, and unrelated unresolved catalog entries do not block a
resolved request.

Go source analysis can discover conditional SDK helper calls. For example, modern
S3 SDK `GetObject` code can reach `CreateSession`, whose direct authorization is
missing from the pinned catalog. Such a request now fails instead of emitting an
incomplete policy. The CLI success fixture explicitly pins S3 SDK v1.45.1, before
`CreateSession` was introduced; the modern SDK fixture verifies this rejection.

The site's API artifact uses schema version 2: a row's permission IDs are `null`
when authorization is unresolved, `[]` for a resolved empty permission set, or an
array for resolved permissions. A `null` FAS entry means its downstream
authorization could not be resolved. The browser labels unresolved rows and
disables copying a selection that contains them. IAM and Terraform artifact
schemas remain at version 1. Catalog source documents, their provenance, and SDK
histories are not rewritten by this classification.
