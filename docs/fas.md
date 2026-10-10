# AWS forward access session supplement

Reviewed: 2026-10-10. The executable source document is
[`fas.json`](../crates/clouds/cloudcover-aws/data/fas.json); it is checked in,
embedded in the AWS provider, and shipped as `data/fas.json` with the website.
Every mapping below is backed by direct AWS documentation. No third-party mapping
dataset or IAM Policy Autopilot data/code is used.

AWS explicitly [excludes FAS permissions from operation-to-action mappings](https://docs.aws.amazon.com/service-authorization/latest/reference/service-reference.html).
The action inventory still includes those permissions. This document supplements
operation dependencies, rather than replacing the Service Authorization Reference.
[AWS's FAS guide](https://docs.aws.amazon.com/IAM/latest/UserGuide/access_forward_access_sessions.html)
explains that downstream calls are authorized against the original caller.
Service roles, service-linked roles, execution roles, and grants can instead supply
internal service permissions. The catalog distinguishes those cases.

## Output contract

CloudCover includes all **documented possible** dependencies for an identified
operation. The source analyzers currently identify operations, not encryption
arguments, existing resource configurations, key policies, grants, or IAM roles.
Applicability notes below are descriptive predicates; they are not evaluated at
runtime. This produces a conservative baseline for covered operations. It does not
establish an exact least-privilege policy for a particular deployment.

* JSON and Terraform HCL preserve KMS service conditions and separate grants for
  independently replaceable resource roles. Direct KMS calls retain unrestricted
  service access; they are never merged into a FAS-only grant.
* `AwsProvider::iam_permissions` is the sorted action union, including possible
  FAS dependencies. It omits conditions. `fas_dependencies` exposes rule evidence,
  applicability, resource role, and the complete parent rule path;
  `fas_coverage` returns `documented_dependencies`, `reviewed_no_dependencies`,
  or `unknown`. The website displays these distinctions and sources, and copied
  names remain an action-only projection.
* CLI notes on stderr identify possible dependencies and unknown coverage.
  Standard output remains a valid JSON policy or Terraform HCL document.
* Dependencies expand transitively. For example, an SSM secret reference requires
  Secrets Manager access and, for a customer-managed secret key, KMS decryption.
  Both the reference predicate and the key predicate apply. Catalog validation
  rejects cycles, unsupported schema/fields, dangling operation/action references,
  contradictory coverage records, and missing primary-source evidence.
* A missing entry means **unknown**, even for a service which has other mappings.
  A reviewed no-dependency entry concerns the documented base operation, not every
  possible integration, SDK helper, session-authorization call, or future feature.

## Resources and conditions

Replace ARN placeholders before deployment. In addition to the reference's
`${Partition}`, `${Region}`, and `${Account}`, the supplement uses distinct key
placeholders such as `${SourceKeyId}`, `${DestinationKeyId}`, `${OldKeyId}`,
`${NewKeyId}`, and `${ReplicaKeyId}`. Replication uses `${PrimaryRegion}` and
`${ReplicaRegion}` independently in both key ARNs and service conditions. For
cross-account or cross-Region copies, replace each source/destination ARN and its
service condition with the actual key/account/Region; common placeholders are not
an assertion that the two keys are in the same account or Region. Non-KMS resources
reuse reference ARN templates; scope credential/secret grants to the actual
cluster, workgroup, database user, or secret.

The [KMS condition reference](https://docs.aws.amazon.com/kms/latest/developerguide/conditions-kms.html#conditions-kms-via-service)
says `kms:ViaService` applies to FAS, lists supported services, and requires the
`.amazonaws.com` suffix in **all partitions**, including China. ARN partition and
service-name suffix are intentionally independent. A service restriction does not
prove encryption is enabled. The DynamoDB guide requires a Region-independent
`dynamodb.*.amazonaws.com` restriction for global tables, emitted with `StringLike`.
The Redshift Data API KMS guide's grant example explicitly uses the nonregional
`redshift.amazonaws.com` value; this record follows that service-specific example
rather than silently substituting the regional name from the general table.

AWS-managed keys often already authorize service use in their key policies.
Customer-managed keys can authorize via IAM, key policy, or grants. A possible
caller requirement does not mean every deployment needs an additional identity
policy statement. This tool emits identity-policy candidates; it does not edit
key policies or create grants. Grant-authorized background calls are not blanket
caller dependencies. `kms:DescribeKey` is added only where API caller documentation
supports it, not for console key pickers.

## Coverage and source audit

The tables below enumerate every executable rule, with operation identity,
conditional applicability, downstream calls, resource role, condition, and direct
AWS evidence. They include general-purpose/directory S3, checksum, multipart,
copy, Secrets Manager value/key validation/replication, messaging, Parameter Store
tiers and secret references, DynamoDB table access/key selection, Data API
credentials and grants, Step Functions caller/role distinctions, ECR lifecycle
grants, EBS volume calls, and Lambda environment-variable configuration.

This is a sourced catalog for twelve services, not a claim of exhaustive coverage
of every AWS service and option. AWS does not publish a complete machine-readable
FAS dependency catalog. In particular, CloudFormation without a service role can
forward the caller's identity to template-dependent services; Athena and Glue
permissions depend on queries, data locations, catalogs, Lake Formation, and
execution roles. EC2 instance-launch/snapshot-copy workflows and Lambda code
artifact encryption need further operation-level audits beyond the volume and
environment-variable entries below. These are **unknown**, not silently inferred
from KMS integration lists or service policies. S3 session/helper operations must
also be discovered by SDK analysis where relevant. AWS additions may outpace the
reviewed catalog.

Downstream entries are API operation identities, not literal IAM action strings.
For example, KMS `ReEncrypt` resolves through the AWS reference to
`kms:ReEncryptFrom` and `kms:ReEncryptTo`.

Evidence is a concise summary of the cited passage, not necessarily a verbatim
quotation. Groups derived from a documented producer/consumer or table-access
category use that category's primary source; no map is imported as evidence.

### dynamodb

#### dynamodb-access

* Parent: `dynamodb:GetItem`, `dynamodb:BatchGetItem`, `dynamodb:Query`, `dynamodb:Scan`, `dynamodb:TransactGetItems`, `dynamodb:PutItem`, `dynamodb:BatchWriteItem`, `dynamodb:UpdateItem`, `dynamodb:DeleteItem`, `dynamodb:TransactWriteItems`, `dynamodb:ExecuteStatement`, `dynamodb:BatchExecuteStatement`, `dynamodb:ExecuteTransaction`.
* Downstream: `kms:Decrypt`.
* Applies when: Table uses customer-managed or AWS-managed KMS encryption; table-key cache miss on caller access. AWS-owned keys need no additional permissions.
* Resource: table key. Key `${TableKeyId}`, Region `${Region}`.
* KMS service restriction: `dynamodb.*.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/encryption.usagenotes.html): When you access an encrypted table, DynamoDB sends a Decrypt request; the user in the event is the principal accessing the table.

#### dynamodb-select-key

* Parent: `dynamodb:CreateTable`, `dynamodb:UpdateTable`.
* Downstream: `kms:Encrypt`, `kms:Decrypt`, `kms:ReEncrypt`, `kms:GenerateDataKey`, `kms:GenerateDataKeyWithoutPlaintext`, `kms:DescribeKey`, `kms:CreateGrant`.
* Applies when: Selecting a customer-managed table encryption key. Background maintenance uses grants rather than adding its calls to every caller.
* Resource: selected table key. Key `${TableKeyId}`, Region `${Region}`.
* KMS service restriction: `dynamodb.*.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/encryption.usagenotes.html): The principal who selects the customer-managed key must have Encrypt, Decrypt, ReEncrypt*, GenerateDataKey*, DescribeKey and CreateGrant.

### ec2

#### ec2-volume

* Parent: `ec2:CreateVolume`.
* Downstream: `kms:CreateGrant`, `kms:GenerateDataKeyWithoutPlaintext`, `kms:Decrypt`.
* Applies when: Creating an encrypted EBS volume, including encryption by default, from an unencrypted snapshot.
* Resource: volume key. Key `${VolumeKeyId}`, Region `${Region}`.
* KMS service restriction: `ec2.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/ebs/latest/userguide/how-ebs-encryption-works.html): EC2 sends CreateGrant, GenerateDataKeyWithoutPlaintext and Decrypt when creating an encrypted volume from an unencrypted snapshot.

#### ec2-volume-encrypted-snapshot

* Parent: `ec2:CreateVolume`.
* Downstream: `kms:GenerateDataKeyWithoutPlaintext`.
* Applies when: Creating an encrypted volume from an encrypted snapshot owned by the caller. This covers the documented same-key case; different-key source permissions remain unknown.
* Resource: volume key. Key `${VolumeKeyId}`, Region `${Region}`.
* KMS service restriction: `ec2.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/ebs/latest/userguide/how-ebs-encryption-works.html): EC2 sends GenerateDataKeyWithoutPlaintext with the KMS key chosen for volume encryption.

#### ec2-attach

* Parent: `ec2:AttachVolume`.
* Downstream: `kms:CreateGrant`, `kms:Decrypt`.
* Applies when: Attaching an encrypted EBS volume to an instance.
* Resource: volume key. Key `${VolumeKeyId}`, Region `${Region}`.
* KMS service restriction: `ec2.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/ebs/latest/userguide/how-ebs-encryption-works.html): When you attach an encrypted volume to an instance, EC2 sends CreateGrant and Decrypt requests to KMS.

#### ec2-ebs-key-validation

* Parent: `ec2:CreateVolume`, `ec2:AttachVolume`.
* Downstream: `kms:DescribeKey`.
* Applies when: Using a KMS key for EBS encryption.
* Resource: volume key. Key `${VolumeKeyId}`, Region `${Region}`.
* KMS service restriction: `ec2.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/ebs/latest/userguide/ebs-encryption-requirements.html): Required user permissions for EBS encryption include kms:DescribeKey.

