# Four-operation PassRole context preservation

This separate boundary covers only Cognito Identity SetIdentityPoolRoles, EMR
StartSession, Observability Admin CreateS3TableIntegration and SageMaker
CreateTrainingSession. It reuses #94's full-closure fingerprint and blocking
disposition machinery without changing the original 22-entry boundary or messages.
The four historical fingerprints are exactly those in the local decision packet.

The candidate changes raw `iam:PassedToService` evidence while the role grants
remain unchanged. Context absence becomes a service constraint for Cognito;
EMR's service value changes; Observability Admin and SageMaker lose a service
alternative. Equal rendered grants do not establish equivalent authorization.

Only exact historical evidence preserves old behavior. Changed, missing,
ambiguous, invalid or unknown closure evidence blocks the entire request in bare
and reviewed JSON/HCL, including opt-in and mixed requests. Existing errors and
diagnostics are reused. No permission, concrete requested role or IAM condition
is inferred, added, dropped from evidence or reconstructed from policy prose.

A strict JSON check prevents duplicate object keys from collapsing into a matching
historical Context. Raw JSON ambiguity conservatively blocks these four operations;
it does not expand this boundary to other operations. Ordinary compiler validation
may reject malformed catalogues before any policy can be rendered.

The fingerprint includes raw operation links/Context and referenced actions,
resources and condition definitions, retaining unknown fields and absent/null
distinctions. SDK-only and object/link ordering differences are normalized by the
unchanged shared helper. Updating a fingerprint requires source review and paired
regression evidence; this is not a grant allowlist or automatic snapshot approval.

The guard is restricted to four operations. Other historical PassRole paths,
scope guards, shared FAS rules and unrelated outputs retain existing behavior.
Evidence-only consumers can still access IAM relationships and must surface policy
blockers separately. This does not unblock #57 or authorize catalogue publication.
Other resource/context changes, coverage losses and the authenticated updater gate
remain separate work. The raw catalogue, manifest, workflows and public API stay
unchanged.
