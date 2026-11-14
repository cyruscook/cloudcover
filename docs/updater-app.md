# Dedicated updater identity

The five scheduled data updaters use a dedicated GitHub App to publish their
branches and create/update PRs. This avoids the approval gate GitHub applies to
PR events generated with the built-in `GITHUB_TOKEN`. It does not approve the
resulting code or bypass required CI or branch protection.

## Required setup before merging this change

1. Create a private GitHub App owned by `cyruscook`, dedicated to CloudCover.
   Disable webhooks; no OAuth callback or user authorization is needed.
2. Grant only repository **Contents: read/write** and **Pull requests:
   read/write**, plus implicit metadata read. Do not grant Actions,
   Administration, Workflows, Secrets, organization permissions or bypass rights.
3. Install it on **only `cyruscook/cloudcover`**.
4. Create the Actions environment `cloudcover-updater`. Its deployment branch
   rules must allow **only the branch `main`**, with no tag rule, wildcard, or
   pull-request ref. Do not use the broad “protected branches” option. This is
   essential: the YAML job condition alone is not a boundary against PR edits.
   Required reviewers can remain unset for unattended operation. Do not allow
   an untrusted branch to use this environment.
5. Store the App's client ID in the environment configuration variable
   `CLOUDCOVER_UPDATER_APP_CLIENT_ID` and its private key in the **environment**
   secret `CLOUDCOVER_UPDATER_APP_PRIVATE_KEY`. Never store the key as a repository
   secret, committed file, artifact, log, or PR text. Use approved secure setup;
   do not paste credentials into chat. Do not duplicate an existing dedicated
   App or secret without checking the existing configuration first.
6. Verify the exact installation permissions, selected repository, deployment
   branch restrictions, and that no ruleset bypass was added before activation.

Refer to GitHub's [App setup guide](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/making-authenticated-api-requests-with-a-github-app-in-a-github-actions-workflow)
and [GITHUB_TOKEN behavior](https://docs.github.com/en/actions/concepts/security/github_token).

## Runtime boundaries

- Only `main` can run the updater job. Checkout does not persist credentials.
- Generation, provenance refresh and existing tests run before token creation.
- Validate all changed/untracked paths, file modes and JSON before committing,
  then validate the complete committed diff from the immutable source SHA before
  minting the token. SDK updaters allow their own direct data JSON files plus the
  four metadata manifests; SAR allows only its catalog and manifest JSON.
- The publication guard is defense against accidental changes, not a sandbox
  for malicious generator code or dependencies executing in the same job.
  Trusted main code and dependency review remain necessary.
- Explicit token scope is `cloudcover`, Contents write and Pull requests write.
  Tokens expire after one hour and are revoked by the official action's normal
  cleanup. They are minted late to avoid expiry during long generators.
- Only push and PR steps receive the token. Git uses a credential helper that
  reads the temporary `GH_TOKEN`, not a token-bearing URL or persistent secret.
- Existing force-with-lease protections, source validation and SAR draft status
  remain unchanged. The App does not merge PRs.
- App contents-write permission is repository-wide, not path-scoped. The path
  checks are additional safeguards, and main branch protections must remain.

## Validation and rollout

Run the Python tool tests and `make check`, plus workflow lint/security analysis.
Validate a newly created updater PR and a subsequent branch update both start
CI without a workflow-approval prompt, using the expected App identity. Wait for
all required checks before merge. Do not use a metadata-only PR body edit as a
CI test; it does not trigger the relevant events. Existing held runs may still
need a separate approval; installing the App is not proof they were released.

If setup is unavailable, keep this PR draft and do not claim unattended updates
are working. Rollback is to revert the workflow changes and revoke/uninstall
this dedicated App if no longer used. Do not relax external-fork policies or
switch PR CI to `pull_request_target` to work around the approval gate.