### ecr

#### ecr-create

* Parent: `ecr:CreateRepository`.
* Downstream: `kms:DescribeKey`, `kms:CreateGrant`, `kms:RetireGrant`.
* Applies when: Repository encryptionType=KMS or KMS_DSSE; the creator configures KMS-backed repository encryption.
* Resource: repository key. Key `${RepositoryKeyId}`, Region `${Region}`.
* KMS service restriction: `ecr.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonECR/latest/userguide/encryption-at-rest.html): The repository creator must have kms:CreateGrant, kms:RetireGrant and kms:DescribeKey. ECR uses grants for image encryption/decryption.

#### ecr-delete

* Parent: `ecr:DeleteRepository`.
* Downstream: `kms:RetireGrant`.
* Applies when: Deleting a KMS-encrypted repository and retiring its encryption grants.
* Resource: repository key. Key `${RepositoryKeyId}`, Region `${Region}`.
* KMS service restriction: `ecr.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonECR/latest/userguide/encryption-at-rest.html): The IAM principal that deletes the repository must have kms:RetireGrant permission.

### kinesis

#### kinesis-produce

* Parent: `kinesis:PutRecord`, `kinesis:PutRecords`.
* Downstream: `kms:GenerateDataKey`.
* Applies when: Stream uses customer-managed KMS encryption.
* Resource: stream key. Key `${StreamKeyId}`, Region `${Region}`.
* KMS service restriction: `kinesis.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/streams/latest/dev/permissions-user-key-KMS.html): Kinesis stream producers must have kms:GenerateDataKey permission.

#### kinesis-consume

* Parent: `kinesis:GetRecords`, `kinesis:SubscribeToShard`.
* Downstream: `kms:Decrypt`.
* Applies when: Consuming records from a stream using customer-managed KMS encryption.
* Resource: stream key. Key `${StreamKeyId}`, Region `${Region}`.
* KMS service restriction: `kinesis.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/streams/latest/dev/permissions-user-key-KMS.html): Kinesis stream consumers must have kms:Decrypt permission.

### lambda

#### lambda-configure-key

* Parent: `lambda:CreateFunction`, `lambda:UpdateFunctionConfiguration`.
* Downstream: `kms:CreateGrant`, `kms:Encrypt`.
* Applies when: Configuring a customer-managed KMS key for server-side environment-variable encryption.
* Resource: function key. Key `${FunctionKeyId}`, Region `${Region}`.
* KMS service restriction: `lambda.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/lambda/latest/dg/configuration-envvars-encryption.html): kms:CreateGrant and kms:Encrypt are needed to configure a customer-managed key on a function. Lambda uses your permissions to create a grant.

#### lambda-view-env

* Parent: `lambda:GetFunction`, `lambda:GetFunctionConfiguration`.
* Downstream: `kms:Decrypt`.
* Applies when: Viewing environment variables encrypted with a customer-managed key.
* Resource: function key. Key `${FunctionKeyId}`, Region `${Region}`.
* KMS service restriction: `lambda.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/lambda/latest/dg/configuration-envvars-encryption.html): kms:Decrypt is needed to view and manage environment variables encrypted with a customer-managed key.

### redshift-data

#### redshift-executestatement-serverless

* Parent: `redshift-data:ExecuteStatement`.
* Downstream: `redshift-serverless:GetCredentials`.
* Applies when: Temporary credentials for a serverless workgroup.
* Resource: workgroup credentials.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_ExecuteStatement.html): Permission to call redshift-serverless:GetCredentials is required.

#### redshift-executestatement-iam

* Parent: `redshift-data:ExecuteStatement`.
* Downstream: `redshift:GetClusterCredentialsWithIAM`.
* Applies when: Temporary credentials for a provisioned cluster as an IAM identity.
* Resource: cluster credentials.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_ExecuteStatement.html): Permission to call redshift:GetClusterCredentialsWithIAM is required.

