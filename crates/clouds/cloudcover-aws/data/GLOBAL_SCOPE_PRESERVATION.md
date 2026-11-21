# Unreviewed global scope transitions

The exact November 20 candidate (`3dc1f0dbba0278c403a0dbaa890fa0d444bbc236`)
adds seven unconditioned global grant atoms across six existing operations:

| Operation | Grant requiring review |
| --- | --- |
| aws-marketplace:AcceptAgreementRequest | aws-marketplace:UpdatePurchaseOrders |
| bedrock-agentcore:UpdateGateway | bedrock-agentcore:CreateGateway |
| identitystore:UpdateUser | sso-directory:DisableUser and sso-directory:EnableUser |
| ram:ListPermissionAssociations | ram:ListPermissionAssociations on `*`, replacing permission ARN scope |
| sagemaker:InvokeEndpoint | sagemaker:CallWithBearerToken |
| sts:GetWebIdentityToken | sts:SetWebIdentityTokenScope |

These entries are a bounded preservation fence. They do not declare that the
upstream action mapping is wrong or that a new global permission is unnecessary.
The application has not reviewed their new default authority or request-dependent
applicability. An explicit upstream global declaration cannot approve that change.

The shared authorization-evidence helper fingerprints each operation's historical
raw links/Context and referenced action, resource and condition definitions. Exact
old evidence preserves all behavior. Changed or ambiguous evidence emits an existing
blocking `manual_review` disposition. All bare/reviewed JSON/HCL policy entry points
reject the entire request, including opt-in and mixed inputs. Raw IAM evidence stays
available. No grant is removed from evidence or guessed from action names, and no
old ARN is restored under disagreeing new metadata.

An operation can retain a scoped base action and gain a different global action.
For example, candidate UpdateGateway retains its scoped update action but adds
CreateGateway on `*`. Checking only missing ARN templates, checking whether any
scoped action remains, or comparing scope changes of previously linked actions
misses this case. The boundary includes operation-to-action links as well as scope.

Fingerprint maintenance requires source review and paired regression evidence;
never update a hash just to make a refresh pass. Unknown fields and most array-order
changes can conservatively block an operation; SDK-only and link/object ordering
changes are normalized. The shared helper refactor preserves #94's role fingerprints.
No public error variant, dependency, raw pin or workflow is changed.

This finite set does not catch every future grant expansion. #57 remains held for
other resource alternatives/replacements, context changes and coverage losses.
The broader provenance-bound comparison gate remains a separate task. Evidence-only
consumers (including site-data's IAM-permission projection) can still build policies
outside these APIs; they must surface policy dispositions before affected data is
promoted. These guards never mean no IAM is needed or that partial policies are safe.
