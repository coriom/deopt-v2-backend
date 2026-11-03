#!/usr/bin/env bash
# Run locally in the operator's WSL terminal. Never pass a PAT to Codex.
set +x
set -euo pipefail
umask 077

repo='coriom/deopt-v2-backend'
package='deopt-v2-backend'
image='ghcr.io/coriom/deopt-v2-backend'

die() { printf '%s\n' "$1" >&2; exit 2; }
gh_call() { GH_TOKEN="$pat" gh api "$@" 2>/dev/null; }
api_status() {
  gh_call -i "$1" >"$session_dir/api-response" || true
  sed -n '1s/^HTTP\/[^ ]* \([0-9][0-9][0-9]\).*$/\1/p' "$session_dir/api-response"
}
classify_package() {
  local status visibility
  status=$(api_status 'user/packages?package_type=container&per_page=100')
  case "$status" in
    200) ;;
    401|403) echo ACCESS_DENIED; return ;;
    *) echo UNKNOWN; return ;;
  esac
  gh_call --paginate 'user/packages?package_type=container&per_page=100' \
    --jq '.[].name' >"$session_dir/package-names" || { echo UNKNOWN; return; }
  if ! grep -Fxq "$package" "$session_dir/package-names"; then
    echo ABSENT
    return
  fi
  status=$(api_status "user/packages/container/$package")
  case "$status" in
    200)
      visibility=$(gh_call "user/packages/container/$package" --jq .visibility) || {
        echo UNKNOWN; return;
      }
      case "$visibility" in
        private) echo EXISTS_PRIVATE ;;
        public) echo EXISTS_PUBLIC ;;
        *) echo UNKNOWN ;;
      esac
      ;;
    401|403) echo ACCESS_DENIED ;;
    *) echo UNKNOWN ;;
  esac
}
cleanup() {
  if [[ -n "${session_dir:-}" && -d "$session_dir" ]]; then
    docker logout ghcr.io >/dev/null 2>&1 || true
    rm -r -- "$session_dir"
  fi
  unset pat
}
main() {
  [[ $# == 1 && ( "$1" == --publish-private || "$1" == --publish-private-update ) ]] ||
    die 'usage: ghcr-private-first-publish.sh --publish-private|--publish-private-update'
  local publish_mode=$1 expected_state confirmation
  if [[ "$publish_mode" == --publish-private-update ]]; then
    expected_state=EXISTS_PRIVATE
    confirmation=PUBLISH_PRIVATE_UPDATE
  else
    expected_state=ABSENT
    confirmation=PUBLISH_PRIVATE
  fi
  [[ -t 0 && -r /dev/tty ]] || die 'INTERACTIVE_TTY_REQUIRED'
  for command_name in git gh docker; do
    command -v "$command_name" >/dev/null || die "MISSING_TOOL_$command_name"
  done
  cd "$(git rev-parse --show-toplevel)"
  [[ "$(git remote get-url origin)" == git@github.com:coriom/deopt-v2-backend.git ]] ||
    die 'GIT_REMOTE_MISMATCH'
  [[ "$(git branch --show-current)" == main ]] || die 'MAIN_BRANCH_REQUIRED'
  [[ -z "$(git status --porcelain)" ]] || die 'CLEAN_WORKTREE_REQUIRED'
  local commit remote_commit tag actor scopes scope entry state source_label platform digest linked
  commit=$(git rev-parse HEAD)
  remote_commit=$(git ls-remote origin refs/heads/main 2>/dev/null | awk '{print $1}') ||
    die 'REMOTE_COMMIT_UNAVAILABLE'
  [[ "$commit" == "$remote_commit" ]] || die 'LOCAL_REMOTE_COMMIT_MISMATCH'
  tag="$image:sha-$commit"

  session_dir=$(mktemp -d "${TMPDIR:-/tmp}/deopt-ghcr.XXXXXXXX")
  export DOCKER_CONFIG="$session_dir/docker"
  mkdir -m 700 "$DOCKER_CONFIG" "$session_dir/anonymous-docker"
  trap cleanup EXIT

  printf 'GitHub PAT classic temporaire (write:packages, éventuellement read:packages) : ' >/dev/tty
  IFS= read -r -s pat </dev/tty || die 'PAT_INPUT_FAILED'
  printf '\n' >/dev/tty
  [[ -n "$pat" ]] || die 'EMPTY_PAT'
  gh_call -i user >"$session_dir/user-response" || die 'GITHUB_AUTH_FAILED'
  actor=$(gh_call user --jq .login) || die 'GITHUB_AUTH_FAILED'
  [[ "$actor" == coriom ]] || die 'GITHUB_ACCOUNT_MISMATCH'
  scopes=$(awk 'tolower($1)=="x-oauth-scopes:" {for (i=2;i<=NF;i++) printf "%s", $i}' \
    "$session_dir/user-response" | tr -d '\r ')
  [[ -n "$scopes" ]] || die 'PAT_CLASSIC_SCOPES_UNVERIFIED'
  [[ ",$scopes," == *,write:packages,* ]] || die 'WRITE_PACKAGES_SCOPE_MISSING'
  IFS=',' read -ra scope_entries <<<"$scopes"
  for entry in "${scope_entries[@]}"; do
    case "$entry" in
      write:packages|read:packages) ;;
      *) die 'PAT_HAS_UNEXPECTED_SCOPE' ;;
    esac
  done

  state=$(classify_package)
  case "$state" in
    ABSENT) [[ "$expected_state" == ABSENT ]] || die 'PACKAGE_MISSING_FOR_UPDATE'
      printf 'PACKAGE_EXISTS=NO_AUTHENTICATED_INVENTORY\n' ;;
    EXISTS_PRIVATE) [[ "$expected_state" == EXISTS_PRIVATE ]] || die 'PACKAGE_EXISTS_PRIVATE_NO_FIRST_PUBLISH_NEEDED'
      printf 'PACKAGE_EXISTS=YES_AUTHENTICATED_PRIVATE\n' ;;
    EXISTS_PUBLIC) die 'PACKAGE_EXISTS_PUBLIC_STOP_USE_NEW_APPROVED_NAME' ;;
    ACCESS_DENIED) die 'PACKAGE_INVENTORY_ACCESS_DENIED' ;;
    *) die 'PACKAGE_VISIBILITY_UNKNOWN' ;;
  esac

  docker buildx build --load --platform linux/amd64 --pull=false --provenance=false \
    --label "org.opencontainers.image.revision=$commit" \
    -t "$tag" . >"$session_dir/build.log" 2>&1 || die 'LOCAL_IMAGE_BUILD_FAILED'
  platform=$(docker image inspect "$tag" --format '{{.Os}}/{{.Architecture}}') ||
    die 'LOCAL_IMAGE_INSPECT_FAILED'
  [[ "$platform" == linux/amd64 ]] || die 'LOCAL_IMAGE_PLATFORM_MISMATCH'
  [[ "$(docker image inspect "$tag" --format '{{index .Config.Labels "org.opencontainers.image.revision"}}')" == "$commit" ]] ||
    die 'LOCAL_IMAGE_REVISION_MISMATCH'
  source_label=$(docker image inspect "$tag" --format '{{index .Config.Labels "org.opencontainers.image.source"}}')
  [[ -z "$source_label" || "$source_label" == '<no value>' ]] ||
    die 'PUBLIC_REPOSITORY_SOURCE_LABEL_FORBIDDEN'
  docker run --rm --network none --read-only --entrypoint /bin/sh "$tag" -c \
    'test "$(id -u)" = 1001; test -x /usr/local/bin/deopt-v2-backend; test -d /app/migrations' \
    >"$session_dir/runtime.log" 2>&1 || die 'LOCAL_IMAGE_RUNTIME_FAILED'
  printf 'LOCAL_IMAGE_VERIFIED=%s\n' "$commit"

  state=$(classify_package)
  [[ "$state" == "$expected_state" ]] || die 'PACKAGE_STATE_CHANGED_BEFORE_PUSH'
  printf 'Publier une seule image privée %s ? Saisir %s : ' "$tag" "$confirmation" >/dev/tty
  IFS= read -r decision </dev/tty || die 'OPERATOR_CONFIRMATION_FAILED'
  [[ "$decision" == "$confirmation" ]] || die 'OPERATOR_DID_NOT_CONFIRM'
  printf '%s' "$pat" | docker login ghcr.io -u coriom --password-stdin \
    >"$session_dir/login.log" 2>&1 || die 'GHCR_LOGIN_FAILED'
  state=$(classify_package)
  [[ "$state" == "$expected_state" ]] || die 'PACKAGE_STATE_CHANGED_BEFORE_PUSH'
  if [[ "$expected_state" == EXISTS_PRIVATE ]]; then
    if docker manifest inspect "$tag" >"$session_dir/tag-check.log" 2>&1; then
      die 'COMMIT_TAG_ALREADY_EXISTS'
    fi
    grep -Eiq 'manifest unknown|no such manifest|not found' "$session_dir/tag-check.log" ||
      die 'COMMIT_TAG_STATE_INCONCLUSIVE'
  fi
  docker push "$tag" >"$session_dir/push.log" 2>&1 || die 'GHCR_PUSH_FAILED'
  digest=$(sed -n 's/.*digest: \(sha256:[0-9a-f]\{64\}\).*/\1/p' "$session_dir/push.log" | tail -1)
  [[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]] || die 'REGISTRY_DIGEST_UNAVAILABLE'
  [[ "$(classify_package)" == EXISTS_PRIVATE ]] || die 'POST_PUBLISH_PRIVATE_NOT_VERIFIED'
  linked=$(gh_call "user/packages/container/$package" --jq '.repository.full_name // ""') ||
    die 'POST_PUBLISH_LINKAGE_UNKNOWN'
  [[ -z "$linked" ]] || die 'PACKAGE_LINKED_TO_REPOSITORY_STOP'
  if env -u DOCKER_AUTH_CONFIG DOCKER_CONFIG="$session_dir/anonymous-docker" \
    docker manifest inspect "$image@$digest" >"$session_dir/anonymous.log" 2>&1; then
    die 'ANONYMOUS_PULL_UNEXPECTEDLY_SUCCEEDED'
  fi
  grep -Eiq 'unauthorized|denied|authentication required|insufficient_scope|forbidden|manifest unknown|no such manifest' \
    "$session_dir/anonymous.log" || die 'ANONYMOUS_PULL_INCONCLUSIVE'
  docker pull "$image@$digest" >"$session_dir/pull.log" 2>&1 ||
    die 'AUTHENTICATED_PULL_FAILED'
  [[ "$(docker image inspect "$image@$digest" --format '{{index .Config.Labels "org.opencontainers.image.revision"}}')" == "$commit" ]] ||
    die 'PULLED_IMAGE_REVISION_MISMATCH'
  printf 'PACKAGE_VISIBILITY=PRIVATE\nANONYMOUS_PULL=DENIED\nAUTHENTICATED_PULL=PASS\n'
  printf 'SOURCE_COMMIT=%s\nREGISTRY_DIGEST=%s\n' "$commit" "$digest"
  echo 'DEPLOYMENT=NOT_PERFORMED'
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