#### redshift-executestatement-dbuser

* Parent: `redshift-data:ExecuteStatement`.
* Downstream: `redshift:GetClusterCredentials`.
* Applies when: Temporary credentials for a provisioned cluster as a database user (DbUser supplied).
* Resource: cluster/database user credentials.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_ExecuteStatement.html): Permission to call redshift:GetClusterCredentials is required.

#### redshift-executestatement-secret

* Parent: `redshift-data:ExecuteStatement`.
* Downstream: `secretsmanager:GetSecretValue`.
* Applies when: SecretArn supplied for Secrets Manager database authentication.
* Resource: database credential secret.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_ExecuteStatement.html): Secrets Manager authentication uses the secret containing username and password; the Data API user policy grants secretsmanager:GetSecretValue.
* [AWS source 2](https://docs.aws.amazon.com/redshift/latest/mgmt/data-api-iam.html): Secrets Manager authentication uses the secret containing username and password; the Data API user policy grants secretsmanager:GetSecretValue.

#### redshift-batchexecutestatement-serverless

* Parent: `redshift-data:BatchExecuteStatement`.
* Downstream: `redshift-serverless:GetCredentials`.
* Applies when: Temporary credentials for a serverless workgroup.
* Resource: workgroup credentials.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_BatchExecuteStatement.html): Permission to call redshift-serverless:GetCredentials is required.

#### redshift-batchexecutestatement-iam

* Parent: `redshift-data:BatchExecuteStatement`.
* Downstream: `redshift:GetClusterCredentialsWithIAM`.
* Applies when: Temporary credentials for a provisioned cluster as an IAM identity.
* Resource: cluster credentials.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_BatchExecuteStatement.html): Permission to call redshift:GetClusterCredentialsWithIAM is required.

#### redshift-batchexecutestatement-dbuser

* Parent: `redshift-data:BatchExecuteStatement`.
* Downstream: `redshift:GetClusterCredentials`.
* Applies when: Temporary credentials for a provisioned cluster as a database user (DbUser supplied).
* Resource: cluster/database user credentials.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_BatchExecuteStatement.html): Permission to call redshift:GetClusterCredentials is required.

#### redshift-batchexecutestatement-secret

* Parent: `redshift-data:BatchExecuteStatement`.
* Downstream: `secretsmanager:GetSecretValue`.
* Applies when: SecretArn supplied for Secrets Manager database authentication.
* Resource: database credential secret.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_BatchExecuteStatement.html): Secrets Manager authentication uses the secret containing username and password; the Data API user policy grants secretsmanager:GetSecretValue.
* [AWS source 2](https://docs.aws.amazon.com/redshift/latest/mgmt/data-api-iam.html): Secrets Manager authentication uses the secret containing username and password; the Data API user policy grants secretsmanager:GetSecretValue.

#### redshift-describetable-serverless

* Parent: `redshift-data:DescribeTable`.
* Downstream: `redshift-serverless:GetCredentials`.
* Applies when: Temporary credentials for a serverless workgroup.
* Resource: workgroup credentials.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_DescribeTable.html): Permission to call redshift-serverless:GetCredentials is required.

#### redshift-describetable-iam

* Parent: `redshift-data:DescribeTable`.
* Downstream: `redshift:GetClusterCredentialsWithIAM`.
* Applies when: Temporary credentials for a provisioned cluster as an IAM identity.
* Resource: cluster credentials.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_DescribeTable.html): Permission to call redshift:GetClusterCredentialsWithIAM is required.

#### redshift-describetable-dbuser

* Parent: `redshift-data:DescribeTable`.
* Downstream: `redshift:GetClusterCredentials`.
* Applies when: Temporary credentials for a provisioned cluster as a database user (DbUser supplied).
* Resource: cluster/database user credentials.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_DescribeTable.html): Permission to call redshift:GetClusterCredentials is required.

#### redshift-describetable-secret

* Parent: `redshift-data:DescribeTable`.
* Downstream: `secretsmanager:GetSecretValue`.
* Applies when: SecretArn supplied for Secrets Manager database authentication.
* Resource: database credential secret.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_DescribeTable.html): Secrets Manager authentication uses the secret containing username and password; the Data API user policy grants secretsmanager:GetSecretValue.
* [AWS source 2](https://docs.aws.amazon.com/redshift/latest/mgmt/data-api-iam.html): Secrets Manager authentication uses the secret containing username and password; the Data API user policy grants secretsmanager:GetSecretValue.

#### redshift-listdatabases-serverless

* Parent: `redshift-data:ListDatabases`.
* Downstream: `redshift-serverless:GetCredentials`.
* Applies when: Temporary credentials for a serverless workgroup.
* Resource: workgroup credentials.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_ListDatabases.html): Permission to call redshift-serverless:GetCredentials is required.

#### redshift-listdatabases-iam

* Parent: `redshift-data:ListDatabases`.
* Downstream: `redshift:GetClusterCredentialsWithIAM`.
* Applies when: Temporary credentials for a provisioned cluster as an IAM identity.
* Resource: cluster credentials.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_ListDatabases.html): Permission to call redshift:GetClusterCredentialsWithIAM is required.

#### redshift-listdatabases-dbuser

* Parent: `redshift-data:ListDatabases`.
* Downstream: `redshift:GetClusterCredentials`.
* Applies when: Temporary credentials for a provisioned cluster as a database user (DbUser supplied).
* Resource: cluster/database user credentials.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_ListDatabases.html): Permission to call redshift:GetClusterCredentials is required.

#### redshift-listdatabases-secret

* Parent: `redshift-data:ListDatabases`.
* Downstream: `secretsmanager:GetSecretValue`.
* Applies when: SecretArn supplied for Secrets Manager database authentication.
* Resource: database credential secret.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_ListDatabases.html): Secrets Manager authentication uses the secret containing username and password; the Data API user policy grants secretsmanager:GetSecretValue.
* [AWS source 2](https://docs.aws.amazon.com/redshift/latest/mgmt/data-api-iam.html): Secrets Manager authentication uses the secret containing username and password; the Data API user policy grants secretsmanager:GetSecretValue.

#### redshift-listschemas-serverless

* Parent: `redshift-data:ListSchemas`.
* Downstream: `redshift-serverless:GetCredentials`.
* Applies when: Temporary credentials for a serverless workgroup.
* Resource: workgroup credentials.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_ListSchemas.html): Permission to call redshift-serverless:GetCredentials is required.

#### redshift-listschemas-iam

* Parent: `redshift-data:ListSchemas`.
* Downstream: `redshift:GetClusterCredentialsWithIAM`.
* Applies when: Temporary credentials for a provisioned cluster as an IAM identity.
* Resource: cluster credentials.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_ListSchemas.html): Permission to call redshift:GetClusterCredentialsWithIAM is required.

