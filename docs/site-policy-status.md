# Browser policy blocker projection

The API data remains schema v2 with three-column operation rows. An additive
`policy_status` array follows the same sorted operation index. `unknown` means
request-specific review has not been established; it is never approval, even when
this engine records no blocker. An optional diagnostic on `unknown` preserves
informational warnings for exactly retained legacy mappings.

At generation time, each operation is evaluated with the existing reviewed-default
JSON API, including its FAS dependencies. A policy is neither exported nor shown.
An unresolved result becomes `blocked` with the engine's diagnostic and any
reviewed blocking disposition source for the actual failing operation, if known.
A FAS leaf never inherits an unrelated root warning's source. Unexpected engine
errors still abort generation.
This does not infer request arguments, validate all possible mixed requests,
authorize catalog publication, or replace evidence review.

Blocked operations have a null legacy authorization marker. Older clients therefore
retain unresolved-copy blocking, including mixed selections. The status record
keeps `evidence_permission_ids` separately (null when unknown), so newer browsers
can show raw relationships and independent IAM details without allowing a blocked
mapping list to be copied. The IAM reference files, FAS evidence, method indexes,
Terraform relationships and navigation identifiers remain independent.

The browser accepts older artifacts without status as unknown. Present malformed
status arrays, unsupported states, invalid references/citations and blocked rows
without a null legacy marker are rejected. Missing status never becomes reviewed.
An explicitly unknown status cannot carry blocked evidence IDs or accompany a null
legacy marker; this does not change older artifacts with no status field. Blank
diagnostics, including whitespace-only messages, are rejected.
Both unknown and blocked views retain the permission-evidence disclaimer. Only
blocked/unresolved mapping selections disable list copying; ordinary reference
views and independent action/ARN copying keep their existing behavior.

The paired fixture build layers the unchanged #97 context guard and exact held
November 20 catalog/manifest in a separate local checkout. Neither fixture input
belongs in this source patch. Actual generation verifies historical role evidence,
new blocked role/scope cases, legitimate empty IAM evidence, nonblocking lifecycle
warnings, and unchanged IAM/FAS/SDK relationships. Full-generation overhead and
artifact-size measurements are part of the separate local verification receipt.
