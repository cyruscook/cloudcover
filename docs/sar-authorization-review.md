# Direct SAR authorization review

The raw pinned SAR catalog and its checksum-paired manifest are evidence. The
compiler never guesses an action from an operation name and unresolved operations
remain errors, even if they have FAS metadata or a same-name IAM action.

The reviewed `data/authorization.json` bridges cover S3 CreateSession and five
AppStream operations: CreateFleet, UpdateFleet, CreateImageBuilder, CreateStack,
and CreateUpdatedImage. A separate exact bridge covers ODB
DisassociateVirtualMachinesFromExadbVmCluster, using the action explicitly linked
by AWS and its retained exadb-vm-cluster ARN scope. Both compilation and rendering
reject missing, wildcard or changed ODB scope; generic resource fallback cannot
widen this grant. AppStream mappings are accepted only when omitted or
identical to the reviewed operation-table action and Context shape. A changed, stricter, missing or unexpected context is rejected. Missing actions, missing
operations, and conflicting nonempty mappings fail compilation. They are not
unioned with the bridge or replaced with a generic same-name fallback.

## Request-dependent direct links

The CLI defaults to ordinary grants and writes each excluded conditional direct
link, its applicability and AWS source to stderr for both JSON and Terraform HCL.
`--include-conditional` explicitly opts into those grants and still emits the
review diagnostics. AppStream PassRole is always diagnostic-only, even with the
opt-in: source analysis does not establish a concrete requested role ARN. The
diagnostic requires a separate manual grant scoped to that validated ARN with
`StringEquals: iam:PassedToService = appstream.amazonaws.com`. Incomplete or
missing role resource metadata cannot cause a wildcard or unconditioned grant.
The opt-in does not infer request parameters or produce additional cross-account
resource policies.

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
with an explicit inclusion boolean for eligible conditional grants; manual-only
PassRole links are never rendered. Both return `ReviewedPolicy<T>`, containing
the policy and conditional permission diagnostics that the caller must present.
The older bare JSON/HCL methods cannot return those diagnostics and therefore
return `ConditionalAuthorization` when a reviewed conditional link is present,
rather than silently issuing an incomplete policy. `iam_permissions` and
`iam_permission_catalog` continue to expose all raw/reviewed evidence; they are
not default-grant APIs. The public error variants also affect exhaustive error matches. This is a deliberate behavior change for the exact tuples
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
compiler bridge, matching reviewed baseline context shape, and retained action references. It reports bridged raw losses
too. It does not modify either snapshot or publish anything. This standalone gate
can be used before updater publication; this change does not alter the updater.

Validation pins main `6892e215617eeee635d3baf11961fb28d5a11d4e` (catalog
`004d21619dbe95ebb3c4f77600173de02bbe7a84a80c83afbc21e2b8008b215a`) and
PR #57 head `3dc1f0dbba0278c403a0dbaa890fa0d444bbc236` (catalog
`36cf78f0309bbba3268998c82150f8c5997a65b724177b3a4e1827edc69f9e42`).
Policy output regressions run against both checksum-paired snapshots in separate
checkouts. Neither raw pair belongs in this source change.

## Reviewed drift dispositions

`data/authorization-dispositions.json` records exact historical action/Context
shapes and retained action/resource scopes. The compiler accepts the historical
populated mapping or the reviewed transition: empty-but-present for CloudWatch
and Inspector, absent operation for the twenty Network Firewall rows. A different
nonempty mapping, changed Context (including explicit null), changed action or
resource scope, missing referenced action, or a different transition fails review.
The gate reports every loss; exact acknowledgments remain visible as
`reviewed_manual_review`, `reviewed_retired`, or `reviewed_deprecated_sdk_api`.
They never mean `NotRequired`, never suppress newly affected operations, and
never authorize a successful empty policy.

All JSON/HCL policy APIs, including both reviewed inclusion modes, return an
`AuthorizationDisposition` error for these exact operations, alone or in a mixed
request. They return no partial policy. CLI incomplete-SDK-mapping errors also include
the disposition when an unresolved row exactly identifies a reviewed legacy
operation. Unknown rows keep their existing failure behavior.
`authorization_disposition(&method)` is
an additive inspection API returning the same structured diagnostic even when a
legacy operation is absent from the current catalog. Raw IAM evidence APIs retain
their existing metadata/coverage meaning; metadata does not prove availability or
that a grant is safe. Callers must present policy errors and every successful
`ReviewedPolicy` diagnostic. Its public fields and `#[must_use]` do not enforce
presentation. Older bare policy APIs remain strict; this patch adds no global
logging, hidden options or redesign of the core provider trait.

