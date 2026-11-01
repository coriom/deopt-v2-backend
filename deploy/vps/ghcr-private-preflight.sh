#!/usr/bin/env bash
# Fail closed before publishing (pre) and verify GHCR visibility afterward (post).
set -euo pipefail

mode="${1:-}"
[[ $# == 1 && ( "$mode" == pre || "$mode" == post ) ]] || {
  echo 'usage: ghcr-private-preflight.sh pre|post' >&2; exit 2;
}
[[ "${GITHUB_REPOSITORY:-}" == coriom/deopt-v2-backend ]] || {
  echo 'GHCR_REPOSITORY_MISMATCH' >&2; exit 2;
}
[[ "${GITHUB_REF:-}" == refs/heads/main ]] || {
  echo 'GHCR_BRANCH_MISMATCH' >&2; exit 2;
}

repository_visibility=$(gh api "repos/$GITHUB_REPOSITORY" --jq .visibility 2>/dev/null) || {
  echo 'GHCR_REPOSITORY_VISIBILITY_UNKNOWN' >&2; exit 2;
}
[[ "$repository_visibility" == private || "$repository_visibility" == public ]] || {
  echo 'GHCR_REPOSITORY_VISIBILITY_UNKNOWN' >&2; exit 2;
}

endpoint='users/coriom/packages/container/deopt-v2-backend'
response=$(mktemp)
trap 'rm -f "$response"' EXIT
gh api -i "$endpoint" >"$response" 2>/dev/null || true
http_status=$(sed -n '1s/^HTTP\/[^ ]* \([0-9][0-9][0-9]\).*$/\1/p' "$response")
case "$http_status" in
  200)
    package_visibility=$(gh api "$endpoint" --jq .visibility 2>/dev/null) || {
      echo 'GHCR_PACKAGE_VISIBILITY_UNKNOWN' >&2; exit 2;
    }
    [[ "$package_visibility" == private ]] || {
      echo 'GHCR_PACKAGE_NOT_PRIVATE' >&2; exit 2;
    }
    if [[ "$mode" == post ]]; then
      linked_repository=$(gh api "$endpoint" --jq '.repository.full_name // empty' 2>/dev/null) || {
        echo 'GHCR_PACKAGE_SOURCE_UNKNOWN' >&2; exit 2;
      }
      [[ "$linked_repository" == "$GITHUB_REPOSITORY" ]] || {
        echo 'GHCR_PACKAGE_SOURCE_MISMATCH' >&2; exit 2;
      }
    fi
    echo 'GHCR_EXISTING_PACKAGE_PRIVATE'
    ;;
  404)
    [[ "$mode" == pre && "$repository_visibility" == private ]] || {
      echo 'GHCR_FIRST_PUBLICATION_PRIVACY_UNPROVEN' >&2; exit 2;
    }
    echo 'GHCR_FIRST_PUBLICATION_FROM_PRIVATE_REPOSITORY'
    ;;
  *)
    echo 'GHCR_PACKAGE_STATUS_UNKNOWN' >&2; exit 2;
    ;;
esac
