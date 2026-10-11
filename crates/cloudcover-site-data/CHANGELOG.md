# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0](https://github.com/cyruscook/cloudcover/compare/cloudcover-site-data-v0.1.9...cloudcover-site-data-v0.2.0) - 2026-10-11

### Other

- Add primary-source FAS permission catalog and expansion ([#31](https://github.com/cyruscook/cloudcover/pull/31))
- Fix SDK service aliases and Terraform security group rule mappings ([#33](https://github.com/cyruscook/cloudcover/pull/33))
- Replace IAM action tables with SAR permission references ([#32](https://github.com/cyruscook/cloudcover/pull/32))
- Fix missing Terraform load balancer and S3 upload permissions ([#29](https://github.com/cyruscook/cloudcover/pull/29))

### Added

- Publish FAS applicability, AWS citations, transitive provenance, and coverage alongside API permissions and distribute the checked-in supplement.

### Fixed

- Resolve Terraform service aliases through the AWS provider so load balancers and other aliased services retain their API and IAM mappings.

## [0.1.2](https://github.com/cyruscook/cloudcover/compare/cloudcover-site-data-v0.1.1...cloudcover-site-data-v0.1.2) - 2026-09-23

### Other

- release v0.1.1 ([#2](https://github.com/cyruscook/cloudcover/pull/2))

## [0.1.1](https://github.com/cyruscook/cloudcover/compare/cloudcover-site-data-v0.1.0...cloudcover-site-data-v0.1.1) - 2026-09-22

### Other

- release v0.1.1 ([#1](https://github.com/cyruscook/cloudcover/pull/1))
