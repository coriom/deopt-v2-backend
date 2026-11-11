#!/usr/bin/env bash
# Operator-only, read-only SSH/R2 download into WSL. No file is written on Netcup.
set -euo pipefail
set +x
umask 077
[[ -t 0 && "$(hostname -s)" != v2202611448349549977 ]] || exit 2
for tool in ssh sha256sum mktemp; do command -v "$tool" >/dev/null || exit 2; done
partial=$(mktemp /tmp/deopt-v20-r2.XXXXXXXX.dump.gpg)
log=$(mktemp /tmp/deopt-v20-r2.XXXXXXXX.log)
cleanup() { rm -f -- "$partial" "$log"; }
trap cleanup EXIT
read -rsp 'Netcup sudo password (WSL terminal, hidden): ' sudo_password </dev/tty
printf '\n' >/dev/tty
remote_command=$(cat <<'REMOTE'
sudo -S bash -c 'set -euo pipefail
export RCLONE_CONFIG=/root/.config/deopt/rclone.conf
root=deopt_r2:deopt-private-backups/postgres
latest=$(rclone lsf "$root" --files-only | grep -E "^20[0-9]{6}T[0-9]{6}Z-[0-9a-f]{32}\.dump\.gpg$" | sort | tail -n 1)
[[ -n "$latest" ]]
rclone cat "$root/$latest" | tee >(sha256sum >&2)
wait'
REMOTE
)
if ! printf '%s\n' "$sudo_password" | ssh -o BatchMode=yes -o StrictHostKeyChecking=yes \
  -o ConnectTimeout=8 deopt-vps "$remote_command" >"$partial" 2>"$log"; then
  unset sudo_password
  echo 'R2_CIPHERTEXT_FETCH=BLOCKED' >&2
  exit 2
fi
unset sudo_password
remote_hash=$(grep -Eo '[0-9a-f]{64}  -' "$log" | tail -n 1 | cut -d' ' -f1 || true)
local_hash=$(sha256sum "$partial" | cut -d' ' -f1)
[[ -n "$remote_hash" && "$remote_hash" == "$local_hash" && -s "$partial" ]] || {
  echo 'R2_CIPHERTEXT_INTEGRITY=FAIL' >&2
  [[ -n "$remote_hash" ]] && echo REMOTE_HASH_PRESENT=YES >&2 || echo REMOTE_HASH_PRESENT=NO >&2
  [[ -s "$partial" ]] && echo CIPHERTEXT_NONEMPTY=YES >&2 || echo CIPHERTEXT_NONEMPTY=NO >&2
  exit 2
}
destination="/tmp/deopt-v20-latest-$(date -u +%Y%m%dT%H%M%SZ).dump.gpg"
[[ ! -e "$destination" ]] || exit 2
mv -- "$partial" "$destination"
chmod 0600 "$destination"
echo "R2_CIPHERTEXT_FETCH=PASS"
echo "R2_CIPHERTEXT_PATH=$destination"
echo "R2_CIPHERTEXT_SHA256=$local_hash"
echo "R2_CIPHERTEXT_SIZE=$(stat -c %s "$destination")"
