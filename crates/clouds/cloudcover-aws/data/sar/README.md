# Pinned AWS Service Authorization Reference

Normal Cargo builds validate `catalog.json` against the SHA-256 digest in
`manifest.json` and compile it without network access or a shared generated cache.
The manifest records the index digest, retrieval time, extractor revision and
refresh script digest. Each service retains its upstream URL, modification marker
and original response digest. The catalog digest pins the normalized documents.

Refresh explicitly from the repository root with `python3 tools/refresh_sar.py`.
Review and commit both changed files together. The updater checks that the index
has not changed during acquisition; AWS does not offer an atomic historical SAR
snapshot. Review policy and FAS tests before accepting an update. The FAS supplement
remains independently versioned in `../fas.json`.

Migration: `CLOUDCOVER_AWS_SAR_REFRESH` and the old shared `actions-v2.rs` cache no
longer select inputs. The checked-in catalog is authoritative. No SDK histories or
public Rust lookup APIs change. Offline builds still need locally available Cargo
and Go dependencies; the SAR acquisition itself is no longer part of a build.

## Scheduled refresh PRs

`update-aws-sar-catalog.yml` checks daily at **04:19 UTC** and supports
`workflow_dispatch`. It always uses code from the default branch. The stable
`automation/aws-sar-catalog` branch has at most one open update PR. Pending branch
files are read only as data, never checked out or executed.

The refresher preserves both files when the fetched catalog is byte-identical to
main or the pending PR. Retrieval timestamps, the current Git revision and the
refresh script digest alone do not cause commits. Actual catalog input changes
(including source/provenance bytes) produce a checksum-paired catalog/manifest and
a summary of service, IAM action, API operation, resource and condition-key changes.
The summary compares against main, matching the proposed PR diff.

Before publishing, the job runs `make check` and
`cargo test --locked --workspace --all-features`, including checksum/schema,
deterministic generation, AWS/FAS policy and CLI integration tests. Publication
uses a lease captured before acquisition to avoid overwriting concurrent branch
edits. New data PRs are drafts; merging remains manual. No-op runs skip expensive
builds and never push or edit a PR.

Only the existing `GITHUB_TOKEN` is used. The updater job requests repository
contents and pull-request write permissions to push that branch and create/edit
its PR; it does not alter repository/account settings or add credentials. Existing
repository rules may prevent Actions from opening PRs, in which case publication
fails rather than changing those rules.

GitHub currently creates **approval-required** workflow runs for `pull_request`
`opened`, `synchronize` and `reopened` events caused by `GITHUB_TOKEN`. A repository
writer can choose **Approve workflows to run** on the PR. Other token-generated
events generally do not trigger runs; `workflow_dispatch` and
`repository_dispatch` are exceptions. See
[GitHub's token event documentation](https://docs.github.com/en/actions/concepts/security/github_token#when-github_token-triggers-workflow-runs).
No PAT, extra GitHub App, automatic approval or auto-merge is configured.