#### redshift-listschemas-dbuser

* Parent: `redshift-data:ListSchemas`.
* Downstream: `redshift:GetClusterCredentials`.
* Applies when: Temporary credentials for a provisioned cluster as a database user (DbUser supplied).
* Resource: cluster/database user credentials.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_ListSchemas.html): Permission to call redshift:GetClusterCredentials is required.

#### redshift-listschemas-secret

* Parent: `redshift-data:ListSchemas`.
* Downstream: `secretsmanager:GetSecretValue`.
* Applies when: SecretArn supplied for Secrets Manager database authentication.
* Resource: database credential secret.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_ListSchemas.html): Secrets Manager authentication uses the secret containing username and password; the Data API user policy grants secretsmanager:GetSecretValue.
* [AWS source 2](https://docs.aws.amazon.com/redshift/latest/mgmt/data-api-iam.html): Secrets Manager authentication uses the secret containing username and password; the Data API user policy grants secretsmanager:GetSecretValue.

#### redshift-listtables-serverless

* Parent: `redshift-data:ListTables`.
* Downstream: `redshift-serverless:GetCredentials`.
* Applies when: Temporary credentials for a serverless workgroup.
* Resource: workgroup credentials.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_ListTables.html): Permission to call redshift-serverless:GetCredentials is required.

#### redshift-listtables-iam

* Parent: `redshift-data:ListTables`.
* Downstream: `redshift:GetClusterCredentialsWithIAM`.
* Applies when: Temporary credentials for a provisioned cluster as an IAM identity.
* Resource: cluster credentials.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_ListTables.html): Permission to call redshift:GetClusterCredentialsWithIAM is required.

#### redshift-listtables-dbuser

* Parent: `redshift-data:ListTables`.
* Downstream: `redshift:GetClusterCredentials`.
* Applies when: Temporary credentials for a provisioned cluster as a database user (DbUser supplied).
* Resource: cluster/database user credentials.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_ListTables.html): Permission to call redshift:GetClusterCredentials is required.

#### redshift-listtables-secret

* Parent: `redshift-data:ListTables`.
* Downstream: `secretsmanager:GetSecretValue`.
* Applies when: SecretArn supplied for Secrets Manager database authentication.
* Resource: database credential secret.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/redshift-data/latest/APIReference/API_ListTables.html): Secrets Manager authentication uses the secret containing username and password; the Data API user policy grants secretsmanager:GetSecretValue.
* [AWS source 2](https://docs.aws.amazon.com/redshift/latest/mgmt/data-api-iam.html): Secrets Manager authentication uses the secret containing username and password; the Data API user policy grants secretsmanager:GetSecretValue.

#### redshift-data-grant

* Parent: `redshift-data:ExecuteStatement`, `redshift-data:BatchExecuteStatement`.
* Downstream: `kms:CreateGrant`, `kms:DescribeKey`.
* Applies when: Querying a cluster or serverless workgroup encrypted with a customer-managed key; Data API creates a grant for query/result encryption.
* Resource: cluster/workgroup key. Key `${WarehouseKeyId}`, Region `${Region}`.
* KMS service restriction: `redshift.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/redshift/latest/mgmt/data-api-kms.html): ExecuteStatement or BatchExecuteStatement creates a grant on your behalf. The example key policy grants DescribeKey and CreateGrant with kms:ViaService=redshift.amazonaws.com.

### s3

#### s3-putobject

* Parent: `s3:PutObject`.
* Downstream: `kms:GenerateDataKey`.
* Applies when: SSE-KMS or DSSE-KMS destination encryption, explicit or bucket default.
* Resource: destination key. Key `${DestinationKeyId}`, Region `${Region}`.
* KMS service restriction: `s3.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonS3/latest/API/API_PutObject.html): To upload an object encrypted with an AWS KMS key to Amazon S3, you need kms:GenerateDataKey permissions on the key.
* [AWS source 2](https://docs.aws.amazon.com/AmazonS3/latest/userguide/UsingKMSEncryption.html): To upload an object encrypted with an AWS KMS key to Amazon S3, you need kms:GenerateDataKey permissions on the key.

#### s3-getobject

* Parent: `s3:GetObject`.
* Downstream: `kms:Decrypt`.
* Applies when: Reading an SSE-KMS or DSSE-KMS encrypted object.
* Resource: object key. Key `${ObjectKeyId}`, Region `${Region}`.
* KMS service restriction: `s3.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonS3/latest/API/API_GetObject.html): To download an object encrypted with an AWS KMS key, you need kms:Decrypt permissions for the key.
* [AWS source 2](https://docs.aws.amazon.com/AmazonS3/latest/userguide/UsingKMSEncryption.html): To download an object encrypted with an AWS KMS key, you need kms:Decrypt permissions for the key.

#### s3-headobject

* Parent: `s3:HeadObject`.
* Downstream: `kms:Decrypt`.
* Applies when: ChecksumMode=ENABLED on an SSE-KMS object; general-purpose bucket.
* Resource: object key. Key `${ObjectKeyId}`, Region `${Region}`.
* KMS service restriction: `s3.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonS3/latest/API/API_HeadObject.html): General purpose buckets: checksum mode with an AWS KMS key requires kms:Decrypt.
* [AWS source 2](https://docs.aws.amazon.com/AmazonS3/latest/userguide/UsingKMSEncryption.html): General purpose buckets: checksum mode with an AWS KMS key requires kms:Decrypt.

#### s3-createmultipartupload

* Parent: `s3:CreateMultipartUpload`.
* Downstream: `kms:GenerateDataKey`.
* Applies when: Initiating an SSE-KMS multipart upload; explicit or default destination encryption.
* Resource: destination key. Key `${DestinationKeyId}`, Region `${Region}`.
* KMS service restriction: `s3.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonS3/latest/API/API_CreateMultipartUpload.html): The requester must have kms:GenerateDataKey for CreateMultipartUpload, and kms:Decrypt on UploadPart and UploadPartCopy.
* [AWS source 2](https://docs.aws.amazon.com/AmazonS3/latest/userguide/UsingKMSEncryption.html): The requester must have kms:GenerateDataKey for CreateMultipartUpload, and kms:Decrypt on UploadPart and UploadPartCopy.

#### s3-uploadpart

* Parent: `s3:UploadPart`.
* Downstream: `kms:Decrypt`.
* Applies when: Uploading parts of an SSE-KMS encrypted multipart upload.
* Resource: destination key. Key `${DestinationKeyId}`, Region `${Region}`.
* KMS service restriction: `s3.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonS3/latest/API/API_UploadPart.html): The requester must have permission to kms:Decrypt on the key.
* [AWS source 2](https://docs.aws.amazon.com/AmazonS3/latest/userguide/UsingKMSEncryption.html): The requester must have permission to kms:Decrypt on the key.

#### s3-completemultipartupload

* Parent: `s3:CompleteMultipartUpload`.
* Downstream: `kms:Decrypt`.
* Applies when: Completing an SSE-KMS multipart upload, including additional checksum validation.
* Resource: destination key. Key `${DestinationKeyId}`, Region `${Region}`.
* KMS service restriction: `s3.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonS3/latest/API/API_CompleteMultipartUpload.html): Additional checksum with a KMS encrypted object requires kms:Decrypt; multipart upload requires GenerateDataKey and Decrypt.
* [AWS source 2](https://docs.aws.amazon.com/AmazonS3/latest/userguide/UsingKMSEncryption.html): Additional checksum with a KMS encrypted object requires kms:Decrypt; multipart upload requires GenerateDataKey and Decrypt.

#### s3-directory-putobject

* Parent: `s3:PutObject`.
* Downstream: `kms:GenerateDataKey`, `kms:Decrypt`.
* Applies when: Directory bucket uses SSE-KMS.
* Resource: directory bucket key. Key `${DirectoryKeyId}`, Region `${Region}`.
* KMS service restriction: `s3.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonS3/latest/API/API_PutObject.html): If encrypted with SSE-KMS, kms:GenerateDataKey and kms:Decrypt permissions are required.

