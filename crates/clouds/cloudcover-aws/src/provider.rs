use cloudcover_core::{
    ApiMethod, CloudProvider, GoMethodReference, JavaScriptMethodReference, Language,
    MethodReference, PythonMethodReference, ResolvedSdk, Sdk, SdkMethodMapping,
    TerraformMethodReference,
};
use std::{collections::BTreeMap, sync::OnceLock};

use crate::{generated, policy};

#[derive(Clone, Copy, Debug, Default)]
pub struct AwsProvider;

impl AwsProvider {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
    /// Returns direct IAM permissions plus documented possible FAS permissions.
    ///
    /// This action-only projection omits conditions and applicability. Use
    /// `fas_dependencies` for evidence and `permissions_policy` for scoped grants.
    ///
    /// # Errors
    ///
    /// Returns [`AwsError::UnknownApiMethod`] when the operation or one of its
    /// generated authorized-action references is unknown.
    pub fn iam_permissions(&self, method: &ApiMethod) -> Result<Vec<&'static str>, AwsError> {
        let mut permissions = policy::resolve_actions(method)?
            .into_iter()
            .map(|action| action.permission)
            .collect::<Vec<_>>();
        permissions.sort_unstable();
        permissions.dedup();
        Ok(permissions)
    }

    /// Returns conditional downstream permissions with primary sources and parent paths.
    ///
    /// # Errors
    /// Returns an error for an unknown operation or an invalid supplement reference.
    pub fn fas_dependencies(
        &self,
        method: &ApiMethod,
    ) -> Result<Vec<crate::fas::FasDependency>, AwsError> {
        crate::fas::dependencies(method)
    }

    /// Returns the supplement's review coverage for this operation.
    ///
    /// # Errors
    /// Returns an error for an unknown operation or an invalid catalog.
    pub fn fas_coverage(&self, method: &ApiMethod) -> Result<crate::fas::FasCoverage, AwsError> {
        crate::fas::coverage(method)
    }

    /// Returns the IAM permissions policy as a Terraform HCL data source.
    ///
    /// # Errors
    ///
    /// Returns an error when the provider cannot map every requested API method
    /// to a permissions policy entry or when the HCL cannot be formatted.
    pub fn permissions_policy_hcl(&self, methods: &[ApiMethod]) -> Result<String, AwsError> {
        policy::build_permissions_policy_hcl(methods)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AwsError {
    #[error("invalid FAS catalog: {message}")]
    InvalidFasCatalog { message: String },
    #[error("unknown AWS API method {service}:{name}")]
    UnknownApiMethod { service: String, name: String },
    #[error("failed to encode IAM policy as Terraform HCL: {source}")]
    HclSerialization { source: hcl::Error },
    #[error("unsupported SDK {name:?} for language {language:?}")]
    UnsupportedSdk { name: String, language: Language },
    #[error("SDK {name:?} requires a version")]
    MissingSdkVersion { name: String },
    #[error("Terraform provider AWS version {version:?} is unsupported: {reason}")]
    UnsupportedTerraformProviderAwsVersion {
        version: String,
        reason: cloudcover_terraform_provider_aws::TerraformProviderAwsUnsupportedReason,
    },
    #[error("unsupported version {version:?} for SDK {name:?}")]
    UnsupportedSdkVersion { name: String, version: String },
    #[error("unsupported AWS Go SDK service module {path:?}")]
    UnsupportedSdkModule { path: String },
    #[error("AWS Go SDK service module {path:?} requires a version")]
    MissingSdkModuleVersion { path: String },
    #[error("unsupported version {version:?} for AWS Go SDK service module {path:?}")]
    UnsupportedSdkModuleVersion { path: String, version: String },
    #[error(
        "AWS Go SDK service module {path:?} uses unsupported replacement {replacement_path:?} {replacement_version:?}"
    )]
    UnsupportedSdkModuleReplacement {
        path: String,
        replacement_path: String,
        replacement_version: Option<String>,
    },
    #[error("unsupported AWS SDK for JavaScript package {path:?}")]
    UnsupportedJsSdkPackage { path: String },
    #[error("AWS SDK for JavaScript package {path:?} requires a version")]
    MissingJsSdkPackageVersion { path: String },
    #[error("unsupported version {version:?} for AWS SDK for JavaScript package {path:?}")]
    UnsupportedJsSdkPackageVersion { path: String, version: String },
}

