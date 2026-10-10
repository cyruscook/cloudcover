// The machine-readable Service Authorization Reference's AuthorizedActions
// omits these calls made by services on the caller's behalf. Keep supplements
// at the API layer so every SDK, the CLI, and the site resolve them identically.
// These permissions depend on encryption/request options; like SAR authorized
// actions, they describe possible requirements, not unconditional requirements.
//
// S3 documents SSE-KMS and checksum requirements in each API's Permissions:
// https://docs.aws.amazon.com/AmazonS3/latest/API/API_PutObject.html
// https://docs.aws.amazon.com/AmazonS3/latest/API/API_GetObject.html
// https://docs.aws.amazon.com/AmazonS3/latest/API/API_HeadObject.html
// https://docs.aws.amazon.com/AmazonS3/latest/API/API_CopyObject.html
// https://docs.aws.amazon.com/AmazonS3/latest/API/API_CreateMultipartUpload.html
// https://docs.aws.amazon.com/AmazonS3/latest/API/API_CompleteMultipartUpload.html
// https://docs.aws.amazon.com/AmazonS3/latest/API/API_UploadPart.html
// https://docs.aws.amazon.com/AmazonS3/latest/API/API_UploadPartCopy.html
// Secrets Manager documents key validation, encryption, and replication here:
// https://docs.aws.amazon.com/secretsmanager/latest/userguide/security-encryption.html
// https://docs.aws.amazon.com/secretsmanager/latest/apireference/API_BatchGetSecretValue.html
pub(crate) fn kms_actions(service: &str, operation: &str) -> &'static [&'static str] {
    match (service, operation) {
        (
            "s3",
            "PutObject"
            | "GetObject"
            | "HeadObject"
            | "CopyObject"
            | "CreateMultipartUpload"
            | "CompleteMultipartUpload"
            | "UploadPartCopy",
        )
        | ("secretsmanager", "CreateSecret" | "PutSecretValue" | "ReplicateSecretToRegions") => {
            &["Decrypt", "GenerateDataKey"]
        }
        ("s3", "UploadPart") | ("secretsmanager", "GetSecretValue" | "BatchGetSecretValue") => {
            &["Decrypt"]
        }
        // Changing the key re-encrypts existing versions using Encrypt; key
        // validation also calls GenerateDataKey and Decrypt without a value.
        ("secretsmanager", "UpdateSecret") => &["Decrypt", "Encrypt", "GenerateDataKey"],
        _ => &[],
    }
}
