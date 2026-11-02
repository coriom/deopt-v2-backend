#!/usr/bin/env bash
# One operator sudo step: verify staged scripts, install root-owned copies, run backup.
set -euo pipefail
set +x
umask 077
[[ ${EUID} == 0 && "$(hostname -s)" == v2202611448349549977 ]] || {
  echo 'REVIEWED_ROOT_HOST_REQUIRED' >&2; exit 2;
}
stage=/home/deoptops/deopt-release.stage
backup_hash=d132f8596042a4a1011257e98fd6a8d340f26407d1b0d4b0dc520f103b13a418
runner_hash=de48460e203be3a601a5853e71e63e7ca44f48482216cfa51c6a3b64892dba2a
backup_source=$stage/backup-offsite.sh
runner_source=$stage/run-first-production-backup-v2.sh
[[ -d "$stage" && ! -L "$stage" && "$(stat -c %U:%G:%a "$stage")" == deoptops:deoptops:700 ]] || {
  echo 'STAGE_DIRECTORY_NOT_REVIEWED' >&2; exit 2;
}
for source in "$backup_source" "$runner_source"; do
  [[ -f "$source" && ! -L "$source" ]] || { echo 'STAGED_SCRIPT_NOT_REGULAR' >&2; exit 2; }
done
[[ "$(sha256sum "$backup_source" | cut -d ' ' -f 1)" == "$backup_hash" &&
   "$(sha256sum "$runner_source" | cut -d ' ' -f 1)" == "$runner_hash" ]] || {
  echo 'STAGED_SCRIPT_INTEGRITY_FAIL' >&2; exit 2;
}
backup_root=/root/deopt-backup-offsite-v1.sh
runner_root=/root/deopt-first-production-backup-v2.sh
install -o root -g root -m 0700 "$backup_source" "$backup_root"
install -o root -g root -m 0700 "$runner_source" "$runner_root"
[[ "$(sha256sum "$backup_root" | cut -d ' ' -f 1)" == "$backup_hash" &&
   "$(sha256sum "$runner_root" | cut -d ' ' -f 1)" == "$runner_hash" &&
   "$(stat -c %U:%G:%a "$backup_root")" == root:root:700 &&
   "$(stat -c %U:%G:%a "$runner_root")" == root:root:700 ]] || {
  echo 'ROOT_COPY_INTEGRITY_FAIL' >&2; exit 2;
}
echo 'ROOT_OWNED_BACKUP_SCRIPTS=VERIFIED'
bash "$runner_root"