impl CloudProvider for AwsProvider {
    type Error = AwsError;

    fn list_api_methods(&self) -> Vec<ApiMethod> {
        generated::OPERATIONS
            .iter()
            .map(|operation| ApiMethod::new(operation.service, operation.name))
            .collect()
    }

    fn list_sdks(&self) -> Vec<Sdk> {
        let mut sdks = vec![
            Sdk::new(cloudcover_aws_sdk_go_v2::SDK_NAME, Language::Go),
            Sdk::new(cloudcover_aws_sdk_go_v1::SDK_NAME, Language::Go),
            Sdk::new("boto3", Language::Python),
            Sdk::new(cloudcover_aws_sdk_js_v3::SDK_NAME, Language::JavaScript),
            Sdk::new(cloudcover_aws_sdk_js_v3::SDK_NAME, Language::TypeScript),
        ];
        sdks.extend(
            cloudcover_terraform_provider_aws::provider_versions()
                .iter()
                .copied()
                .map(|version| {
                    Sdk::new(
                        cloudcover_terraform_provider_aws::SDK_NAME,
                        Language::Terraform,
                    )
                    .with_version(version)
                }),
        );
        sdks
    }

    fn sdk_method_mappings(
        &self,
        resolved_sdk: &ResolvedSdk,
    ) -> Result<Vec<SdkMethodMapping>, Self::Error> {
        let sdk = resolved_sdk.sdk();
        if sdk.name() == cloudcover_aws_sdk_go_v1::SDK_NAME && sdk.language() == Language::Go {
            return go_sdk_v1_method_mappings(resolved_sdk);
        }
        if sdk.name() == cloudcover_aws_sdk_go_v2::SDK_NAME && sdk.language() == Language::Go {
            return go_sdk_method_mappings(resolved_sdk);
        }
        if sdk.name() == cloudcover_aws_sdk_js_v3::SDK_NAME
            && matches!(sdk.language(), Language::JavaScript | Language::TypeScript)
        {
            return javascript_sdk_method_mappings(resolved_sdk);
        }
        if sdk.name() == "boto3" && sdk.language() == Language::Python {
            return match sdk.version() {
                None => Ok(python_sdk_method_mappings()),
                Some(version) => Err(AwsError::UnsupportedSdkVersion {
                    name: sdk.name().to_owned(),
                    version: version.to_owned(),
                }),
            };
        }
        if sdk.name() == cloudcover_terraform_provider_aws::SDK_NAME
            && sdk.language() == Language::Terraform
        {
            let Some(version) = sdk.version() else {
                return Err(AwsError::MissingSdkVersion {
                    name: sdk.name().to_owned(),
                });
            };
            return terraform_provider_aws_sdk_method_mappings(version);
        }
        Err(AwsError::UnsupportedSdk {
            name: sdk.name().to_owned(),
            language: sdk.language(),
        })
    }

    fn permissions_policy(&self, methods: &[ApiMethod]) -> Result<serde_json::Value, Self::Error> {
        policy::build_permissions_policy(methods)
    }
}

fn python_sdk_method_mappings() -> Vec<SdkMethodMapping> {
    generated::SDK_METHOD_MAPPINGS
        .iter()
        .map(|row| {
            SdkMethodMapping::new(
                Sdk::new(row.sdk_package, Language::Python),
                MethodReference::Python(PythonMethodReference::new(
                    "boto3",
                    Some(row.sdk_name.to_owned()),
                    row.sdk_method,
                )),
                vec![ApiMethod::new(row.api_service, row.api_name)],
            )
        })
        .collect()
}

