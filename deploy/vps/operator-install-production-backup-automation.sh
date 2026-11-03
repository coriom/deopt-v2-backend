#!/usr/bin/env bash
# One operator sudo intervention. No webhook URL is accepted in argv or logs.
set -euo pipefail
set +x
umask 077
[[ ${EUID} == 0 && "$(hostname -s)" == v2202611448349549977 ]] || {
  echo 'REVIEWED_ROOT_HOST_REQUIRED' >&2; exit 2;
}
stage=/home/deoptops/deopt-backup-automation.stage
manifest=$stage/production-backup-bundle.sha256
expected_manifest=7f9b11ce834c38318e66de02e6f6259769eff54d155cb156ff7f726396d6495c
[[ -d "$stage" && ! -L "$stage" && "$(stat -c %U:%G:%a "$stage")" == deoptops:deoptops:700 ]] || {
  echo 'STAGE_DIRECTORY_NOT_REVIEWED' >&2; exit 2;
}
[[ -f "$manifest" && ! -L "$manifest" &&
   "$(sha256sum "$manifest" | cut -d ' ' -f 1)" == "$expected_manifest" ]] || {
  echo 'BUNDLE_MANIFEST_INTEGRITY_FAIL' >&2; exit 2;
}
(
  cd "$stage"
  sha256sum --check --status production-backup-bundle.sha256
) || { echo 'STAGED_BUNDLE_INTEGRITY_FAIL' >&2; exit 2; }
files=(
  run-daily-production-backup.sh
  discord_backup_alert.py
  check_backup_freshness.py
  systemd/deopt-production-backup.service
  systemd/deopt-production-backup.timer
  systemd/deopt-production-backup-watchdog.service
  systemd/deopt-production-backup-watchdog.timer
  systemd/deopt-backup-alert@.service
  systemd/deopt-backup-failure-probe.service
  systemd/deopt-backup-alert-retry.service
  systemd/deopt-backup-alert-retry.timer
)
dest_for() {
  case "$1" in
    run-daily-production-backup.sh) echo /usr/local/sbin/deopt-production-backup ;;
    discord_backup_alert.py) echo /usr/local/sbin/deopt-backup-discord-alert ;;
    check_backup_freshness.py) echo /usr/local/sbin/deopt-backup-freshness ;;
    systemd/*) echo "/etc/systemd/system/${1##*/}" ;;
    *) return 2 ;;
  esac
}
for file in "${files[@]}"; do
  source=$stage/$file
  target=$(dest_for "$file")
  [[ -f "$source" && ! -L "$source" ]] || { echo 'STAGED_FILE_UNSAFE' >&2; exit 2; }
  if [[ -e "$target" || -L "$target" ]]; then
    [[ -f "$target" && ! -L "$target" &&
       "$(sha256sum "$target" | cut -d ' ' -f 1)" == "$(sha256sum "$source" | cut -d ' ' -f 1)" ]] || {
      echo 'EXISTING_SYSTEM_FILE_CONFLICT' >&2; exit 2;
    }
  fi
done
backup_script=/root/deopt-backup-offsite-v1.sh
[[ -f "$backup_script" && ! -L "$backup_script" &&
   "$(stat -c %U:%G:%a "$backup_script")" == root:root:700 &&
   "$(sha256sum "$backup_script" | cut -d ' ' -f 1)" == d132f8596042a4a1011257e98fd6a8d340f26407d1b0d4b0dc520f103b13a418 ]] || {
  echo 'REVIEWED_BACKUP_SCRIPT_MISSING' >&2; exit 2;
}
for timer in deopt-production-backup.timer deopt-production-backup-watchdog.timer deopt-backup-alert-retry.timer; do
  if systemctl is-active --quiet "$timer"; then
    echo 'EXISTING_ACTIVE_BACKUP_TIMER_REQUIRES_SEPARATE_REVIEW' >&2
    exit 2
  fi
