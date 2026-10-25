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
default_branch=$(gh api "repos/$repo" --jq '.default_branch')
[[ "$default_branch" == "main" ]] || {
  echo "Refusing cleanup: default branch is $default_branch, not main" >&2
  exit 2
}
open_prs=$(gh api "repos/$repo/pulls?state=open&per_page=100" --jq 'length')
[[ "$open_prs" == "0" ]] || {
  echo "Refusing cleanup: $open_prs pull request(s) are still open" >&2
  exit 2
}
mapfile -t branches < <(gh api --paginate "repos/$repo/branches?per_page=100" --jq '.[].name')
declare -a pending=()
declare -A archived_heads=()
for branch in "${branches[@]}"; do
  [[ "$branch" == "main" ]] && continue
  sha=$(gh api "repos/$repo/git/ref/heads/$branch" --jq '.object.sha')
  ref="refs/tags/archive/consolidation-20261026/$branch"
  pending+=("$branch")
  archived_heads["$branch"]="$sha"
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
  current=$(gh api "repos/$repo/git/ref/heads/$branch" --jq '.object.sha')
  [[ "$current" == "${archived_heads[$branch]}" ]] || {
    echo "Refusing to delete updated branch $branch; preserve its new head first" >&2
    exit 1
  }
  gh api --method DELETE "repos/$repo/git/refs/heads/$branch"
  echo "DELETED $branch"
done
echo "Remaining branches:"
mapfile -t remaining < <(gh api --paginate "repos/$repo/branches?per_page=100" --jq '.[].name')
printf '%s\n' "${remaining[@]}"
[[ "${#remaining[@]}" == "1" && "${remaining[0]}" == "main" ]] || {
  echo "Branch cleanup incomplete: inspect remaining refs" >&2
  exit 1
}
echo "Verified: main is the only remaining branch (historical commits retained as archive tags)."