fn go_sdk_method_mappings(resolved_sdk: &ResolvedSdk) -> Result<Vec<SdkMethodMapping>, AwsError> {
    const SERVICE_PREFIX: &str = "github.com/aws/aws-sdk-go-v2/service/";

    let sdk = resolved_sdk.sdk();
    if let Some(version) = sdk.version() {
        return Err(AwsError::UnsupportedSdkVersion {
            name: sdk.name().to_owned(),
            version: version.to_owned(),
        });
    }

    let mut mappings = Vec::new();
    for module in resolved_sdk.modules() {
        let Some(service) = module.path().strip_prefix(SERVICE_PREFIX) else {
            continue;
        };
        if service.contains('/') {
            continue;
        }
        if !cloudcover_aws_sdk_go_v2::service_modules().contains(&module.path()) {
            return Err(AwsError::UnsupportedSdkModule {
                path: module.path().to_owned(),
            });
        }
        if let Some(replacement) = module.replacement() {
            return Err(AwsError::UnsupportedSdkModuleReplacement {
                path: module.path().to_owned(),
                replacement_path: replacement.path().to_owned(),
                replacement_version: replacement.version().map(str::to_owned),
            });
        }
        if module.version().is_empty() {
            return Err(AwsError::MissingSdkModuleVersion {
                path: module.path().to_owned(),
            });
        }
        let Some(version) = module.version().strip_prefix('v') else {
            return Err(AwsError::UnsupportedSdkModuleVersion {
                path: module.path().to_owned(),
                version: module.version().to_owned(),
            });
        };
        let Some(rows) = cloudcover_aws_sdk_go_v2::service_method_mappings(module.path(), version)
        else {
            return Err(AwsError::UnsupportedSdkModuleVersion {
                path: module.path().to_owned(),
                version: module.version().to_owned(),
            });
        };

        mappings.extend(rows.filter_map(|row| {
            let api_methods = row
                .api_methods
                .iter()
                .filter_map(|api_method| {
                    catalog_sdk_api_method(row.package, api_method.service, api_method.name)
                })
                .collect::<Vec<_>>();
            if api_methods.is_empty() {
                return None;
            }

            Some(SdkMethodMapping::new(
                sdk.clone(),
                MethodReference::Go(GoMethodReference::new(
                    row.package,
                    Some(row.receiver.to_owned()),
                    row.method,
                )),
                api_methods,
            ))
        }));
    }
    mappings.sort();
    mappings.dedup();
    Ok(mappings)
}

fn go_sdk_v1_method_mappings(
    resolved_sdk: &ResolvedSdk,
) -> Result<Vec<SdkMethodMapping>, AwsError> {
    let sdk = resolved_sdk.sdk();
    if let Some(version) = sdk.version() {
        return Err(AwsError::UnsupportedSdkVersion {
            name: sdk.name().to_owned(),
            version: version.to_owned(),
        });
    }

    let mut mappings = Vec::new();
    for module in resolved_sdk.modules() {
        if module.path() != cloudcover_aws_sdk_go_v1::MODULE_PATH {
            continue;
        }
        if let Some(replacement) = module.replacement() {
            return Err(AwsError::UnsupportedSdkModuleReplacement {
                path: module.path().to_owned(),
                replacement_path: replacement.path().to_owned(),
                replacement_version: replacement.version().map(str::to_owned),
            });
        }
        if module.version().is_empty() {
            return Err(AwsError::MissingSdkModuleVersion {
                path: module.path().to_owned(),
            });
        }
        let Some(version) = module.version().strip_prefix('v') else {
            return Err(AwsError::UnsupportedSdkModuleVersion {
                path: module.path().to_owned(),
                version: module.version().to_owned(),
            });
        };
        let Some(rows) = cloudcover_aws_sdk_go_v1::sdk_method_mappings(version) else {
            return Err(AwsError::UnsupportedSdkModuleVersion {
                path: module.path().to_owned(),
                version: module.version().to_owned(),
            });
        };
        mappings.extend(rows.filter_map(|row| {
            let api_methods = row
                .api_methods
                .iter()
                .filter_map(|api_method| {
                    catalog_sdk_api_method(row.package, api_method.service, api_method.name)
                })
                .collect::<Vec<_>>();
            if api_methods.is_empty() {
                return None;
            }
            Some(SdkMethodMapping::new(
                sdk.clone(),
                MethodReference::Go(GoMethodReference::new(
                    row.package,
                    Some(row.receiver.to_owned()),
                    row.method,
                )),
                api_methods,
            ))
        }));
    }
    mappings.sort();
    mappings.dedup();
    Ok(mappings)
}

