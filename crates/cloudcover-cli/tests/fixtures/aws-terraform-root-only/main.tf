terraform {
  required_providers {
    aws = {
      source = "hashicorp/aws"
    }
  }
}

resource "aws_s3_bucket" "root" {
  bucket = "cloudcover-root-only"
}

data "aws_ecr_image" "base" {
  repository_name = "example"
  image_tag       = "latest"
}
