#!/usr/bin/env bash
# Build a local-only amd64 release candidate. Never publishes or deploys it.
set -euo pipefail
[[ "${1:-}" == --build-local && $# == 1 ]] || {
  echo 'usage: build-private-image-local.sh --build-local' >&2; exit 2;
}
repo=$(git rev-parse --show-toplevel)
cd "$repo"
[[ -z "$(git status --porcelain)" ]] || { echo 'Clean source tree required' >&2; exit 2; }
commit=$(git rev-parse HEAD)
tag="deopt-readonly-local:${commit}"
docker buildx build --load --pull=false --platform linux/amd64 \
  --label "org.opencontainers.image.revision=$commit" \
  --label 'org.opencontainers.image.source=https://github.com/coriom/deopt-v2-backend' \
  -t "$tag" .
actual=$(docker image inspect "$tag" --format '{{index .Config.Labels "org.opencontainers.image.revision"}}')
[[ "$actual" == "$commit" ]] || { echo 'Image source revision label mismatch' >&2; exit 2; }
platform=$(docker image inspect "$tag" --format '{{.Os}}/{{.Architecture}}')
[[ "$platform" == linux/amd64 ]] || { echo 'Image architecture mismatch' >&2; exit 2; }
printf 'SOURCE_COMMIT=%s\n' "$commit"
printf 'DOCKERFILE_SHA256=%s\n' "$(sha256sum Dockerfile | cut -d' ' -f1)"
printf 'LOCAL_IMAGE_ID=%s\n' "$(docker image inspect "$tag" --format '{{.Id}}')"
echo 'REGISTRY_MANIFEST_DIGEST=NOT_AVAILABLE_UNPUBLISHED'