fn javascript_sdk_method_mappings(
    resolved_sdk: &ResolvedSdk,
) -> Result<Vec<SdkMethodMapping>, AwsError> {
    let sdk = resolved_sdk.sdk();
    if let Some(version) = sdk.version() {
        return Err(AwsError::UnsupportedSdkVersion {
            name: sdk.name().to_owned(),
            version: version.to_owned(),
        });
    }

    let mut mappings = Vec::new();
    for module in resolved_sdk.modules() {
        if !cloudcover_aws_sdk_js_v3::service_modules().contains(&module.path()) {
            return Err(AwsError::UnsupportedJsSdkPackage {
                path: module.path().to_owned(),
            });
        }
        if module.replacement().is_some() {
            return Err(AwsError::UnsupportedJsSdkPackage {
                path: module.path().to_owned(),
            });
        }
        if module.version().is_empty() {
            return Err(AwsError::MissingJsSdkPackageVersion {
                path: module.path().to_owned(),
            });
        }
        let Some(rows) =
            cloudcover_aws_sdk_js_v3::service_method_mappings(module.path(), module.version())
        else {
            return Err(AwsError::UnsupportedJsSdkPackageVersion {
                path: module.path().to_owned(),
                version: module.version().to_owned(),
            });
        };
        mappings.extend(rows.iter().filter_map(|row| {
            let api_methods = row
                .api_methods
                .iter()
                .filter_map(|api_method| {
                    catalog_sdk_api_method(row.package, api_method.service, api_method.name)
                })
                .collect::<Vec<_>>();
            if api_methods.is_empty() {
                return None;
            }
            Some(SdkMethodMapping::new(
                sdk.clone(),
                MethodReference::JavaScript(JavaScriptMethodReference::new(
                    row.package,
                    row.receiver.map(str::to_owned),
                    row.method,
                )),
                api_methods,
            ))
        }));
    }
    mappings.sort();
    mappings.dedup();
    Ok(mappings)
}

fn terraform_provider_aws_sdk_method_mappings(
    version: &str,
) -> Result<Vec<SdkMethodMapping>, AwsError> {
    let rows = match cloudcover_terraform_provider_aws::sdk_method_mappings(version) {
        cloudcover_terraform_provider_aws::TerraformProviderAwsMappingsLookup::Supported(rows) => {
            rows
        }
        cloudcover_terraform_provider_aws::TerraformProviderAwsMappingsLookup::Unsupported(
            reason,
        ) => {
            return Err(AwsError::UnsupportedTerraformProviderAwsVersion {
                version: version.to_owned(),
                reason,
            });
        }
        cloudcover_terraform_provider_aws::TerraformProviderAwsMappingsLookup::Unknown => {
            return Err(AwsError::UnsupportedSdkVersion {
                name: cloudcover_terraform_provider_aws::SDK_NAME.to_owned(),
                version: version.to_owned(),
            });
        }
    };

    Ok(rows
        .iter()
        .map(|row| {
            let api_methods = row
                .api_methods
                .iter()
                .filter_map(|api_method| catalog_api_method(api_method.service, api_method.name))
                .collect::<Vec<_>>();

            SdkMethodMapping::new(
                Sdk::new(
                    cloudcover_terraform_provider_aws::SDK_NAME,
                    Language::Terraform,
                )
                .with_version(version),
                MethodReference::Terraform(TerraformMethodReference::new(
                    row.kind,
                    row.type_name,
                    row.action,
                )),
                api_methods,
            )
        })
        .collect())
}