#### s3-directory-getobject

* Parent: `s3:GetObject`.
* Downstream: `kms:GenerateDataKey`, `kms:Decrypt`.
* Applies when: Directory bucket uses SSE-KMS.
* Resource: directory bucket key. Key `${DirectoryKeyId}`, Region `${Region}`.
* KMS service restriction: `s3.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonS3/latest/API/API_GetObject.html): If encrypted with SSE-KMS, kms:GenerateDataKey and kms:Decrypt permissions are required.

#### s3-directory-headobject

* Parent: `s3:HeadObject`.
* Downstream: `kms:GenerateDataKey`, `kms:Decrypt`.
* Applies when: Directory bucket uses SSE-KMS. ChecksumMode=ENABLED.
* Resource: directory bucket key. Key `${DirectoryKeyId}`, Region `${Region}`.
* KMS service restriction: `s3.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonS3/latest/API/API_HeadObject.html): If encrypted with SSE-KMS, kms:GenerateDataKey and kms:Decrypt permissions are required.

#### s3-directory-getobjectattributes

* Parent: `s3:GetObjectAttributes`.
* Downstream: `kms:GenerateDataKey`, `kms:Decrypt`.
* Applies when: Directory bucket uses SSE-KMS.
* Resource: directory bucket key. Key `${DirectoryKeyId}`, Region `${Region}`.
* KMS service restriction: `s3.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonS3/latest/API/API_GetObjectAttributes.html): If encrypted with SSE-KMS, kms:GenerateDataKey and kms:Decrypt permissions are required.

#### s3-directory-createmultipartupload

* Parent: `s3:CreateMultipartUpload`.
* Downstream: `kms:GenerateDataKey`, `kms:Decrypt`.
* Applies when: Directory bucket uses SSE-KMS.
* Resource: directory bucket key. Key `${DirectoryKeyId}`, Region `${Region}`.
* KMS service restriction: `s3.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonS3/latest/API/API_CreateMultipartUpload.html): If encrypted with SSE-KMS, kms:GenerateDataKey and kms:Decrypt permissions are required.

#### s3-directory-uploadpart

* Parent: `s3:UploadPart`.
* Downstream: `kms:GenerateDataKey`, `kms:Decrypt`.
* Applies when: Directory bucket uses SSE-KMS.
* Resource: directory bucket key. Key `${DirectoryKeyId}`, Region `${Region}`.
* KMS service restriction: `s3.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonS3/latest/API/API_UploadPart.html): If encrypted with SSE-KMS, kms:GenerateDataKey and kms:Decrypt permissions are required.

#### s3-directory-completemultipartupload

* Parent: `s3:CompleteMultipartUpload`.
* Downstream: `kms:GenerateDataKey`, `kms:Decrypt`.
* Applies when: Directory bucket uses SSE-KMS.
* Resource: directory bucket key. Key `${DirectoryKeyId}`, Region `${Region}`.
* KMS service restriction: `s3.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonS3/latest/API/API_CompleteMultipartUpload.html): If encrypted with SSE-KMS, kms:GenerateDataKey and kms:Decrypt permissions are required.

#### s3-directory-createsession

* Parent: `s3:CreateSession`.
* Downstream: `kms:GenerateDataKey`, `kms:Decrypt`.
* Applies when: Directory bucket uses SSE-KMS.
* Resource: directory bucket key. Key `${DirectoryKeyId}`, Region `${Region}`.
* KMS service restriction: `s3.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonS3/latest/API/API_CreateSession.html): If encrypted with SSE-KMS, kms:GenerateDataKey and kms:Decrypt permissions are required.

#### s3-copyobject-source

* Parent: `s3:CopyObject`.
* Downstream: `kms:Decrypt`.
* Applies when: Copy source object is SSE-KMS or DSSE-KMS encrypted.
* Resource: source key. Key `${SourceKeyId}`, Region `${Region}`.
* KMS service restriction: `s3.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonS3/latest/API/API_CopyObject.html): Downloading encrypted objects requires Decrypt; multipart copy requires decryption of encrypted parts.
* [AWS source 2](https://docs.aws.amazon.com/AmazonS3/latest/userguide/UsingKMSEncryption.html): Downloading encrypted objects requires Decrypt; multipart copy requires decryption of encrypted parts.

#### s3-copyobject-destination

* Parent: `s3:CopyObject`.
* Downstream: `kms:GenerateDataKey`.
* Applies when: Copy destination uses SSE-KMS; for multipart copy, directory-bucket permissions explicitly include data-key generation.
* Resource: destination key. Key `${DestinationKeyId}`, Region `${Region}`.
* KMS service restriction: `s3.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonS3/latest/API/API_CopyObject.html): SSE-KMS destination requires GenerateDataKey; directory copy permissions list GenerateDataKey and Decrypt.
* [AWS source 2](https://docs.aws.amazon.com/AmazonS3/latest/userguide/UsingKMSEncryption.html): SSE-KMS destination requires GenerateDataKey; directory copy permissions list GenerateDataKey and Decrypt.

#### s3-copyobject-destination-decrypt

* Parent: `s3:CopyObject`.
* Downstream: `kms:Decrypt`.
* Applies when: Copy destination is a directory bucket using SSE-KMS.
* Resource: destination key. Key `${DestinationKeyId}`, Region `${Region}`.
* KMS service restriction: `s3.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonS3/latest/API/API_CopyObject.html): Directory copy and multipart upload permissions include kms:Decrypt.

#### s3-uploadpartcopy-source

