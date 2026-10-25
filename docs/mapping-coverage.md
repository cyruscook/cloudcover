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
