#!/usr/bin/env bash
# One root-owned, reviewed intervention. Replaces only inactive backup units.
set -euo pipefail
set +x
umask 077

[[ ${EUID} == 0 && "$(hostname -s)" == v2202611448349549977 ]] || {
  echo 'REVIEWED_ROOT_HOST_REQUIRED' >&2; exit 2;
}
stage=/home/deoptops/deopt-backup-local.stage
manifest=$stage/production-backup-local-bundle.sha256
expected_manifest=27d937991b02794da78ec8bfa5979659588d940b30f0fff6bc45240cdd32a9e2
[[ -d "$stage" && ! -L "$stage" && "$(stat -c %U:%G:%a "$stage")" == deoptops:deoptops:700 ]] || {
  echo 'STAGE_DIRECTORY_UNSAFE' >&2; exit 2;
}
[[ -f "$manifest" && ! -L "$manifest" && "$(sha256sum "$manifest" | cut -d ' ' -f 1)" == "$expected_manifest" ]] || {
  echo 'BUNDLE_MANIFEST_INTEGRITY_FAIL' >&2; exit 2;
}
( cd "$stage" && sha256sum --check --status production-backup-local-bundle.sha256 ) || {
  echo 'STAGED_BUNDLE_INTEGRITY_FAIL' >&2; exit 2;
}
files=(
  run-daily-production-backup.sh
  check_backup_freshness.py
  systemd/no-discord/deopt-production-backup.service
  systemd/no-discord/deopt-production-backup.timer
  systemd/no-discord/deopt-production-backup-watchdog.service
  systemd/deopt-production-backup-watchdog.timer
)
for file in "${files[@]}"; do
  [[ -f "$stage/$file" && ! -L "$stage/$file" ]] || {
    echo 'STAGED_FILE_UNSAFE' >&2; exit 2;
  }
done

# Freeze the checked user-owned stage into a root-owned snapshot before use.
# Rechecking the snapshot closes the verification-to-installation race.
release=/root/deopt-backup-local-release-v1
[[ ! -e "$release" && ! -L "$release" ]] || {
  echo 'ROOT_RELEASE_DIRECTORY_ALREADY_EXISTS' >&2; exit 2;
}
install -d -o root -g root -m 0700 "$release" "$release/systemd" "$release/systemd/no-discord"
install -o root -g root -m 0600 "$manifest" "$release/production-backup-local-bundle.sha256"
for file in "${files[@]}"; do
  install -o root -g root -m 0600 "$stage/$file" "$release/$file"
done
[[ "$(sha256sum "$release/production-backup-local-bundle.sha256" | cut -d ' ' -f 1)" == "$expected_manifest" ]] || {
  echo 'ROOT_MANIFEST_INTEGRITY_FAIL' >&2; exit 2;
}
( cd "$release" && sha256sum --check --status production-backup-local-bundle.sha256 ) || {
  echo 'ROOT_RELEASE_INTEGRITY_FAIL' >&2; exit 2;
}

# The real, previously restored production backup engine is unchanged.
backup_script=/root/deopt-backup-offsite-v1.sh
[[ -f "$backup_script" && ! -L "$backup_script" &&
   "$(stat -c %U:%G:%a "$backup_script")" == root:root:700 &&
   "$(sha256sum "$backup_script" | cut -d ' ' -f 1)" == d132f8596042a4a1011257e98fd6a8d340f26407d1b0d4b0dc520f103b13a418 ]] || {
  echo 'REVIEWED_BACKUP_ENGINE_MISSING' >&2; exit 2;
}
for timer in deopt-production-backup.timer deopt-production-backup-watchdog.timer deopt-backup-alert-retry.timer; do
  if systemctl is-active --quiet "$timer"; then
    echo 'ACTIVE_BACKUP_TIMER_REQUIRES_REVIEW' >&2; exit 2;
  fi
done

# Pin the currently installed Discord-linked units before replacing them.
declare -A old_hash=(
  [deopt-production-backup.service]=ab6b720e90f1513738ee3b1313321bf184f772eb497659929a4b8685240a9e8a
  [deopt-production-backup.timer]=03daef0c80f76284b0b42ad64fb29f6606cdec3e49929ed909307320b06ab705
  [deopt-production-backup-watchdog.service]=d254e077c1792ffe31c28c421d8375e3d4b9a8e1e0bf2d725398f7b2abf51d18
  [deopt-production-backup-watchdog.timer]=b8c3238fcff59f4f63fffc1b0192f1a1c40015e2f2e0e032fb8b238039e2e9b0
)
for name in "${!old_hash[@]}"; do
  file=/etc/systemd/system/$name
  [[ -f "$file" && ! -L "$file" && "$(stat -c %U:%G:%a "$file")" == root:root:644 &&
     "$(sha256sum "$file" | cut -d ' ' -f 1)" == "${old_hash[$name]}" ]] || {
    echo "INSTALLED_UNIT_DRIFT=$name" >&2; exit 2;
  }
