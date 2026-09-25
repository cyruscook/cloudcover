use cloudcover_core::{
    ApiMethod, CloudProvider, GoMethodReference, Language, MethodReference, PythonMethodReference,
    ResolvedSdk, Sdk, SdkMethodMapping, TerraformMethodReference,
};

use crate::{generated, policy};

#[derive(Clone, Copy, Debug, Default)]
pub struct AwsProvider;

impl AwsProvider {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
    /// Returns the IAM permissions authorized by an AWS API operation.
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
                .filter_map(|api_method| catalog_api_method(api_method.service, api_method.name))
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
                .filter_map(|api_method| catalog_api_method(api_method.service, api_method.name))
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
    let service = if exists(service) {
        service
    } else {
        let catalog_service = catalog_service_alias(service)?;
        if !exists(catalog_service) {
            return None;
        }
        catalog_service
    };
    Some(ApiMethod::new(service, name))
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
        "arcregionswitch" => "arc-region-switch",
        "arczonalshift" => "arc-zonal-shift",
        "autoscalingplans" => "autoscaling-plans",
        "bcmdataexports" => "bcm-data-exports",
        "bedrockagent" => "bedrock",
        "bedrockagentcorecontrol" => "bedrock-agentcore",
        "chimesdkmediapipelines" | "chimesdkvoice" => "chime",
        "cloudcontrol" | "cloudcontrolapi" => "cloudformation",
        "cloudfrontkeyvaluestore" => "cloudfront-keyvaluestore",
        "cloudhsmv2" => "cloudhsm",
        "cloudwatchevents" | "eventbridge" => "events",
        "cloudwatchevidently" => "evidently",
        "cloudwatchrum" => "rum",
        "codeguruprofiler" => "codeguru-profiler",
        "codegurureviewer" => "codeguru-reviewer",
        "codestarconnections" => "codestar-connections",
        "codestarnotifications" => "codestar-notifications",
        "cognitoidentity" => "cognito-identity",
        "cognitoidentityprovider" => "cognito-idp",
        "computeoptimizer" => "compute-optimizer",
        "configservice" => "config",
        "costandusagereportservice" => "cur",
        "costexplorer" => "ce",
        "costoptimizationhub" => "cost-optimization-hub",
        "customerprofiles" => "profile",
        "databasemigrationservice" => "dms",
        "devopsguru" => "devops-guru",
        "directoryservice" => "ds",
        "docdb" | "neptune" => "rds",
        "docdbelastic" => "docdb-elastic",
        "ecrpublic" => "ecr-public",
        "efs" => "elasticfilesystem",
        "elasticloadbalancingv2" | "elbv2" | "elb" => "elasticloadbalancing",
        "elasticsearchservice" => "es",
        "emr" => "elasticmapreduce",
        "emrcontainers" => "emr-containers",
        "emrserverless" => "emr-serverless",
        "keyspaces" => "cassandra",
        "kinesisanalyticsv2" => "kinesisanalytics",
        "lambdacore" | "lambdamicrovms" => "lambda",
        "lexmodelbuildingservice" | "lexmodelsv2" => "lex",
        "licensemanager" => "license-manager",
        "location" | "locationservice" => "geo",
        "mailmanager" | "sesv2" => "ses",
        "managedgrafana" => "grafana",
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
        "ssmincidents" => "ssm-incidents",
        "ssmquicksetup" => "ssm-quicksetup",
        "ssoadmin" => "sso",
        "timestreaminfluxdb" => "timestream-influxdb",
        "timestreamquery" | "timestreamwrite" => "timestream",
        "vpclattice" => "vpc-lattice",
        "wafregional" => "waf-regional",
        "workspacesweb" => "workspaces-web",
        _ => return None,
    })
}
