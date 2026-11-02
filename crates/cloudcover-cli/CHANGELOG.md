# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- Restore modern Go S3 policy generation with explicit warnings for conservatively included directory-session permissions; preserve failure for unrelated unresolved authorization.

## [0.2.2](https://github.com/cyruscook/cloudcover/compare/cloudcover-cli-v0.2.1...cloudcover-cli-v0.2.2) - 2026-10-25

### Other

- Retain unresolved SDK operations and reject incomplete policies ([#51](https://github.com/cyruscook/cloudcover/pull/51))

## [0.2.0](https://github.com/cyruscook/cloudcover/compare/cloudcover-cli-v0.1.9...cloudcover-cli-v0.2.0) - 2026-10-11

### Other

- Add primary-source FAS permission catalog and expansion ([#31](https://github.com/cyruscook/cloudcover/pull/31))

### Added

- Include possible FAS permissions in JSON/HCL policies and report applicability review and unknown supplement coverage on stderr.

## [0.1.7](https://github.com/cyruscook/cloudcover/compare/cloudcover-cli-v0.1.6...cloudcover-cli-v0.1.7) - 2026-10-04

### Other

- Publish javascript and typescript crates ([#19](https://github.com/cyruscook/cloudcover/pull/19))

## [0.1.4](https://github.com/cyruscook/cloudcover/compare/cloudcover-cli-v0.1.3...cloudcover-cli-v0.1.4) - 2026-09-25

### Other

- Fix unhandled services ([#8](https://github.com/cyruscook/cloudcover/pull/8))

## [0.1.1](https://github.com/cyruscook/cloudcover/compare/cloudcover-cli-v0.1.0...cloudcover-cli-v0.1.1) - 2026-09-22

### Other

- Add cargo binstall support
- Resolve Terraform provider local names