| Exact operation(s) | Disposition and required next step |
| --- | --- |
| ODB DisassociateVirtualMachinesFromExadbVmCluster | The reviewed single base action renders only with `arn:${Partition}:odb:${Region}:${Account}:exadb-vm-cluster/${ExadbVmClusterId}`. Substitute the intended cluster. No PassRole or other action is inferred. |
| Inspector Classic RegisterCrossAccountAccessRole | Retired: AWS ended support and access to Classic resources May 20, 2026. Review a migration to Inspector2 separately. No API alias or automatic role grant is supplied. |
| CloudWatch Omni CreateAlert | Manual authorization review: AWS explicitly links the action in its SDK model. Its managed policy shows `StringEquals cloudwatch:HasAccessGrant=true`; the API requires a caller-supplied space and access profile. Review these together. This is an AWS-supported conditioned policy example, not proof of complete request authorization. No automatic action or PassRole grant is emitted. |
| The twenty explicitly listed Network Firewall Egress Proxy operations | Deprecated SDK APIs: the v1.80.0 model removes exactly these operations. This does not establish endpoint shutdown. Review migration to current Firewall Proxy APIs; no replacement or legacy grant is inferred. DeleteProxy and DescribeProxy keep their mapped grants. UpdateProxySettings remains in the model but its IAM mapping was already unresolved in both snapshots; it stays unresolved. All three are outside this disposition. |

Primary evidence:

- [ODB operation/action/resource table](https://docs.aws.amazon.com/service-authorization/latest/reference/list_odb.html), [VM-removal API](https://docs.aws.amazon.com/odb/latest/APIReference/API_DisassociateVirtualMachinesFromExadbVmCluster.html), and [AmazonODBExascaleVmClusterAdmin](https://docs.aws.amazon.com/aws-managed-policy/latest/reference/AmazonODBExascaleVmClusterAdmin.html).
- [Inspector Classic end-of-support notice](https://docs.aws.amazon.com/managedservices/latest/userguide/inspector.html) and [the pinned Classic SDK model](https://raw.githubusercontent.com/aws/aws-sdk-go-v2/bdca6eea5804101da37e4567f25c3b39b2eaae52/codegen/sdk-codegen/aws-models/inspector.json). The retained Classic SDK operation does not establish a live service, and is not an Inspector2 operation.
- [CloudWatch Omni CreateAlert API](https://docs.aws.amazon.com/cloudwatch-omni/latest/APIReference/API_CreateAlert.html), [explicit IAM action trait in the pinned SDK model](https://raw.githubusercontent.com/aws/aws-sdk-go-v2/bdca6eea5804101da37e4567f25c3b39b2eaae52/codegen/sdk-codegen/aws-models/cloudwatchomni.json), [CloudWatch SAR](https://docs.aws.amazon.com/service-authorization/latest/reference/list_cloudwatch.html), and [CloudWatchOmniSpaceAccessPolicy](https://docs.aws.amazon.com/aws-managed-policy/latest/reference/CloudWatchOmniSpaceAccessPolicy.html).
- Network Firewall [pre-change model](https://raw.githubusercontent.com/aws/aws-sdk-go-v2/aeab1d85b9e2fbfbbe193ce93907b60ae32f8a28/codegen/sdk-codegen/aws-models/network-firewall.json), [post-change model](https://raw.githubusercontent.com/aws/aws-sdk-go-v2/d0c69168827d92b7f44d04fcaf21e5f317d440cd/codegen/sdk-codegen/aws-models/network-firewall.json), [v1.80.0 deprecation release note](https://raw.githubusercontent.com/aws/aws-sdk-go-v2/service/networkfirewall/v1.80.0/service/networkfirewall/CHANGELOG.md), and [published SAR](https://docs.aws.amazon.com/service-authorization/latest/reference/list_network-firewall.html). SAR still lists the historical links and retained IAM actions. This documentary/model difference is why the diagnostic says deprecated rather than shut down.

The gate acknowledges all 28 previously mapped losses in the reviewed snapshot:
six compiler bridges, one manual review, one retirement, and twenty deprecated
SDK APIs. Acknowledgment is not restored functionality for the blocked operations
and does not establish semantic safety of every other refresh change. The gate
is a local read-only command; updater workflows and publication remain unchanged.
PR #57 still requires an independent full-refresh review and a separate merge
decision. Conditional advanced-use grants and AppStream role scope still require
the manual steps described above.
