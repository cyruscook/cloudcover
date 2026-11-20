# Direct SAR authorization review

The raw pinned SAR catalog and its checksum-paired manifest are evidence. The
compiler never guesses an action from an operation name and unresolved operations
remain errors, even if they have FAS metadata or a same-name IAM action.

The reviewed `data/authorization.json` bridges cover S3 CreateSession and five
AppStream operations: CreateFleet, UpdateFleet, CreateImageBuilder, CreateStack,
and CreateUpdatedImage. AppStream mappings are accepted only when omitted or
identical to the reviewed operation-table mapping. Missing actions, missing
operations, and conflicting nonempty mappings fail compilation. They are not
unioned with the bridge or replaced with a generic same-name fallback.

## Request-dependent direct links

The CLI defaults to ordinary grants and writes each excluded conditional direct
link, its applicability and AWS source to stderr for both JSON and Terraform HCL.
`--include-conditional` explicitly opts into those grants and still emits the
review diagnostics. AppStream PassRole statements use
`StringEquals: iam:PassedToService = appstream.amazonaws.com`; the role ARN still
needs review. The opt-in does not infer request parameters or produce any
additional cross-account resource policies.

Only these exact operation/action pairs are classified:

| Operation | Conditional action | Applicability |
| --- | --- | --- |
| AppStream CreateFleet, CreateImageBuilder, CreateStack, CreateUpdatedImage | appstream:TagResource | Request supplies tags |
| AppStream CreateFleet, UpdateFleet, CreateImageBuilder | iam:PassRole | Request passes a role to AppStream |
| DynamoDB Scan | dynamodb:ReadDataForReplication | Multi-account replication; review service principal and table resource policy |
| Lambda CreateFunction, UpdateFunctionConfiguration | lambda:AssociateRuntimeAlias, lambda:AssociateRuntimeVersion | Customer-managed runtime alias or version; review which applies |
| Secrets Manager CreateSecret | secretsmanager:PutResourcePolicy | Cross-account replica creation |

Other existing request-dependent links keep their previous behavior. Independent
TagResource and PutResourcePolicy operations still grant their own actions. FAS
traversal, conditions and resource scopes are unchanged. Classification cannot
turn an operation with no ordinary direct authorization into a successful policy.

Library callers use `review_permissions_policy` or `review_permissions_policy_hcl`
with an explicit inclusion boolean. Both return `ReviewedPolicy<T>`, containing
the policy and conditional permission diagnostics that the caller must present.
The older bare JSON/HCL methods cannot return those diagnostics and therefore
return `ConditionalAuthorization` when a reviewed conditional link is present,
rather than silently issuing an incomplete policy. `iam_permissions` and
`iam_permission_catalog` continue to expose all raw/reviewed evidence; they are
not default-grant APIs. This is a deliberate behavior change for the exact tuples
above. Advanced callers must review request applicability and resource policies,
not merely enable every optional permission.

AWS evidence:

- [AppStream operation table](https://docs.aws.amazon.com/service-authorization/latest/reference/list_appstream.html) and each operation's API request parameters.
- [DynamoDB replication authorization](https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/globaltables_MA_security.html).
- [Lambda customer-managed runtime permissions](https://docs.aws.amazon.com/lambda/latest/dg/cmr-permissions.html).
- [Secrets Manager cross-account replication](https://docs.aws.amazon.com/secretsmanager/latest/userguide/auth-and-access_cross-account-replication.html) and [ordinary create policy example](https://docs.aws.amazon.com/secretsmanager/latest/userguide/auth-and-access_iam-policies.html).

## Read-only drift gate

Run `python3 -B tools/review_sar_authorization.py BASELINE_DIR CANDIDATE_DIR`, where
each directory contains the unchanged `catalog.json` and `manifest.json` pair.
It verifies both checksums, reports every previously mapped operation that lost
its links or disappeared, and exits 1 for any transition without an exact reviewed
compiler bridge and retained action references. It reports bridged raw losses
too. It does not modify either snapshot or publish anything. This standalone gate
can be used before updater publication; this change does not alter the updater.

Validation pins main `6892e215617eeee635d3baf11961fb28d5a11d4e` (catalog
`004d21619dbe95ebb3c4f77600173de02bbe7a84a80c83afbc21e2b8008b215a`) and
PR #57 head `3dc1f0dbba0278c403a0dbaa890fa0d444bbc236` (catalog
`36cf78f0309bbba3268998c82150f8c5997a65b724177b3a4e1827edc69f9e42`).
Policy output regressions run against both checksum-paired snapshots in separate
checkouts. Neither raw pair belongs in this source change.

The candidate still loses direct links for CloudWatch CreateAlert, Inspector
RegisterCrossAccountAccessRole, and ODB
DisassociateVirtualMachinesFromExadbVmCluster. Those remain unresolved. Removed
Network Firewall operations are also reported by the gate. These require separate
evidence and disposition: the targeted AppStream repair does not make #57 ready
to merge, and a green ordinary workspace suite does not establish semantic safety
of a SAR refresh.
