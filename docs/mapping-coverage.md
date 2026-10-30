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

## S3 Express and conditional SDK calls

A reviewed supplement in `crates/clouds/cloudcover-aws/data/authorization.json`
bridges API operation `s3:CreateSession` to IAM action
`s3express:CreateSession`. The existing pinned SAR supplies that action's resource
and condition metadata; raw SAR documents remain unchanged. The build rejects
missing references, duplicate mappings, absent evidence and contradictory upstream
mappings. An identical upstream fix is accepted, allowing the supplement to be
retired after review. This is not a permissionless exception or a name-based guess.

Go analysis starts from all functions in the requested source packages, including
library entrypoints and initialization; unused functions in imported dependencies
no longer contribute calls. It still follows reachable dependency wrappers and
interface dispatch, rather than silently dropping SDK internals.

Go S3 service module v1.46.0 introduced S3 Express. With v1.104.0, an ordinary
`GetObject` resolves normal SigV4 credentials; directory endpoints can instead
resolve Express credentials and call `CreateSession`. Rooted callgraph analysis
still includes both conditional branches: it does not evaluate bucket arguments,
endpoint overrides or `DisableS3ExpressSessionAuth`. A modern SDK policy therefore
conservatively includes the documented session action and emits an explicit stderr
warning. It does **not** establish that ordinary S3 needs this permission. Direct
API lookup for `s3:GetObject` does not acquire the SDK-only helper dependency.

Review and substitute the directory bucket/access-point ARN placeholders before
using the generated template; no bucket is inferred. `CreateSession` can authorize
more than reads, and the analyzer does not infer `s3express:SessionMode`. Do not
silently assume ReadOnly from `GetObject`. The existing FAS supplement separately
includes the possible SSE-KMS requirements with their applicability and key scope.
This is a conservative template, not an argument-sensitive least-privilege proof.
For a general-purpose-only deployment, review the session statement as inapplicable
and remove it from the deployment policy; for directory buckets, retain it scoped
to the actual directory resource and intended session mode. Unknown deployment
context must not be treated as proof that sessions cannot occur.

The modern fixture now verifies successful JSON/HCL generation with a visible
conditional-grant warning. The v1.45.1 fixture remains a historical regression
that has no session helper, not a recommended SDK downgrade. Unrelated unresolved
authorization still rejects the entire request.

Sources:
- [AWS CreateSession API](https://docs.aws.amazon.com/AmazonS3/latest/API/API_CreateSession.html)
- [S3 Express SAR](https://docs.aws.amazon.com/service-authorization/latest/reference/list_s3express.html)
- [Go SDK v1.46.0 changelog](https://github.com/aws/aws-sdk-go-v2/blob/service/s3/v1.46.0/service/s3/CHANGELOG.md)
- [Go SDK v1.104.0 identity selection](https://github.com/aws/aws-sdk-go-v2/blob/service/s3/v1.104.0/service/s3/options.go)
- [Go SDK v1.104.0 session provider](https://github.com/aws/aws-sdk-go-v2/blob/service/s3/v1.104.0/service/s3/express_default.go)

The site's API artifact uses schema version 2: a row's permission IDs are `null`
when authorization is unresolved, `[]` for a resolved empty permission set, or an
array for resolved permissions. A `null` FAS entry means its downstream
authorization could not be resolved. The browser labels unresolved rows and
disables copying a selection that contains them. IAM and Terraform artifact
schemas remain at version 1. Catalog source documents, their provenance, and SDK
histories are not rewritten by this classification.