fn catalog_api_method(service: &str, name: &str) -> Option<ApiMethod> {
    let exists = |service| {
        generated::OPERATIONS
            .binary_search_by(|operation| (operation.service, operation.name).cmp(&(service, name)))
            .is_ok()
    };
    if exists(service) {
        return Some(ApiMethod::new(service, name));
    }
    let identifier = sdk_service_identifier(service);
    if exists(&identifier) {
        return Some(ApiMethod::new(identifier, name));
    }
    if let Some(canonical) = catalog_service_alias(&identifier)
        && exists(canonical)
    {
        return Some(ApiMethod::new(canonical, name));
    }
    let canonical = catalog_sdk_aliases()
        .get(&(identifier, name))
        .copied()
        .flatten()?;
    Some(ApiMethod::new(canonical, name))
}

fn catalog_sdk_api_method(package: &str, service: &str, name: &str) -> Option<ApiMethod> {
    catalog_api_method(service, name).or_else(|| {
        // A signing name can be shared by different clients (for example
        // OpenSearch and Elasticsearch both sign with "es"). The SDK package
        // retains the client identity when its signing name cannot be resolved.
        let client = package
            .strip_prefix("github.com/aws/aws-sdk-go-v2/service/")
            .or_else(|| package.strip_prefix("github.com/aws/aws-sdk-go/service/"))
            .or_else(|| package.strip_prefix("@aws-sdk/client-"))?;
        if client.contains('/') {
            return None;
        }
        catalog_api_method(client, name)
    })
}

