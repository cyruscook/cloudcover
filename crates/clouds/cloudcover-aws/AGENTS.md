# cloudcover-aws

Implements AWS cloud provider functionality for CloudCover.

Uses Service Authorization Reference data (https://servicereference.us-east-1.amazonaws.com/) to cover API operation -> IAM permission and boto3 SDK -> API operation mapping.

aws-sdk-go-v2 SDK -> API operation mapping is provided by the `cloudcover-aws-sdk-go-v2` crate.