* Parent: `s3:UploadPartCopy`.
* Downstream: `kms:Decrypt`.
* Applies when: Copy source object is SSE-KMS or DSSE-KMS encrypted.
* Resource: source key. Key `${SourceKeyId}`, Region `${Region}`.
* KMS service restriction: `s3.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonS3/latest/API/API_UploadPartCopy.html): Downloading encrypted objects requires Decrypt; multipart copy requires decryption of encrypted parts.
* [AWS source 2](https://docs.aws.amazon.com/AmazonS3/latest/userguide/UsingKMSEncryption.html): Downloading encrypted objects requires Decrypt; multipart copy requires decryption of encrypted parts.

#### s3-uploadpartcopy-destination

* Parent: `s3:UploadPartCopy`.
* Downstream: `kms:GenerateDataKey`.
* Applies when: Multipart copy destination is a directory bucket using SSE-KMS; its API permissions explicitly require data-key generation.
* Resource: destination key. Key `${DestinationKeyId}`, Region `${Region}`.
* KMS service restriction: `s3.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonS3/latest/API/API_UploadPartCopy.html): SSE-KMS destination requires GenerateDataKey; directory copy permissions list GenerateDataKey and Decrypt.
* [AWS source 2](https://docs.aws.amazon.com/AmazonS3/latest/userguide/UsingKMSEncryption.html): SSE-KMS destination requires GenerateDataKey; directory copy permissions list GenerateDataKey and Decrypt.

#### s3-uploadpartcopy-destination-decrypt

* Parent: `s3:UploadPartCopy`.
* Downstream: `kms:Decrypt`.
* Applies when: Destination multipart upload uses SSE-KMS, requiring decryption of encrypted parts for assembly.
* Resource: destination key. Key `${DestinationKeyId}`, Region `${Region}`.
* KMS service restriction: `s3.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AmazonS3/latest/API/API_UploadPartCopy.html): Directory copy and multipart upload permissions include kms:Decrypt.

### secretsmanager

#### secrets-create-key-validation

* Parent: `secretsmanager:CreateSecret`.
* Downstream: `kms:GenerateDataKey`, `kms:Decrypt`.
* Applies when: Customer-managed key is established, including key-access validation without a secret value.
* Resource: secret key. Key `${SecretKeyId}`, Region `${Region}`.
* KMS service restriction: `secretsmanager.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/secretsmanager/latest/userguide/security-encryption.html): When you establish or change the KMS key, Secrets Manager calls GenerateDataKey and Decrypt to validate access.

#### secrets-write-value

* Parent: `secretsmanager:PutSecretValue`.
* Downstream: `kms:GenerateDataKey`.
* Applies when: Writing a secret value encrypted with a customer-managed key.
* Resource: secret key. Key `${SecretKeyId}`, Region `${Region}`.
* KMS service restriction: `secretsmanager.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/secretsmanager/latest/userguide/security-encryption.html): PutSecretValue requests a new data key to encrypt the specified secret value.

#### secrets-idempotency

* Parent: `secretsmanager:PutSecretValue`, `secretsmanager:UpdateSecret`.
* Downstream: `kms:Decrypt`.
* Applies when: Idempotency comparison against an existing version encrypted with a customer-managed key.
* Resource: existing version key. Key `${ExistingKeyId}`, Region `${Region}`.
* KMS service restriction: `secretsmanager.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/secretsmanager/latest/userguide/security-encryption.html): PutSecretValue or UpdateSecret decrypts the existing secret value and compares it to the request.

#### secrets-read

* Parent: `secretsmanager:GetSecretValue`, `secretsmanager:BatchGetSecretValue`.
* Downstream: `kms:Decrypt`.
* Applies when: Reading secret values encrypted with customer-managed keys.
* Resource: secret key. Key `${SecretKeyId}`, Region `${Region}`.
* KMS service restriction: `secretsmanager.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/secretsmanager/latest/userguide/security-encryption.html): GetSecretValue and BatchGetSecretValue decrypt the secret value before returning it.

#### secrets-update-new-key

* Parent: `secretsmanager:UpdateSecret`.
* Downstream: `kms:GenerateDataKey`, `kms:Decrypt`.
* Applies when: Secret value changes or a customer-managed key is established/changed; key validation may occur.
* Resource: new secret key. Key `${NewKeyId}`, Region `${Region}`.
* KMS service restriction: `secretsmanager.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/secretsmanager/latest/userguide/security-encryption.html): UpdateSecret requests a new data key if the value or KMS key changes; key validation uses GenerateDataKey and Decrypt.

#### secrets-update-old-key

* Parent: `secretsmanager:UpdateSecret`.
* Downstream: `kms:Decrypt`.
* Applies when: Changing the encryption key for existing secret versions.
* Resource: old secret key. Key `${OldKeyId}`, Region `${Region}`.
* KMS service restriction: `secretsmanager.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/secretsmanager/latest/userguide/security-encryption.html): Secrets Manager decrypts encrypted data keys before re-encrypting existing versions with the new key.

#### secrets-reencrypt

* Parent: `secretsmanager:UpdateSecret`.
* Downstream: `kms:Encrypt`.
* Applies when: Changing the encryption key for AWSCURRENT, AWSPREVIOUS and AWSPENDING versions.
* Resource: new secret key. Key `${NewKeyId}`, Region `${Region}`.
* KMS service restriction: `secretsmanager.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/secretsmanager/latest/userguide/security-encryption.html): If you change the KMS key, Secrets Manager re-encrypts the data key that protects existing secret versions with the new key.

#### secrets-replicate-primary

* Parent: `secretsmanager:ReplicateSecretToRegions`.
* Downstream: `kms:Decrypt`.
* Applies when: Replication of a secret encrypted with a customer-managed source key.
* Resource: primary Region key. Key `${PrimaryKeyId}`, Region `${PrimaryRegion}`.
* KMS service restriction: `secretsmanager.${PrimaryRegion}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/secretsmanager/latest/userguide/security-encryption.html): Secrets Manager first decrypts the secret value in the primary Region before re-encrypting in the replica Region.

#### secrets-replicate-replica

* Parent: `secretsmanager:ReplicateSecretToRegions`.
* Downstream: `kms:GenerateDataKey`, `kms:Encrypt`, `kms:Decrypt`.
* Applies when: Replica encryption uses a customer-managed key; includes encryption and key-access validation.
* Resource: replica Region key. Key `${ReplicaKeyId}`, Region `${ReplicaRegion}`.
* KMS service restriction: `secretsmanager.${ReplicaRegion}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/secretsmanager/latest/userguide/security-encryption.html): Replication requests a data key in the replica Region; the Monitor interaction section also explicitly describes an Encrypt request during replication.

### sns

#### sns-publish