fn sdk_service_identifier(service: &str) -> String {
    service
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

// AWS publishes the SDK service identifier for each operation. Use that
// explicit relationship for runtime clients and older SDK display names;
// matching an operation name across unrelated services is ambiguous.
type CatalogAliases = BTreeMap<(String, &'static str), Option<&'static str>>;

fn catalog_sdk_aliases() -> &'static CatalogAliases {
    static ALIASES: OnceLock<CatalogAliases> = OnceLock::new();
    ALIASES.get_or_init(|| {
        let mut aliases = BTreeMap::new();
        for row in generated::SDK_METHOD_MAPPINGS {
            let key = (sdk_service_identifier(row.sdk_name), row.api_name);
            aliases
                .entry(key)
                .and_modify(|canonical| {
                    if *canonical != Some(row.api_service) {
                        *canonical = None;
                    }
                })
                .or_insert(Some(row.api_service));
        }
        aliases
    })
}

// SDK service identifiers and Service Authorization Reference identifiers differ.
// Only translate known services; operation names alone do not identify a service.
fn catalog_service_alias(service: &str) -> Option<&'static str> {
    Some(match service {
        "accessanalyzer" => "access-analyzer",
        "accountaccess" => "account-access",
        "acmpca" => "acm-pca",
        "agentregistrycontrol" => "agent-registry",
        "amp" | "prometheusservice" => "aps",
        "apigatewayv2" => "apigateway",
        "appintegrations" | "appintegrationsservice" => "app-integrations",
        "applicationautoscaling" => "application-autoscaling",
        "applicationdiscoveryservice" => "discovery",
        "arcregionswitch" => "arc-region-switch",
        "arczonalshift" => "arc-zonal-shift",
        "autoscalingplans" => "autoscaling-plans",
        "bcmdataexports" => "bcm-data-exports",
        "bedrockagent" | "bedrockruntime" => "bedrock",
        "bedrockagentcorecontrol" => "bedrock-agentcore",
        "chimesdkmediapipelines" | "chimesdkvoice" => "chime",
        "cloudcontrol" | "cloudcontrolapi" => "cloudformation",
        "cloudfrontkeyvaluestore" => "cloudfront-keyvaluestore",
        "cloudhsmv2" => "cloudhsm",
        "cloudwatchevents" | "eventbridge" => "events",
        "cloudwatchevidently" => "evidently",
        "cloudwatchlogs" => "logs",
        "cloudwatchrum" => "rum",
        "codeguruprofiler" => "codeguru-profiler",
        "codegurureviewer" => "codeguru-reviewer",
        "codestarconnections" => "codestar-connections",
        "codestarnotifications" => "codestar-notifications",
        "cognitoidentity" => "cognito-identity",
        "cognitoidentityprovider" => "cognito-idp",
        "commander" | "ssmincidents" => "ssm-incidents",
        "computeoptimizer" => "compute-optimizer",
        "configservice" => "config",
        "costandusagereportservice" => "cur",
        "costexplorer" => "ce",
        "costoptimizationhub" => "cost-optimization-hub",
        "customerprofiles" => "profile",
        "databasemigrationservice" => "dms",
        "devopsguru" => "devops-guru",
        "directoryservice" => "ds",
        "directoryservicedata" => "ds-data",
        "docdb" | "neptune" => "rds",
        "docdbelastic" => "docdb-elastic",
        "ecrpublic" => "ecr-public",
        "efs" => "elasticfilesystem",
        "elasticloadbalancingv2" | "elbv2" | "elb" => "elasticloadbalancing",
        "elasticsearchservice" => "es",
        "emr" => "elasticmapreduce",
        "emrcontainers" => "emr-containers",
        "emrserverless" => "emr-serverless",
        "iotdataplane" => "iot",
        "iotjobsdataplane" => "iotjobsdata",
        "keyspaces" => "cassandra",
        "kinesisanalyticsv2" => "kinesisanalytics",
        "lambdacore" | "lambdamicrovms" => "lambda",
        "lexmodelbuildingservice" | "lexmodelsv2" | "lexruntimeservice" | "lexruntimev2" => "lex",
        "licensemanager" => "license-manager",
        "location" | "locationservice" => "geo",
        "mailmanager" | "sesv2" => "ses",
        "managedgrafana" => "grafana",
        "marketplaceentitlementservice" | "marketplacemetering" => "aws-marketplace",
        "mechanicalturkrequesterservice" => "mechanicalturk",
        "migrationhub" => "mgh",
        "mwaa" => "airflow",
        "neptunegraph" => "neptune-graph",
        "networkfirewall" => "network-firewall",
        "notificationscontacts" => "notifications-contacts",
        "opensearchserverless" => "aoss",
        "opensearchservice" => "opensearch",
        "paymentcryptography" => "payment-cryptography",
        "pinpoint" => "mobiletargeting",
        "pinpointsmsvoicev2" => "sms-voice",
        "redshiftdata" | "redshiftdataapiservice" => "redshift-data",
        "redshiftserverless" => "redshift-serverless",
        "resiliencehubv2" => "resiliencehub",
        "resourceexplorer2" => "resource-explorer-2",
        "resourcegroups" => "resource-groups",
        "resourcegroupstaggingapi" => "tag",
        "route53recoverycontrolconfig" => "route53-recovery-control-config",
        "route53recoveryreadiness" => "route53-recovery-readiness",
        "s3control" => "s3",
        "s3outposts" => "s3-outposts",
        "serverlessapplicationrepository" => "serverlessrepo",
        "servicecatalogappregistry" => "servicecatalog",
        "simpledb" => "sdb",
        "ssmcontacts" => "ssm-contacts",
        "ssmquicksetup" => "ssm-quicksetup",
        "ssoadmin" => "sso",
        "sfn" => "states",
        "timestreaminfluxdb" => "timestream-influxdb",
        "timestreamquery" | "timestreamwrite" => "timestream",
        "transcribeservice" => "transcribe",
        "vpclattice" => "vpc-lattice",
        "wafregional" => "waf-regional",
        "workspacesweb" => "workspaces-web",
        _ => return None,
    })
}

#[cfg(test)]
mod alias_tests {
    use super::{catalog_api_method, catalog_sdk_api_method, generated};
    use cloudcover_core::ApiMethod;

