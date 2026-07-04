# cloudcover-core

Shared domain model for CloudCover.

This crate defines the stable types used across analyzers, SDK mapping crates, cloud providers, and the CLI, including:
- `Language`
- `Sdk`
- `MethodReference` variants
- `ApiMethod`
- `SdkMethodMapping`
- `CloudProvider`

Keep this crate free of provider-specific or analyzer-specific behavior beyond the shared model surface.