* Parent: `sns:Publish`, `sns:PublishBatch`.
* Downstream: `kms:GenerateDataKey`, `kms:Decrypt`.
* Applies when: Publishing to an SSE-KMS encrypted topic; does not apply to direct SMS publication.
* Resource: topic key. Key `${TopicKeyId}`, Region `${Region}`.
* KMS service restriction: `sns.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/sns/latest/dg/sns-key-management.html): The publisher must have kms:GenerateDataKey* and kms:Decrypt; PublishBatch uses the same Publish API action.
* [AWS source 2](https://docs.aws.amazon.com/sns/latest/dg/sns-batch-api-actions.html): The publisher must have kms:GenerateDataKey* and kms:Decrypt; PublishBatch uses the same Publish API action.

### sqs

#### sqs-produce

* Parent: `sqs:SendMessage`, `sqs:SendMessageBatch`.
* Downstream: `kms:GenerateDataKey`, `kms:Decrypt`.
* Applies when: Queue uses SSE-KMS; requests after data-key reuse expires validate/generate a key.
* Resource: queue key. Key `${QueueKeyId}`, Region `${Region}`.
* KMS service restriction: `sqs.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/sqs-key-management.html): SendMessage or SendMessageBatch triggers Decrypt and GenerateDataKey after data-key reuse expires.

#### sqs-consume

* Parent: `sqs:ReceiveMessage`.
* Downstream: `kms:Decrypt`.
* Applies when: Receiving SSE-KMS encrypted messages, including messages encrypted with previous queue keys.
* Resource: message key. Key `${MessageKeyId}`, Region `${Region}`.
* KMS service restriction: `sqs.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/sqs-key-management.html): Consumer must have kms:Decrypt for any KMS key used to encrypt messages in the queue.

#### sqs-dead-letter

* Parent: `sqs:ReceiveMessage`.
* Downstream: `kms:Decrypt`.
* Applies when: Receiving messages from a dead-letter queue whose source queue uses a different KMS key.
* Resource: source queue key. Key `${SourceQueueKeyId}`, Region `${Region}`.
* KMS service restriction: `sqs.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/sqs-key-management.html): A dead-letter queue consumer must also have kms:Decrypt for any key used to encrypt messages in the source queue.

#### sqs-redrive-dlq

* Parent: `sqs:StartMessageMoveTask`, `sqs:CancelMessageMoveTask`.
* Downstream: `kms:Decrypt`.
* Applies when: The dead-letter queue has messages encrypted with KMS; applies to starting and cancelling redrive.
* Resource: dead-letter message key. Key `${DeadLetterKeyId}`, Region `${Region}`.
* KMS service restriction: `sqs.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/sqs-configure-dead-letter-queue-redrive.html): Starting redrive requires Decrypt for any key used to encrypt messages; cancelling redrive also requires Decrypt when the dead-letter queue is encrypted.

#### sqs-redrive-original

* Parent: `sqs:StartMessageMoveTask`.
* Downstream: `kms:Decrypt`.
* Applies when: Redriving messages whose original source queue used KMS encryption, possibly with a different or historical key.
* Resource: original source queue key. Key `${OriginalQueueKeyId}`, Region `${Region}`.
* KMS service restriction: `sqs.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/sqs-configure-dead-letter-queue-redrive.html): If either the dead-letter queue or the original source queue is encrypted, Decrypt for any key used to encrypt the messages is required.

#### sqs-redrive-destination

* Parent: `sqs:StartMessageMoveTask`.
* Downstream: `kms:GenerateDataKey`, `kms:Decrypt`.
* Applies when: The destination queue uses SSE-KMS encryption.
* Resource: redrive destination key. Key `${DestinationQueueKeyId}`, Region `${Region}`.
* KMS service restriction: `sqs.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/sqs-configure-dead-letter-queue-redrive.html): If the destination queue is encrypted, GenerateDataKey and Decrypt are required.

### ssm

#### ssm-standard-write

* Parent: `ssm:PutParameter`.
* Downstream: `kms:Encrypt`.
* Applies when: Type=SecureString, standard tier, customer-managed key.
* Resource: parameter key. Key `${ParameterKeyId}`, Region `${Region}`.
* KMS service restriction: `ssm.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/systems-manager/latest/userguide/secure-string-parameter-kms-encryption.html): To encrypt a standard SecureString value, the user needs kms:Encrypt.

#### ssm-advanced-write

* Parent: `ssm:PutParameter`.
* Downstream: `kms:GenerateDataKey`.
* Applies when: Type=SecureString, advanced tier (including Intelligent-Tiering selecting advanced), customer-managed key.
* Resource: parameter key. Key `${ParameterKeyId}`, Region `${Region}`.
* KMS service restriction: `ssm.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/systems-manager/latest/userguide/secure-string-parameter-kms-encryption.html): To encrypt an advanced SecureString value, the user needs kms:GenerateDataKey.

#### ssm-read-getparameter

* Parent: `ssm:GetParameter`.
* Downstream: `kms:Decrypt`.
* Applies when: WithDecryption=true for SecureString values encrypted with customer-managed keys.
* Resource: parameter/version key. Key `${ParameterKeyId}`, Region `${Region}`.
* KMS service restriction: `ssm.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/systems-manager/latest/userguide/secure-string-parameter-kms-encryption.html): WithDecryption returns decrypted SecureString values; decrypting either tier requires kms:Decrypt.
* [AWS source 2](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_GetParameter.html): WithDecryption returns decrypted SecureString values; decrypting either tier requires kms:Decrypt.

#### ssm-read-getparameters

* Parent: `ssm:GetParameters`.
* Downstream: `kms:Decrypt`.
* Applies when: WithDecryption=true for SecureString values encrypted with customer-managed keys.
* Resource: parameter/version key. Key `${ParameterKeyId}`, Region `${Region}`.
* KMS service restriction: `ssm.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/systems-manager/latest/userguide/secure-string-parameter-kms-encryption.html): WithDecryption returns decrypted SecureString values; decrypting either tier requires kms:Decrypt.
* [AWS source 2](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_GetParameters.html): WithDecryption returns decrypted SecureString values; decrypting either tier requires kms:Decrypt.

#### ssm-read-getparametersbypath

* Parent: `ssm:GetParametersByPath`.
* Downstream: `kms:Decrypt`.
* Applies when: WithDecryption=true for SecureString values encrypted with customer-managed keys.
* Resource: parameter/version key. Key `${ParameterKeyId}`, Region `${Region}`.
* KMS service restriction: `ssm.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/systems-manager/latest/userguide/secure-string-parameter-kms-encryption.html): WithDecryption returns decrypted SecureString values; decrypting either tier requires kms:Decrypt.
* [AWS source 2](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_GetParametersByPath.html): WithDecryption returns decrypted SecureString values; decrypting either tier requires kms:Decrypt.

#### ssm-read-getparameterhistory