    #[test]
    fn resolves_runtime_and_historical_sdk_service_identifiers() {
        for (service, name, canonical) in [
            ("ACM PCA", "CreateCertificateAuthority", "acm-pca"),
            ("Bedrock Runtime", "InvokeModel", "bedrock"),
            ("bedrockruntime", "ApplyGuardrail", "bedrock"),
            ("directoryservicedata", "CreateUser", "ds-data"),
            ("SageMaker Runtime", "InvokeEndpoint", "sagemaker"),
            (
                "paymentcryptographydata",
                "EncryptData",
                "payment-cryptography",
            ),
            ("Lex Models V2", "CreateBot", "lex"),
            ("CloudWatch Logs", "AssociateKmsKey", "logs"),
            ("SFN", "CreateStateMachine", "states"),
            ("Commander", "CreateResponsePlan", "ssm-incidents"),
            (
                "Elastic Load Balancing",
                "CreateLoadBalancer",
                "elasticloadbalancing",
            ),
            (
                "Application Discovery Service",
                "CreateApplication",
                "discovery",
            ),
            (
                "MechanicalTurkRequesterService",
                "CreateHIT",
                "mechanicalturk",
            ),
            ("Migration Hub", "AssociateCreatedArtifact", "mgh"),
            ("IoT Data Plane", "Publish", "iot"),
            (
                "IoT Jobs Data Plane",
                "GetPendingJobExecutions",
                "iotjobsdata",
            ),
            ("Lex Runtime Service", "PostText", "lex"),
            ("Lex Runtime V2", "RecognizeText", "lex"),
            ("Marketplace Metering", "MeterUsage", "aws-marketplace"),
            (
                "Marketplace Entitlement Service",
                "GetEntitlements",
                "aws-marketplace",
            ),
            ("transcribeservice", "StartTranscriptionJob", "transcribe"),
        ] {
            assert_eq!(
                catalog_api_method(service, name),
                Some(ApiMethod::new(canonical, name))
            );
        }
        assert_eq!(catalog_api_method("unrelated", "CreateUser"), None);
        assert_eq!(catalog_api_method("bedrock-runtime", "CreateUser"), None);
    }

    #[test]
    fn every_published_sdk_alias_resolves_to_its_catalog_operation() {
        for row in generated::SDK_METHOD_MAPPINGS {
            assert_eq!(
                catalog_api_method(row.sdk_name, row.api_name),
                Some(ApiMethod::new(row.api_service, row.api_name)),
                "{}:{}",
                row.sdk_name,
                row.api_name,
            );
        }
    }

    #[test]
    fn resolves_shared_signing_names_through_known_sdk_packages() {
        for (package, signer, name, canonical) in [
            (
                "github.com/aws/aws-sdk-go/service/opensearchservice",
                "es",
                "CreateDomain",
                "opensearch",
            ),
            (
                "github.com/aws/aws-sdk-go-v2/service/opensearch",
                "es",
                "CreateDomain",
                "opensearch",
            ),
            (
                "@aws-sdk/client-opensearch",
                "es",
                "CreateDomain",
                "opensearch",
            ),
            (
                "github.com/aws/aws-sdk-go/service/resourcegroupstaggingapi",
                "tagging",
                "GetResources",
                "tag",
            ),
            (
                "github.com/aws/aws-sdk-go/service/ssmincidents",
                "commander",
                "CreateResponsePlan",
                "ssm-incidents",
            ),
        ] {
            assert_eq!(
                catalog_sdk_api_method(package, signer, name),
                Some(ApiMethod::new(canonical, name))
            );
        }
        assert_eq!(
            catalog_sdk_api_method("example.com/opensearch", "es", "CreateDomain"),
            None
        );
        assert_eq!(
            catalog_sdk_api_method(
                "github.com/aws/aws-sdk-go/service/s3",
                "unknown",
                "CreateDomain"
            ),
            None
        );
    }
}
