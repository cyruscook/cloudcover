# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.2](https://github.com/cyruscook/cloudcover/compare/cloudcover-terraform-provider-aws-v0.2.1...cloudcover-terraform-provider-aws-v0.2.2) - 2026-10-25

### Other

- Validate SDK metadata contracts and add provenance manifests ([#46](https://github.com/cyruscook/cloudcover/pull/46))
- Preserve successor snapshots when backfilling SDK histories ([#45](https://github.com/cyruscook/cloudcover/pull/45))

## [0.2.0](https://github.com/cyruscook/cloudcover/compare/cloudcover-terraform-provider-aws-v0.1.9...cloudcover-terraform-provider-aws-v0.2.0) - 2026-10-11

### Other

- Fix SDK service aliases and Terraform security group rule mappings ([#33](https://github.com/cyruscook/cloudcover/pull/33))
- Fix missing Terraform load balancer and S3 upload permissions ([#29](https://github.com/cyruscook/cloudcover/pull/29))

### Fixed

- Follow factory-bound callback fields and promoted interfaces. Restore VPC security group rule create/read/delete operations from provider 4.56.0 onward and remove permissions for the opposite rule direction.
- Recognize AWS S3 uploader helper calls and restore single-part, multipart, and cleanup operations in S3 object create/update mappings from provider 3.71.0 onward.

## [0.1.9](https://github.com/cyruscook/cloudcover/compare/cloudcover-terraform-provider-aws-v0.1.8...cloudcover-terraform-provider-aws-v0.1.9) - 2026-10-10

### Other

- Fix Terraform AWS provider tag permission leakage ([#27](https://github.com/cyruscook/cloudcover/pull/27))

## [0.1.8](https://github.com/cyruscook/cloudcover/compare/cloudcover-terraform-provider-aws-v0.1.7...cloudcover-terraform-provider-aws-v0.1.8) - 2026-10-09

### Other

- Fix Terraform provider analysis of external schema lifecycle handlers ([#23](https://github.com/cyruscook/cloudcover/pull/23))
- Fix Terraform AWS provider analysis of helper packages ([#21](https://github.com/cyruscook/cloudcover/pull/21))

## [0.1.5](https://github.com/cyruscook/cloudcover/compare/cloudcover-terraform-provider-aws-v0.1.4...cloudcover-terraform-provider-aws-v0.1.5) - 2026-09-26

### Other

- Reduce Terraform AWS analyzer memory ([#12](https://github.com/cyruscook/cloudcover/pull/12))

## [0.1.1](https://github.com/cyruscook/cloudcover/compare/cloudcover-terraform-provider-aws-v0.1.0...cloudcover-terraform-provider-aws-v0.1.1) - 2026-09-22

### Other

- Fix canonical service name generation
