#!/usr/bin/env bash
# Archive the exact SHA of every non-main branch to an immutable recovery tag
# before deleting a single branch. Requires authenticated GitHub CLI (gh auth login).
# No secrets are stored by this script. Dry-run by default.
set -euo pipefail
repo="EMN90909/NoeSQL"
mode="${1:---dry-run}"
[[ "$mode" == "--dry-run" || "$mode" == "--apply" ]] || {
  printf 'Usage: bash scripts/archive-delete-branches.sh [--dry-run|--apply]\n' >&2
  exit 2
}
command -v gh >/dev/null || { echo 'GitHub CLI (gh) is required' >&2; exit 2; }
gh auth status >/dev/null 2>&1 || { echo 'Run gh auth login before branch cleanup' >&2; exit 2; }
mapfile -t branches < <(gh api --paginate "repos/$repo/branches?per_page=100" --jq '.[].name')
declare -a pending=()
for branch in "${branches[@]}"; do
  [[ "$branch" == "main" ]] && continue
  sha=$(gh api "repos/$repo/git/ref/heads/$branch" --jq '.object.sha')
  ref="refs/tags/archive/consolidation-20261021/$branch"
  pending+=("$branch")
  echo "PRESERVE $branch ($sha) -> $ref"
  [[ "$mode" == "--dry-run" ]] && continue
  location="${ref#refs/}"
  if old=$(gh api "repos/$repo/git/ref/$location" --jq '.object.sha' 2>/dev/null); then
    [[ "$old" == "$sha" ]] || { echo "Refuse to overwrite mismatched archive tag $ref" >&2; exit 1; }
  else
    created=$(gh api --method POST "repos/$repo/git/refs" -f ref="$ref" -f sha="$sha" --jq '.object.sha')
    [[ "$created" == "$sha" ]] || { echo "Unable to verify archive creation $ref" >&2; exit 1; }
  fi
done
if [[ "$mode" == "--dry-run" ]]; then
  echo "Dry-run only. Review the branches, then rerun with --apply."
  exit 0
fi
echo "All ${#pending[@]} branch commits archived; deleting only non-main branch refs."
for branch in "${pending[@]}"; do
  gh api --method DELETE "repos/$repo/git/refs/heads/$branch"
  echo "DELETED $branch"
done
echo "Remaining branches:"
gh api --paginate "repos/$repo/branches?per_page=100" --jq '.[].name'
