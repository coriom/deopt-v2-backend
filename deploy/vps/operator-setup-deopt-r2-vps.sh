#!/usr/bin/env bash
# Operator-only, root-run setup. Never pass credentials as arguments or via Codex.
set +x
set -euo pipefail
umask 077
endpoint='https://77dc4e907cd36ee29964c8aff47f7c38.r2.cloudflarestorage.com'
bucket='deopt-private-backups'
stage='/home/deoptops/deopt-bootstrap.stage'
config='/root/.config/deopt/rclone.conf'
key_home='/root/.local/share/deopt-backup-gpg'
classify_anonymous_s3_http() {
  case "$1" in
    2[0-9][0-9]) printf 'PUBLIC_OBJECT_ACCESS_HTTP_%s\n' "$1"; return 1 ;;
    400) printf 'INCONCLUSIVE_HTTP_400\n' ;;
    000) printf 'INCONCLUSIVE_TRANSPORT\n' ;;
    [0-9][0-9][0-9]) printf 'INCONCLUSIVE_HTTP_%s\n' "$1" ;;
    *) printf 'INCONCLUSIVE_TRANSPORT\n' ;;
  esac
}
if [[ "${1:-}" == '--classify-anonymous-s3-http' ]]; then
  [[ $# == 2 ]] || exit 2
  classify_anonymous_s3_http "$2"
  exit $?
fi
[[ ${EUID} == 0 && $# == 1 && "$1" =~ ^[0-9A-Fa-f]{40}$ ]] || {
  echo 'Run as root with the reviewed 40-character GPG fingerprint' >&2; exit 2;
}
[[ "$(hostname -s)" == 'v2202611448349549977' ]] || { echo 'Wrong host' >&2; exit 2; }
expected_fpr="${1^^}"
[[ -f "$stage/deopt-backup-public.asc" && ! -L "$stage/deopt-backup-public.asc" ]] || {
  echo 'Staged public key missing' >&2; exit 2;
}
observed_fpr="$(gpg --batch --show-keys --with-colons "$stage/deopt-backup-public.asc" 2>/dev/null | awk -F: '$1=="pub"{seen=1;next} seen && $1=="fpr"{print $10;exit}')"
[[ "$observed_fpr" == "$expected_fpr" ]] || { echo 'Public key fingerprint mismatch' >&2; exit 2; }
if ! command -v rclone >/dev/null 2>&1; then
  apt-get update -qq
  apt-get install -y -qq --no-install-recommends rclone
fi
rclone_version="$(rclone version | awk 'NR==1 {sub(/^v/,"",$2); print $2}')"
dpkg --compare-versions "$rclone_version" ge 1.59 || { echo 'rclone too old for R2' >&2; exit 2; }
install -d -m 0700 "$key_home" /root/.config/deopt
gpg --homedir "$key_home" --batch --import "$stage/deopt-backup-public.asc" >/dev/null 2>&1
[[ -z "$(gpg --homedir "$key_home" --batch --with-colons --list-secret-keys 2>/dev/null)" ]] || {
  echo 'Private key must not be present on VPS' >&2; exit 2;
}
imported_fpr="$(gpg --homedir "$key_home" --batch --with-colons --list-keys 2>/dev/null | awk -F: '$1=="pub"{seen=1;next} seen && $1=="fpr"{print $10;exit}')"
[[ "$imported_fpr" == "$expected_fpr" ]] || { echo 'Imported key mismatch' >&2; exit 2; }
if [[ ! -e "$config" ]]; then
  [[ -r /dev/tty && -w /dev/tty ]] || { echo 'Interactive terminal required' >&2; exit 2; }
  printf 'R2 Access Key ID (hidden): ' >/dev/tty
  IFS= read -r -s access_key </dev/tty
  printf '\nR2 Secret Access Key (hidden): ' >/dev/tty
  IFS= read -r -s secret_key </dev/tty
  printf '\n' >/dev/tty
  [[ "$access_key" =~ ^[A-Za-z0-9_-]{8,128}$ && "$secret_key" =~ ^[A-Za-z0-9_+./=-]{16,256}$ ]] || {
    unset access_key secret_key
    echo 'Invalid credential format' >&2; exit 2;
  }
  # Builtins only: credentials never appear in process arguments or command output.
  ( set -C; printf '[deopt_r2]\ntype = s3\nprovider = Cloudflare\nregion = auto\nendpoint = %s\nno_check_bucket = true\naccess_key_id = %s\nsecret_access_key = %s\n' \
      "$endpoint" "$access_key" "$secret_key" >"$config" )
  unset access_key secret_key
fi
[[ ! -L "$config" && "$(stat -c '%u:%a' "$config")" == '0:600' ]] || {
  echo 'R2 config must be root-owned mode 0600' >&2; exit 2;
}
work="$(mktemp -d /tmp/deopt-r2-probe.XXXXXXXX)"
trap 'rm -rf -- "$work"' EXIT
printf 'DeOpt encrypted backup probe; no database or user data; nonce=%s\n' "$(openssl rand -hex 16)" >"$work/plain"
plain_sha="$(sha256sum "$work/plain" | cut -d ' ' -f 1)"
gpg --homedir "$key_home" --batch --yes --trust-model always --encrypt --recipient "$expected_fpr" --output "$work/probe.gpg" "$work/plain"
[[ -s "$work/probe.gpg" ]] || { echo 'Encryption failed' >&2; exit 1; }
object="probes/$(date -u +%Y%m%dT%H%M%SZ)-$(openssl rand -hex 16).gpg"
rclone --config "$config" copyto "$work/probe.gpg" "deopt_r2:$bucket/$object" >/dev/null 2>&1 || {
  echo 'R2 upload failed (authentication or permission)' >&2; exit 1;
}
rclone --config "$config" cat "deopt_r2:$bucket/$object" >"$work/download.gpg" 2>/dev/null || {
  echo 'R2 download failed' >&2; exit 1;
}
cmp -s "$work/probe.gpg" "$work/download.gpg" || { echo 'R2 ciphertext mismatch' >&2; exit 1; }
cipher_sha="$(sha256sum "$work/download.gpg" | cut -d ' ' -f 1)"
if RCLONE_CONFIG_DEOPT_R2_ACCESS_KEY_ID='INVALID_TEST_KEY' \
   RCLONE_CONFIG_DEOPT_R2_SECRET_ACCESS_KEY='INVALID_TEST_SECRET' \
   rclone --config "$config" cat "deopt_r2:$bucket/$object" >/dev/null 2>&1; then
  echo 'Invalid credentials unexpectedly succeeded' >&2; exit 1
fi
anonymous_code="$(curl --silent --show-error --location --max-time 12 --output /dev/null --write-out '%{http_code}' "$endpoint/$bucket/$object" 2>/dev/null)" || anonymous_code='TRANSPORT_ERROR'
anonymous_status="$(classify_anonymous_s3_http "$anonymous_code")" || {
  echo 'Unauthenticated S3 request accessed the probe object; stop and investigate public access' >&2; exit 1;
}
# S3 endpoint responses do not establish r2.dev or Custom Domain exposure.
[[ -r /dev/tty && -w /dev/tty ]] || { echo 'Cloudflare dashboard attestation requires an interactive terminal' >&2; exit 2; }
printf 'Confirm in Cloudflare R2 > deopt-private-backups > Settings: Public Development URL is disabled AND no Custom Domain has Access to Bucket allowed.\nType PRIVATE deopt-private-backups to attest these two settings: ' >/dev/tty
IFS= read -r public_access_attestation </dev/tty
[[ "$public_access_attestation" == 'PRIVATE deopt-private-backups' ]] || {
  echo 'Cloudflare public-access configuration not attested' >&2; exit 1;
}
unset public_access_attestation
probe_stage="$(mktemp -d "$stage/probe.XXXXXXXX")"
install -m 0600 "$work/download.gpg" "$probe_stage/probe.gpg"
chown -R deoptops:deoptops "$probe_stage"
printf 'R2_ENDPOINT=CONFIGURED\nRCLONE_AUTH=PASS\nGPG_PUBLIC_KEY_ON_VPS=PASS\nSYNTHETIC_ENCRYPTION=PASS\nR2_UPLOAD=PASS\nR2_DOWNLOAD=PASS\nSHA256_INTEGRITY=PASS\nBAD_CREDENTIALS=REJECTED\nS3_ANONYMOUS_CHECK=%s\nCLOUDFLARE_PUBLIC_ACCESS=OPERATOR_ATTESTED_NOT_API_VERIFIED\nPROBE_STAGE=%s\nPLAINTEXT_SHA256=%s\nCIPHERTEXT_SHA256=%s\n' \
  "$anonymous_status" "$(basename "$probe_stage")" "$plain_sha" "$cipher_sha"
