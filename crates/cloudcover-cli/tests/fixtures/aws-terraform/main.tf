terraform {
  required_providers {
    aws = {
      source = "hashicorp/aws"
    }
  }
}

provider "aws" {
  region = "us-east-1"
}

module "storage" {
  source = "./.terraform/modules/storage"
}

resource "aws_s3_bucket" "root" {
  bucket = "cloudcover-root"
}
