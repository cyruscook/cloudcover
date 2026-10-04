# AWS SDK for JavaScript / TypeScript v3 mappings

`data/mappings.json` records exact stable npm package releases for current and
historical `@aws-sdk/client-*` packages. Each package starts with a full mapping
snapshot, followed by removals and upserts for each subsequent release. Releases
with no mapping changes remain indexed under their exact package version.

Mappings cover command symbols, aggregated client methods, paginators, and
waiters. API service names come from the version-matched SDK signing metadata.
The build validates release ordering and deltas, shares identical release
mapping arrays, and generates an exact package/version lookup.

## Regenerate

Install Git and Node.js 24 or later. From the repository root:

```sh
npm ci --prefix crates/sdks/cloudcover-aws-sdk-js-v3/generator
node crates/sdks/cloudcover-aws-sdk-js-v3/generator/generate.mjs --all
make check
cargo test -p cloudcover-aws-sdk-js-v3
```

The generator caches a bare AWS SDK repository under
`~/.cache/cloudcover/aws-sdk-js-v3.git` and refreshes its tags before generation.
Set `CLOUDCOVER_AWS_SDK_REPOSITORY` to use an existing checkout or bare repository;
refresh its tags yourself before running. Exact versions absent from Git history
use their published npm tarball. Unavailable tarballs are reported explicitly.

Use `--latest` for a current snapshot, `--package NAME@VERSION` for exact selected
releases, and `--output PATH` to write a separate data file.
