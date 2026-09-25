terraform {
  required_providers {
    amazon = {
      source = "hashicorp/aws"
    }
  }
}

provider "amazon" {
  region = "us-east-1"
}

module "storage" {
  source = "./.terraform/modules/storage"
}

resource "aws_s3_bucket" "root" {
  bucket = "cloudcover-root"
  provider = amazon
}

data "aws_ecr_image" "base" {
  repository_name = "amazonlinux"
  image_tag       = "latest"
  provider = amazon
}

data "aws_iam_policy_document" "local" {
  provider = amazon
}

resource "aws_acm_certificate_validation" "certificate" {
  provider = amazon
}

resource "aws_bedrockagentcore_gateway" "root" {}

resource "aws_cognito_user_pool" "root" {}

resource "aws_cognito_identity_pool" "root" {}

resource "aws_apigatewayv2_api" "root" {}

resource "aws_cloudwatch_event_rule" "root" {}

resource "aws_scheduler_schedule" "root" {}

data "aws_caller_identity" "current" {}
