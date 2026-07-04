# cloudcover-aws

Implements the AWS cloud provider for CloudCover.

Responsibilities:
- expose supported AWS API operations
- map supported SDK method references to AWS API methods
- translate API methods to IAM permissions policy output

Current SDK inputs:
- boto3 Python mappings generated in this crate from Service Authorization Reference data
- Go mappings from `cloudcover-aws-sdk-go-v2`
- Terraform provider entrypoint mappings from `cloudcover-terraform-provider-aws`
