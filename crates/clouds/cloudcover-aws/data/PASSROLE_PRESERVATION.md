# Local PassRole preservation boundary

This source-only defense covers the 22 new default `iam:PassRole` paths found in
candidate SAR commit `3dc1f0dbba0278c403a0dbaa890fa0d444bbc236`: 12 existing
operations (five previously unresolved) and 10 newly indexed operations. It does
not approve their role grants or supply request-dependent permissions.

`passrole-preservation.json` records paired input provenance and one historical
authorization fingerprint per operation. It contains no duplicate action mappings,
service-principal allowlist or substitute SAR definitions. Fingerprints include
raw operation fields except the SDK projection, complete AuthorizedActions Context,
and referenced action, resource and condition-key definitions. Missing versus null
is retained; duplicate definitions are ambiguous. An absent operation is bound to
explicit-operation mode, so removing the table cannot enable action-name fallback.
Object ordering, link ordering and SDK-only changes do not require reauthorization;
other array ordering and unknown-field changes conservatively require review.

The compiler emits an existing `manual_review` disposition whenever a covered
operation differs from its historical evidence. This cannot add grants: all bare
and reviewed JSON/HCL policy APIs reject the entire request, even with conditional
opt-in or an otherwise valid operation. The shared guard also applies to FAS leaves.
IAM coverage and permission inspection retain the upstream evidence; the disposition
explains the blocker. No new public error variant or request-scope API is introduced.
The exact old pin retains all behavior, including unknown and unresolved operations.
An unchanged historical path outside these 22 receives no new exception or blocker.

Neither a template role ARN nor raw `iam:PassedToService` Context supplies a concrete
requested role or reviewed service scope. Resolve an affected request with a separately
reviewed, concretely scoped manual policy. Any future additive request-bound role API
needs separate security and compatibility review; the existing boolean opt-in cannot
bypass this boundary. Removing a new role link does not automatically approve other
changed evidence; only the exact historical fingerprint restores historical behavior.

The fingerprints are preservation assertions, not IAM implication proofs. They are
intentionally conservative and can block a benign future metadata change. Updating
a fingerprint requires reviewing the old/new evidence and rerunning complete paired
projections; never change a fingerprint merely to make a refresh pass. The ledger is
trusted source, and source review remains necessary. Consumers using evidence-only
APIs to build their own policies do not receive automatic policy enforcement.

This does not make #57 safe. Seven new global grant atoms on existing operations
(including the RAM widening), 266 added resource alternatives, 28 incomparable scope
replacements, four changed historical PassRole contexts, five nonempty mapping losses,
and 27 other fallback losses remain separate review gates. The incomplete-scope defense
blocks 22 different operations without repairing their missing upstream definitions.
The semantic comparison gate remains local and is not enforced in the live updater.
Future new role paths outside this finite reviewed set still require that broader gate;
this patch is not a general IAM analyzer or a permanent blanket exception.