done
on_exit() {
  result=$?
  if [[ "$result" != 0 ]]; then
    systemctl disable --now deopt-production-backup.timer deopt-production-backup-watchdog.timer deopt-backup-alert-retry.timer >/dev/null 2>&1 || true
    echo 'AUTOMATION_NOT_ACTIVE; ROLLBACK=TIMERS_DISABLED' >&2
  fi
}
trap on_exit EXIT
for file in "${files[@]}"; do
  source=$stage/$file
  target=$(dest_for "$file")
  case "$file" in systemd/*) mode=0644 ;; *) mode=0700 ;; esac
  expected_file=$(awk -v name="$file" '$2 == name { print $1 }' "$manifest")
  [[ "$expected_file" =~ ^[0-9a-f]{64}$ ]] || {
    echo 'BUNDLE_ENTRY_MISSING' >&2; exit 2;
  }
  install -o root -g root -m "$mode" "$source" "$target"
  [[ "$(sha256sum "$target" | cut -d ' ' -f 1)" == "$expected_file" &&
     "$(stat -c %U:%G:%a "$target")" == "root:root:${mode#0}" ]] || {
    echo 'ROOT_COPY_INTEGRITY_FAIL' >&2; exit 2;
  }
done
echo 'ROOT_OWNED_BUNDLE=VERIFIED'
config_dir=/etc/deopt
[[ ! -e "$config_dir" || ( -d "$config_dir" && ! -L "$config_dir" ) ]] || {
  echo 'SECRET_DIRECTORY_CONFLICT' >&2; exit 2;
}
install -d -o root -g root -m 0700 "$config_dir"
[[ "$(stat -c %U:%G:%a "$config_dir")" == root:root:700 ]] || {
  echo 'SECRET_DIRECTORY_UNSAFE' >&2; exit 2;
}
secret_file=$config_dir/backup-discord-webhook
if [[ ! -e "$secret_file" && ! -L "$secret_file" ]]; then
  printf 'Collez l’URL du webhook Discord (saisie masquée) : ' >/dev/tty
  IFS= read -r -s webhook </dev/tty
  printf '\n' >/dev/tty
  [[ "$webhook" =~ ^https://discord\.com/api/webhooks/[0-9]{15,25}/[A-Za-z0-9._-]{20,256}$ ]] || {
    unset webhook
    echo 'WEBHOOK_FORMAT_REJECTED' >&2; exit 2;
  }
  temporary=$(mktemp "$config_dir/.webhook.XXXXXXXX")
  printf '%s\n' "$webhook" > "$temporary"
  unset webhook
  chmod 0600 "$temporary"
  mv -T -- "$temporary" "$secret_file"
fi
[[ -f "$secret_file" && ! -L "$secret_file" && "$(stat -c %U:%G:%a "$secret_file")" == root:root:600 ]] || {
  echo 'WEBHOOK_SECRET_PERMISSIONS_FAIL' >&2; exit 2;
}
echo 'WEBHOOK_SECRET=ROOT_ONLY_NO_VALUE_PRINTED'
systemctl daemon-reload
units=(
  /etc/systemd/system/deopt-production-backup.service
  /etc/systemd/system/deopt-production-backup.timer
  /etc/systemd/system/deopt-production-backup-watchdog.service
  /etc/systemd/system/deopt-production-backup-watchdog.timer
  /etc/systemd/system/deopt-backup-alert@.service
  /etc/systemd/system/deopt-backup-failure-probe.service
  /etc/systemd/system/deopt-backup-alert-retry.service
  /etc/systemd/system/deopt-backup-alert-retry.timer
)
systemd-analyze verify "${units[@]}"
echo 'SYSTEMD_UNIT_VERIFY=PASS'
[[ ! -e /var/lib/deopt-backup || ( -d /var/lib/deopt-backup && ! -L /var/lib/deopt-backup ) ]] || {
  echo 'BACKUP_STATE_DIRECTORY_CONFLICT' >&2; exit 2;
}
install -d -o root -g root -m 0700 /var/lib/deopt-backup
rm -f -- /run/deopt-backup-alert-probe-success
systemctl reset-failed deopt-backup-failure-probe.service >/dev/null 2>&1 || true
if systemctl start deopt-backup-failure-probe.service >/dev/null 2>&1; then
  echo 'SYNTHETIC_FAILURE_PROBE_DID_NOT_FAIL' >&2; exit 2
fi
for _ in $(seq 1 60); do
  [[ -f /run/deopt-backup-alert-probe-success ]] && break
  sleep 1
done
[[ -f /run/deopt-backup-alert-probe-success &&
   "$(stat -c %U:%G:%a /run/deopt-backup-alert-probe-success)" == root:root:600 ]] || {
  echo 'DISCORD_SYNTHETIC_DELIVERY_NOT_CONFIRMED' >&2; exit 2;
}
systemctl reset-failed deopt-backup-failure-probe.service >/dev/null 2>&1 || true
echo 'DISCORD_SYNTHETIC_ONFAILURE=POSTED'
printf 'Vérifiez le message FAIL synthétique dans Discord, puis tapez RECEIVED : ' >/dev/tty
IFS= read -r confirmation </dev/tty
[[ "$confirmation" == RECEIVED ]] || { echo 'DISCORD_OPERATOR_RECEIPT_NOT_CONFIRMED' >&2; exit 2; }
echo 'DISCORD_OPERATOR_RECEIPT=CONFIRMED'
systemctl start deopt-production-backup.service
python3 /usr/local/sbin/deopt-backup-freshness
systemctl start deopt-production-backup-watchdog.service
systemctl enable --now deopt-production-backup.timer deopt-production-backup-watchdog.timer deopt-backup-alert-retry.timer
for timer in deopt-production-backup.timer deopt-production-backup-watchdog.timer deopt-backup-alert-retry.timer; do
  systemctl is-enabled --quiet "$timer"
  systemctl is-active --quiet "$timer"
done
next=$(systemctl show --value --property=NextElapseUSecRealtime deopt-production-backup.timer)
[[ -n "$next" && "$next" != n/a ]] || { echo 'NEXT_BACKUP_NOT_SCHEDULED' >&2; exit 2; }
echo 'BACKUP_TIMER=ACTIVE; WATCHDOG_TIMER=ACTIVE; ALERT_RETRY_TIMER=ACTIVE'
printf 'NEXT_BACKUP=%s\n' "$next"
echo 'AUTOMATION=ACTIVE'
