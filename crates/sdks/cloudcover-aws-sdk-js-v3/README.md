# AWS SDK for JavaScript / TypeScript v3 mappings

`data/mappings.json` records exact stable npm package releases for current and
historical `@aws-sdk/client-*` packages and `@aws-sdk/lib-dynamodb`. Each package starts with a full mapping
snapshot, followed by removals and upserts for each subsequent release. Releases
with no mapping changes remain indexed under their exact package version.

Mappings cover command symbols, aggregated client methods, paginators, and
waiters. API service names come from the version-matched SDK signing metadata.
The build validates release ordering and deltas, shares identical release
mapping arrays, and generates an exact package/version lookup.

`lib-dynamodb` command mappings come from the underlying `client-dynamodb`
command constructed by each wrapper, rather than its shortened name: `GetCommand`
maps to `GetItem`, `PutCommand` to `PutItem`, `TransactWriteCommand` to
`TransactWriteItems`, and so on. Document convenience methods and pagination
helpers use the same operation mappings, including Query/Scan and the newer
`paginatedBatchGet`. This preserves the existing DynamoDB authorization catalog:
transactions use item actions, while PartiQL uses the `PartiQL*` actions. Request
arguments and statement text are not narrowed, so policies retain the catalog's
conservative action union and documented possible FAS permissions.

Client constructors, `.from()`, `.destroy()`, and `NumberValue` helpers make no API
calls. Unknown SDK operations and unrecorded package versions continue to fail
closed. The document client history starts at its first stable release, `3.8.0`;
helpers are only mapped in releases that contain them.

Source evidence: [AWS document command implementation](https://github.com/aws/aws-sdk-js-v3/blob/main/lib/lib-dynamodb/src/commands/GetCommand.ts),
[document convenience client](https://github.com/aws/aws-sdk-js-v3/blob/main/lib/lib-dynamodb/src/DynamoDBDocument.ts),
and [PartiQL authorization](https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/ql-iam.html).

## Regenerate

Install Git and Node.js 24.22.1 or a newer patched supported release. From the repository root:

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
releases, `--package-history NAME` for all stable releases of one package, and
`--output PATH` to write a separate data file. Each invocation replaces its output;
use `--all` for the checked-in inventory to retain all other packages.

Generator fixtures retain published package modules with inline source maps
removed, plus their exact tarball URL and SHA-256. The analyzer and CLI regression
fixture uses real pinned SDK dependencies. To run it locally:

```sh
npm ci --ignore-scripts --prefix crates/langs/cloudcover-javascript/analyzer/fixtures/dynamodb
node --test crates/langs/cloudcover-javascript/analyzer/analyzer.test.js
cargo test -p cloudcover-aws --test dynamodb
cargo test -p cloudcover-cli --test policy
```

## Pooled compiled lookups and migration

The source JSON history and exact recorded versions are unchanged. Compiled
snapshots now hold checked `u32` row IDs into one global mapping pool. The release
directory uses one module range, one exact version list and `u32` snapshot IDs,
rather than repeated package/version/pointer tuples. `u32` is intentional: the
current pool has 77,653 rows and already exceeds the capacity of `u16`.

`service_method_mappings` now returns `Option<AwsSdkJsV3Mappings>`, a borrowed view,
in place of `Option<&'static [AwsSdkJsV3MethodMapping]>`. Existing `.iter()`, `.len()`,
`.is_empty()`, indexing and `for` iteration remain available. `.get()` returns a
borrowed row, and `.to_vec()` explicitly materializes an owned contiguous list for
callers that require one. Code with a concrete slice return type should migrate
to the view or collect into a vector. Ordinary lookup/iteration does not allocate.
The API-method lists on each row retain their static slice representation.

Exact membership is checked even after numeric version search, so unrecorded
spellings, prereleases and build suffixes are rejected. The full-history test
compares independent JSON replay to the pooled public decoder for all 253,568
package releases, including retired packages and IDs beyond `u16`.

`examples/metadata_walk.rs` walks all versions and their rows to keep the entire
metadata graph reachable when measuring linked sizes. Generated Rust byte counts
and linked ELF section sizes should be reported separately, together with compiler,
profile and linker settings. See the PR validation record for measured results.

### Measured encoding comparison

Using the c5a2294 history, generated Rust decreases from **52,010,178** to
**28,545,304 bytes**. The 77,653 pooled rows and 524,606 distinct-snapshot memberships
retain all 253,568 recorded package releases.

A Linux x86-64 harness walking every release (`metadata_walk.rs`) linked against
Cargo dev-profile libraries with Rust 1.99.0, an opt-level-2 harness, no LTO, and
`-C debuginfo=0 -C strip=symbols`, measured **121,921,992 -> 29,196,928 bytes**.
These are complete stripped harness executables, not CloudCover release binaries.
ELF `size` data sections decreased from **50,934,568 to 10,204,080 bytes**; reducing
pointer relocations also contributes substantially to the file-size improvement.
Both executables produce checksum `587630968`. Three concurrent-build-environment
walks ranged 0.159–0.296 seconds before and 0.148–0.349 after. These noisy runs do
not establish a latency improvement; full-release decoding equivalence is the
correctness gate. Hot view/iterator methods are inline across crate boundaries.

To repeat, preserve this example source and compile it against each revision's
`cargo build -p cloudcover-aws-sdk-js-v3` rlib with the same rustc/linker settings.
Do not compare a dev-profile baseline to an optimized release library. Package
sharding, binary version-string encoding and additional compression remain separate
follow-ups; this change only pools rows and compacts release indexes.
