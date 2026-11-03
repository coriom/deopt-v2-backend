"""Release checks for the Discord-free production backup systemd bundle."""

from pathlib import Path
import subprocess
import unittest


ROOT = Path(__file__).resolve().parent
LOCAL = ROOT / "systemd" / "no-discord"
MANIFEST = ROOT / "production-backup-local-bundle.sha256"


class LocalBackupReleaseTests(unittest.TestCase):
    def test_bundle_integrity(self):
        subprocess.run(
            ["sha256sum", "--check", "--status", MANIFEST.name],
            cwd=ROOT,
            check=True,
        )

    def test_no_discord_failure_dependency(self):
        for name in (
            "deopt-production-backup.service",
            "deopt-production-backup-watchdog.service",
        ):
            body = (LOCAL / name).read_text()
            self.assertNotIn("OnFailure=", body)
            self.assertNotIn("discord", body.lower())
            self.assertIn("StandardError=journal", body)

    def test_schedule_and_nonoverlap(self):
        timer = (LOCAL / "deopt-production-backup.timer").read_text()
        self.assertIn("OnCalendar=*-*-* 02:00:00 UTC", timer)
        self.assertIn("RandomizedDelaySec=15min", timer)
        self.assertIn("Persistent=true", timer)
        runner = (ROOT / "run-daily-production-backup.sh").read_text()
        self.assertIn("flock -n 9", runner)
        self.assertIn('bash "$backup_script"', runner)
        self.assertLess(runner.index('bash "$backup_script"'), runner.index('state_file="$(mktemp'))

    def test_installer_guards_and_no_webhook(self):
        installer = (ROOT / "operator-install-production-backup-local-v1.sh").read_text()
        self.assertIn("STAGED_BUNDLE_INTEGRITY_FAIL", installer)
        self.assertIn("ROOT_MANIFEST_INTEGRITY_FAIL", installer)
        self.assertIn("ROOT_RELEASE_INTEGRITY_FAIL", installer)
        self.assertIn('install -o root -g root -m "$mode" "$release/$file" "$dest"', installer)
        self.assertIn("INSTALLED_UNIT_DRIFT", installer)
        self.assertIn("PREVIOUS_UNITS_RESTORED", installer)
        self.assertIn("DISCORD_FAILURE_HOOK_STILL_ACTIVE", installer)
        self.assertIn("systemctl enable --now deopt-production-backup.timer", installer)
        self.assertNotIn("backup-discord-webhook", installer)
        self.assertNotIn("read -r -s", installer)
        self.assertNotIn("systemctl enable --now deopt-backup-alert-retry.timer", installer)


if __name__ == "__main__":
    unittest.main()