done
for pair in '/usr/local/sbin/deopt-production-backup a0db0e727007b71bb981e86deea75c0a1fb4e0387446d4f60f40ce55604562bc' \
            '/usr/local/sbin/deopt-backup-freshness 26f02a6a1e5f360b10aea512dcd26652b8cf0550474944a58d261c47b923d545'; do
  read -r file expected <<< "$pair"
  if [[ -e "$file" || -L "$file" ]]; then
    [[ -f "$file" && ! -L "$file" && "$(stat -c %U:%G:%a "$file")" == root:root:700 &&
       "$(sha256sum "$file" | cut -d ' ' -f 1)" == "$expected" ]] || {
      echo 'INSTALLED_EXECUTABLE_DRIFT' >&2; exit 2;
    }
  fi
done

rollback=/root/deopt-backup-local-rollback-v1
[[ ! -e "$rollback" && ! -L "$rollback" ]] || {
  echo 'ROLLBACK_DIRECTORY_ALREADY_EXISTS' >&2; exit 2;
}
install -d -o root -g root -m 0700 "$rollback"
for name in "${!old_hash[@]}"; do
  install -o root -g root -m 0644 "/etc/systemd/system/$name" "$rollback/$name"
  [[ "$(sha256sum "$rollback/$name" | cut -d ' ' -f 1)" == "${old_hash[$name]}" ]] || {
    echo 'ROLLBACK_COPY_INTEGRITY_FAIL' >&2; exit 2;
  }
done
rollback_ready=true
on_exit() {
  result=$?
  if [[ "$result" != 0 && "$rollback_ready" == true ]]; then
    systemctl disable --now deopt-production-backup.timer deopt-production-backup-watchdog.timer >/dev/null 2>&1 || true
    for name in "${!old_hash[@]}"; do
      install -o root -g root -m 0644 "$rollback/$name" "/etc/systemd/system/$name" || true
    done
    systemctl daemon-reload || true
    echo 'LOCAL_BACKUP_AUTOMATION_NOT_ACTIVE; PREVIOUS_UNITS_RESTORED' >&2
  fi
}
trap on_exit EXIT

dest_for() {
  case "$1" in
    run-daily-production-backup.sh) echo /usr/local/sbin/deopt-production-backup ;;
    check_backup_freshness.py) echo /usr/local/sbin/deopt-backup-freshness ;;
    systemd/*) echo "/etc/systemd/system/${1##*/}" ;;
    *) return 2 ;;
  esac
}
for file in "${files[@]}"; do
  dest=$(dest_for "$file")
  expected=$(awk -v name="$file" '$2 == name { print $1 }' "$release/production-backup-local-bundle.sha256")
  [[ "$expected" =~ ^[0-9a-f]{64}$ ]] || { echo 'BUNDLE_ENTRY_MISSING' >&2; exit 2; }
  case "$file" in systemd/*) mode=0644 ;; *) mode=0700 ;; esac
  install -o root -g root -m "$mode" "$release/$file" "$dest"
  [[ "$(sha256sum "$dest" | cut -d ' ' -f 1)" == "$expected" &&
     "$(stat -c %U:%G:%a "$dest")" == "root:root:${mode#0}" ]] || {
    echo 'ROOT_COPY_INTEGRITY_FAIL' >&2; exit 2;
  }
done
echo 'ROOT_OWNED_BUNDLE=VERIFIED'
systemctl daemon-reload
systemd-analyze verify \
  /etc/systemd/system/deopt-production-backup.service \
  /etc/systemd/system/deopt-production-backup.timer \
  /etc/systemd/system/deopt-production-backup-watchdog.service \
  /etc/systemd/system/deopt-production-backup-watchdog.timer
[[ -z "$(systemctl show --value --property=OnFailure deopt-production-backup.service)" &&
   -z "$(systemctl show --value --property=OnFailure deopt-production-backup-watchdog.service)" ]] || {
  echo 'DISCORD_FAILURE_HOOK_STILL_ACTIVE' >&2; exit 2;
}
echo 'SYSTEMD_UNIT_VERIFY=PASS; DISCORD_HOOK=ABSENT'

# A real encrypted backup seeds the freshness record before enabling timers.
systemctl start deopt-production-backup.service
python3 /usr/local/sbin/deopt-backup-freshness
systemctl start deopt-production-backup-watchdog.service
systemctl enable --now deopt-production-backup.timer deopt-production-backup-watchdog.timer
for timer in deopt-production-backup.timer deopt-production-backup-watchdog.timer; do
  systemctl is-enabled --quiet "$timer"
  systemctl is-active --quiet "$timer"
done
next=$(systemctl show --value --property=NextElapseUSecRealtime deopt-production-backup.timer)
[[ -n "$next" && "$next" != n/a ]] || { echo 'NEXT_BACKUP_NOT_SCHEDULED' >&2; exit 2; }
rollback_ready=false
echo 'BACKUP_TIMER=ACTIVE; WATCHDOG_TIMER=ACTIVE; DISCORD_RETRY_TIMER=INACTIVE'
printf 'NEXT_BACKUP=%s\n' "$next"
echo 'BACKUP_AUTOMATION=ACTIVE'
