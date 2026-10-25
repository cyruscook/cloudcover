# SDK metadata contract migration

Keep the SDK-specific extractors and reviewable history formats. `tools/metadata_contracts.py`
validates their common invariants and creates versioned `metadata-manifest.json`
sidecars. Run with `--write` after data/extractor changes; ordinary CI verifies the
sidecars without modifying inputs. Scheduled updater PRs refresh and stage all
changed manifests, including the Terraform dependency inventory.

SAR catalogs, SDK histories and hashed extractor sources use raw-byte checksums.
The scoped `.gitattributes` rules preserve their LF bytes during Windows checkouts;
verifiers do not normalize bytes to accept altered inputs. Sidecar paths always
use `/`, and the verifier writes and compares exact UTF-8/LF bytes on every OS.
Existing Windows clones need to check out the protected files again after fetching
these attributes; a fresh checkout avoids retaining previously converted CRLF files.

The adapters enforce canonical exact stable versions, unique package/module and
mapping identities, arrays rather than null collections, valid removals,
non-conflicting non-redundant changes, and nonempty API identities. Terraform
retains its explicit unsupported-release and known-no-call semantics. JavaScript
accepts legacy unversioned schema 1 and emitted schema 2, rejecting other schemas.
No history tuples or recorded version membership change in this migration.

Each sidecar pins source bytes and current extractor source files by SHA-256,
records unique canonical snapshot hashes and a digest of the ordered
(identity, exact version, coverage status, snapshot digest) stream. Canonical
snapshots use compact UTF-8 JSON with sorted mapping keys and sorted unique API
pairs. No-op releases retain membership in that stream. This permits auditing
semantic equivalence independently of physical compiled encodings.

Historical upstream content digests, extractor revisions/options and extraction
completeness were not recorded. The migration explicitly marks these as unknown
(`legacy_unverified`), rather than inventing provenance. Current extractor hashes
identify code at migration/refresh time, not the historic extractor. Terraform
records the migrated Go SDK inventory digests; these are not claims about which
SDK release was analyzed for every historic provider release. Capturing those
per-analysis immutable dependencies requires future extractor output envelopes.

New envelopes should include immutable upstream source identity/content digest,
extractor revision/options, exact dependency artifact digests, base/result snapshot
digests, and explicit coverage diagnostics. Retain the legacy readers until full
history replay demonstrates equivalent snapshots. The sidecar schema is versioned
independently of internal binary lookup encodings. Current manifests are verified
by CI; builds continue to validate and compile the existing history formats.
