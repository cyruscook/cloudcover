# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.2](https://github.com/cyruscook/cloudcover/compare/cloudcover-aws-v0.2.1...cloudcover-aws-v0.2.2) - 2026-10-25

### Other

- Retain unresolved SDK operations and reject incomplete policies ([#51](https://github.com/cyruscook/cloudcover/pull/51))
- Refresh the pinned SAR catalog through a daily update PR ([#49](https://github.com/cyruscook/cloudcover/pull/49))
- Pin SAR catalog inputs for reproducible offline builds ([#43](https://github.com/cyruscook/cloudcover/pull/43))

## [0.2.0](https://github.com/cyruscook/cloudcover/compare/cloudcover-aws-v0.1.9...cloudcover-aws-v0.2.0) - 2026-10-11

### Other

- Add primary-source FAS permission catalog and expansion ([#31](https://github.com/cyruscook/cloudcover/pull/31))
- Fix SDK service aliases and Terraform security group rule mappings ([#33](https://github.com/cyruscook/cloudcover/pull/33))
- Replace IAM action tables with SAR permission references ([#32](https://github.com/cyruscook/cloudcover/pull/32))
- Fix missing Terraform load balancer and S3 upload permissions ([#29](https://github.com/cyruscook/cloudcover/pull/29))

### Added

- Expand documented possible FAS dependencies from a checked-in AWS source catalog, preserving conditions, resource roles, transitive paths, and coverage status.

### Fixed

- Resolve historical SDK display names and runtime client service aliases using AWS-published operation metadata, retaining their API and IAM mappings.

## [0.1.9](https://github.com/cyruscook/cloudcover/compare/cloudcover-aws-v0.1.8...cloudcover-aws-v0.1.9) - 2026-10-10

### Other

- Fix Terraform AWS provider tag permission leakage ([#27](https://github.com/cyruscook/cloudcover/pull/27))

## [0.1.4](https://github.com/cyruscook/cloudcover/compare/cloudcover-aws-v0.1.3...cloudcover-aws-v0.1.4) - 2026-09-25

### Other

- Fix unhandled services ([#8](https://github.com/cyruscook/cloudcover/pull/8))

## [0.1.1](https://github.com/cyruscook/cloudcover/compare/cloudcover-aws-v0.1.0...cloudcover-aws-v0.1.1) - 2026-09-22

### Other

- Fix canonical service name generation
- Normalize AWS service mapping identifiers
