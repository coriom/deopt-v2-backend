#!/usr/bin/env bash
# Operator-only WSL actions. Never run this through Codex: it creates or uses
# the real private recovery key and prompts for passphrases via pinentry.
set +x
set -euo pipefail
umask 077
key_home="$HOME/.local/share/deopt-backup-gpg"
key_uid='DeOpt Backup Recovery'

fingerprint() {
  gpg --homedir "$key_home" --with-colons --list-secret-keys 2>/dev/null |
    awk -F: '$1=="sec" {primary=1; next} primary && $1=="fpr" {print $10; exit}'
}

case "${1:-}" in
  --create)
    [[ $# == 2 && "$2" == /mnt/* && ! -e "$2" && ! -L "$2" && -d "$(dirname "$2")" ]] || {
      echo 'Provide a new encrypted recovery-backup path on a Windows-mounted drive' >&2; exit 2;
    }
    [[ -t 0 ]] || { echo 'Interactive WSL terminal required' >&2; exit 2; }
    export GPG_TTY="$(tty)"
    install -d -m 0700 "$key_home"
    fpr="$(fingerprint)"
    if [[ -z "$fpr" ]]; then
      gpg --homedir "$key_home" --quick-generate-key "$key_uid" ed25519 cert 2y
      fpr="$(fingerprint)"
      [[ "$fpr" =~ ^[0-9A-Fa-f]{40,64}$ ]] || { echo 'GPG fingerprint unavailable' >&2; exit 2; }
      gpg --homedir "$key_home" --quick-add-key "$fpr" cv25519 encr 2y
    fi
    [[ "$fpr" =~ ^[0-9A-Fa-f]{40,64}$ ]] || { echo 'Invalid GPG fingerprint' >&2; exit 2; }
    gpg --homedir "$key_home" --armor --export "$fpr" >"$key_home/deopt-backup-public.asc"
    [[ -s "$key_home/deopt-backup-public.asc" ]] || { echo 'Public key export failed' >&2; exit 2; }
    # No plaintext secret-key export ever touches the filesystem.
    gpg --homedir "$key_home" --export-secret-keys "$fpr" |
      gpg --homedir "$key_home" --symmetric --cipher-algo AES256 --output "$2"
    [[ -s "$2" ]] || { echo 'Encrypted recovery export failed' >&2; exit 2; }
    verify_home="$(mktemp -d /tmp/deopt-key-restore.XXXXXXXX)"
    trap 'rm -rf -- "${verify_home:-}"' EXIT
    gpg --homedir "$key_home" --decrypt "$2" |
      gpg --homedir "$verify_home" --batch --import >/dev/null 2>&1
    restored="$(gpg --homedir "$verify_home" --with-colons --list-secret-keys 2>/dev/null |
      awk -F: '$1=="sec" {primary=1; next} primary && $1=="fpr" {print $10; exit}')"
    [[ "$restored" == "$fpr" ]] || { echo 'Encrypted key recovery test failed' >&2; exit 1; }
    printf 'GPG_FINGERPRINT=%s\nGPG_PUBLIC_KEY_FILE=%s\nENCRYPTED_KEY_EXPORT=PASS_WINDOWS_PATH\nPRIVATE_KEY_OFF_VPS=YES\n' \
      "$fpr" "$key_home/deopt-backup-public.asc"
    ;;
  --verify-probe)
    [[ $# == 3 && "$2" =~ ^probe\.[A-Za-z0-9]{8}$ && "$3" =~ ^[0-9a-f]{64}$ ]] || {
      echo 'Expected probe directory and original plaintext SHA-256 required' >&2; exit 2;
    }
    [[ -t 0 ]] || { echo 'Interactive WSL terminal required' >&2; exit 2; }
    export GPG_TTY="$(tty)"
    tmp="$(mktemp -d /tmp/deopt-r2-verify.XXXXXXXX)"
    trap 'rm -rf -- "${tmp:-}"' EXIT
    scp -o BatchMode=yes -o StrictHostKeyChecking=yes -o ForwardAgent=no \
      "deopt-vps:deopt-bootstrap.stage/$2/probe.gpg" "$tmp/probe.gpg" >/dev/null
    gpg --homedir "$key_home" --batch --decrypt --output "$tmp/plain" "$tmp/probe.gpg" >/dev/null
    actual="$(sha256sum "$tmp/plain" | cut -d ' ' -f 1)"
    [[ "$actual" == "$3" ]] || { echo 'Restored synthetic data mismatch' >&2; exit 1; }
    echo 'OFF_HOST_DECRYPTION=PASS; RESTORED_DATA_MATCH=PASS'
    ;;
  *)
    echo 'usage: operator-deopt-backup-key-wsl.sh --create /mnt/<drive>/.../recovery.gpg | --verify-probe probe.XXXXXXXX ORIGINAL_SHA256' >&2
    exit 2
    ;;
esac
