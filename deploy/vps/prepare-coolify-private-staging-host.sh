#!/usr/bin/env bash
# Operator-only root preflight. Pulls the reviewed image and creates one DB
# password file. Does not create containers, change proxy ports, or deploy.
set -euo pipefail
umask 077

[[ $# == 1 && "$1" == --prepare ]] || {
  echo 'usage: prepare-coolify-private-staging-host.sh --prepare' >&2; exit 2;
}
[[ "$EUID" == 0 ]] || { echo 'ROOT_REQUIRED' >&2; exit 2; }
image='ghcr.io/coriom/deopt-v2-backend@sha256:b7ed384e9cae879579cf255fac79cce087de7f7c6922e65940c1147c78382436'
commit='e2f139fc33286fea06d94989b31ab9d33aab8b98'
postgres_image='postgres:16@sha256:5db6dac2ef5dcff512f4f00a009c3c332b9f2f28f24277175bf3e4a3d02ef83a'
secret_dir='/srv/deopt/staging/secrets'
secret_file="$secret_dir/postgres_password"

command -v docker >/dev/null || { echo 'DOCKER_MISSING' >&2; exit 2; }
command -v openssl >/dev/null || { echo 'OPENSSL_MISSING' >&2; exit 2; }
docker info >/dev/null 2>&1 || { echo 'ROOT_DOCKER_CONTEXT_UNAVAILABLE' >&2; exit 2; }
containers=$(docker ps --format '{{.Names}}') || { echo 'DOCKER_CONTAINER_LIST_FAILED' >&2; exit 2; }
if grep -Eq '(^|[-_])deopt([-_]|$)' <<<"$containers"; then
  echo 'DEOPT_CONTAINER_ALREADY_PRESENT_STOP' >&2; exit 2;
fi
docker pull "$image" >/dev/null 2>&1 || { echo 'ROOT_CONTEXT_GHCR_PULL_FAILED' >&2; exit 2; }
platform=$(docker image inspect "$image" --format '{{.Os}}/{{.Architecture}}')
revision=$(docker image inspect "$image" --format '{{index .Config.Labels "org.opencontainers.image.revision"}}')
[[ "$platform" == linux/amd64 && "$revision" == "$commit" ]] || {
  echo 'GHCR_IMAGE_IDENTITY_MISMATCH' >&2; exit 2;
}
echo 'ROOT_CONTEXT_GHCR_PULL=PASS'
echo 'BACKEND_IMAGE_PLATFORM=linux/amd64'
printf 'BACKEND_IMAGE_COMMIT=%s\n' "$revision"
docker pull "$postgres_image" >/dev/null 2>&1 || { echo 'POSTGRES_16_IMAGE_PULL_FAILED' >&2; exit 2; }
[[ "$(docker image inspect "$postgres_image" --format '{{.Os}}/{{.Architecture}}')" == linux/amd64 ]] ||
  { echo 'POSTGRES_16_PLATFORM_MISMATCH' >&2; exit 2; }
echo 'POSTGRES_16_IMAGE_PULL=PASS_AMD64'

docker inspect coolify-proxy --format '{{json .HostConfig.PortBindings}}' |
  python3 -c 'import json,sys; p=json.load(sys.stdin); b=p.get("8080/tcp") or []; print("PROXY_8080_IPV4_PUBLIC="+str(any(x.get("HostIp") in ("0.0.0.0","") for x in b))); print("PROXY_8080_IPV6_PUBLIC="+str(any(x.get("HostIp")=="::" for x in b))); print("PROXY_80_443_PUBLISHED="+str("80/tcp" in p and "443/tcp" in p))'
docker inspect coolify-proxy --format '{{json .Config.Cmd}}' |
  python3 -c 'import json,sys; c=json.load(sys.stdin) or []; print("PROXY_API_INSECURE="+str("--api.insecure=true" in c)); print("PROXY_ENTRYPOINT_8080="+str(any(x.startswith("--entrypoints.") and ":8080" in x for x in c)))'

ensure_private_dir() {
  local path="$1"
  if [[ -e "$path" || -L "$path" ]]; then
    [[ -d "$path" && ! -L "$path" && "$(stat -c '%u:%g:%a' "$path")" == 0:0:700 ]] ||
      { echo 'EXISTING_SECRET_DIRECTORY_UNEXPECTED' >&2; exit 2; }
  else
    install -d -o root -g root -m 0700 "$path"
  fi
}
ensure_private_dir /srv/deopt
ensure_private_dir /srv/deopt/staging
ensure_private_dir "$secret_dir"
if [[ -e "$secret_file" || -L "$secret_file" ]]; then
  [[ -f "$secret_file" && ! -L "$secret_file" &&
     "$(stat -c '%u:%g:%a' "$secret_file")" == 0:1001:640 &&
     -s "$secret_file" ]] || { echo 'EXISTING_SECRET_FILE_UNEXPECTED' >&2; exit 2; }
  echo 'POSTGRES_SECRET=PRESERVED'
else
  tmp=$(mktemp "$secret_dir/.postgres_password.XXXXXXXX")
  trap 'rm -f -- "${tmp:-}"' EXIT
  openssl rand -hex 32 >"$tmp"
  chown root:1001 "$tmp"
  chmod 0640 "$tmp"
  ln "$tmp" "$secret_file" || { echo 'SECRET_CREATE_RACE_STOP' >&2; exit 2; }
  rm -f -- "$tmp"
  trap - EXIT
  echo 'POSTGRES_SECRET=CREATED_NO_VALUE_PRINTED'
fi
echo 'DEOPT_CONTAINERS_STARTED=NO'
echo 'PROXY_CONFIGURATION_CHANGED=NO'