* Parent: `ssm:GetParameterHistory`.
* Downstream: `kms:Decrypt`.
* Applies when: WithDecryption=true for SecureString values encrypted with customer-managed keys.
* Resource: parameter/version key. Key `${ParameterKeyId}`, Region `${Region}`.
* KMS service restriction: `ssm.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/systems-manager/latest/userguide/secure-string-parameter-kms-encryption.html): WithDecryption returns decrypted SecureString values; decrypting either tier requires kms:Decrypt.
* [AWS source 2](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_GetParameterHistory.html): WithDecryption returns decrypted SecureString values; decrypting either tier requires kms:Decrypt.

#### ssm-secrets-reference

* Parent: `ssm:GetParameter`, `ssm:GetParameters`.
* Downstream: `secretsmanager:GetSecretValue`.
* Applies when: Name has /aws/reference/secretsmanager/ prefix; Parameter Store honors the caller's secret permissions.
* Resource: referenced secret.
* KMS service restriction: none; credential and secret permissions use their service resource scopes.
* [AWS source 1](https://docs.aws.amazon.com/systems-manager/latest/userguide/integration-ps-secretsmanager.html): Only GetParameter and GetParameters support secret references. Parameter Store functions as a pass-through service.

### states

#### states-configure

* Parent: `states:CreateStateMachine`, `states:UpdateStateMachine`.
* Downstream: `kms:DescribeKey`, `kms:GenerateDataKey`.
* Applies when: State machine encryption uses a customer-managed KMS key.
* Resource: state machine key. Key `${StateMachineKeyId}`, Region `${Region}`.
* KMS service restriction: `states.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/step-functions/latest/dg/encryption-at-rest.html): The Required permissions for API callers table lists these actions. The service calls KMS on behalf of the API caller; execution-role calls are separate.

#### states-describe

* Parent: `states:DescribeStateMachine`, `states:DescribeStateMachineForExecution`.
* Downstream: `kms:Decrypt`.
* Applies when: Customer-managed key encryption and includedData requests encrypted data rather than METADATA_ONLY.
* Resource: state machine key. Key `${StateMachineKeyId}`, Region `${Region}`.
* KMS service restriction: `states.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/step-functions/latest/dg/encryption-at-rest.html): The Required permissions for API callers table lists these actions. The service calls KMS on behalf of the API caller; execution-role calls are separate.

#### states-execution-data

* Parent: `states:DescribeExecution`, `states:GetExecutionHistory`.
* Downstream: `kms:Decrypt`.
* Applies when: Retrieving encrypted execution data; metadata-only/excluded execution-data requests may avoid KMS permissions.
* Resource: state machine key. Key `${StateMachineKeyId}`, Region `${Region}`.
* KMS service restriction: `states.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/step-functions/latest/dg/encryption-at-rest.html): The Required permissions for API callers table lists these actions. The service calls KMS on behalf of the API caller; execution-role calls are separate.

#### states-sync

* Parent: `states:StartSyncExecution`.
* Downstream: `kms:Decrypt`.
* Applies when: Synchronous Express workflow uses a customer-managed encryption key and returns execution data.
* Resource: state machine key. Key `${StateMachineKeyId}`, Region `${Region}`.
* KMS service restriction: `states.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/step-functions/latest/dg/encryption-at-rest.html): The Required permissions for API callers table lists these actions. The service calls KMS on behalf of the API caller; execution-role calls are separate.

#### states-activity-create

* Parent: `states:CreateActivity`.
* Downstream: `kms:DescribeKey`.
* Applies when: Activity encryption uses a customer-managed key.
* Resource: activity key. Key `${ActivityKeyId}`, Region `${Region}`.
* KMS service restriction: `states.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/step-functions/latest/dg/encryption-at-rest.html): The Required permissions for API callers table lists these actions. The service calls KMS on behalf of the API caller; execution-role calls are separate.

#### states-activity-task

* Parent: `states:GetActivityTask`.
* Downstream: `kms:Decrypt`.
* Applies when: Retrieving task data from an activity encrypted with a customer-managed key.
* Resource: activity key. Key `${ActivityKeyId}`, Region `${Region}`.
* KMS service restriction: `states.${Region}.amazonaws.com`.
* [AWS source 1](https://docs.aws.amazon.com/step-functions/latest/dg/encryption-at-rest.html): The Required permissions for API callers table lists these actions. The service calls KMS on behalf of the API caller; execution-role calls are separate.

## Reviewed exclusions

* `secretsmanager:DescribeSecret`, `secretsmanager:ListSecrets`, `secretsmanager:ListSecretVersionIds`, `secretsmanager:DeleteSecret`, `secretsmanager:TagResource`, `secretsmanager:UntagResource`, `secretsmanager:RotateSecret`. Cryptographic permission section enumerates value/key operations. RotateSecret itself does not change a value; a rotation Lambda uses its own role. [AWS source](https://docs.aws.amazon.com/secretsmanager/latest/userguide/security-encryption.html).

* `s3:DeleteObject`, `s3:DeleteObjects`, `s3:ListObjects`, `s3:ListObjectsV2`, `s3:AbortMultipartUpload`. KMS permissions are for encrypted object data access and upload, not deleting/listing objects or aborting uploads. [AWS source](https://docs.aws.amazon.com/AmazonS3/latest/userguide/UsingKMSEncryption.html).

* `states:StartExecution`, `states:SendTaskSuccess`, `states:SendTaskFailure`, `states:StopExecution`, `states:RedriveExecution`. The Required permissions for API callers table shows no caller KMS permission for these APIs. The state machine execution role supplies the execution permissions. [AWS source](https://docs.aws.amazon.com/step-functions/latest/dg/encryption-at-rest.html).

* `ecr:PutImage`, `ecr:UploadLayerPart`, `ecr:CompleteLayerUpload`, `ecr:InitiateLayerUpload`, `ecr:BatchGetImage`, `ecr:GetDownloadUrlForLayer`. ECR creates grants at repository creation to authorize image encryption/decryption; do not add those grant-authorized internal cryptographic calls to image callers. [AWS source](https://docs.aws.amazon.com/AmazonECR/latest/userguide/encryption-at-rest.html).

## Maintenance and validation

Add or change rules in the JSON document, update the corresponding audit entries
here, and record the review date. Cite operation-specific API permission sections
where available, otherwise the service's caller-permission documentation. Verify
who authorizes each downstream call; a service's use of KMS alone is insufficient.
Do not infer dependency graphs from broad managed policies, the general KMS
integration table, or a third-party mapping catalog. Use those only to locate
primary documentation for a subsequent audit.

Run `make check`, provider/CLI/site-data tests, and the site catalog tests after
changes. Regression coverage checks conditional S3 rules, old/new and replica key
scopes, negative metadata/role cases, transitive secret references, direct plus FAS
KMS grants, HCL/JSON parity, catalog validation, deterministic output, and site
provenance. These static checks establish catalog consistency. Live AWS access,
CloudTrail, or AccessDenied evidence is still needed to validate a deployment's
actual key policies, grants, resource configuration, and options.
