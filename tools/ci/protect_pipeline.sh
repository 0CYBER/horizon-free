#!/usr/bin/env bash
# Makes main's branch protection require the code owners' review (docs/security.md): a pull request that
# changes a file .github/CODEOWNERS lists waits for Ludovic's approval; every other one still merges as soon as
# `verify` passes (no approvals are required otherwise). Approvals are dismissed by a later push. Everything
# else in the protection (the required `verify` from GitHub Actions, admins included) is kept as it is.
#
# Apply it only once agents open and push pull requests as the horizon-agent GitHub App, not as ludovic111:
# GitHub never counts an author's approval of their own pull request, so until then an agent's change to a
# listed file could not merge at all.
#
#   tools/ci/protect_pipeline.sh            # print the protection it would set, change nothing
#   tools/ci/protect_pipeline.sh --apply    # set it (needs the maintainer's admin login)
set -euo pipefail
repo=${GITHUB_REPOSITORY:-ludovic111/horizon}
api="repos/$repo/branches/main/protection"
body=$(gh api "$api" | jq '{
  required_status_checks: (.required_status_checks
    | {strict, checks: [.checks[] | {context, app_id}]}),
  enforce_admins: .enforce_admins.enabled,
  required_pull_request_reviews: {
    require_code_owner_reviews: true,
    required_approving_review_count: 0,
    dismiss_stale_reviews: true,
    require_last_push_approval: false
  },
  restrictions: null,
  required_linear_history: .required_linear_history.enabled,
  allow_force_pushes: .allow_force_pushes.enabled,
  allow_deletions: .allow_deletions.enabled,
  required_conversation_resolution: .required_conversation_resolution.enabled,
  lock_branch: .lock_branch.enabled,
  allow_fork_syncing: .allow_fork_syncing.enabled
}')
echo "$body"
if [[ "${1:-}" == --apply ]]; then
  gh api -X PUT "$api" --input - <<< "$body" > /dev/null
  echo "main now requires code owners' reviews."
else
  echo "(not applied: run with --apply)"
fi
