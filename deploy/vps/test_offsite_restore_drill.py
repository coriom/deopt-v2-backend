"""Safety checks for the future offsite restore drill; no real DB or remote."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "deploy/vps/restore-offsite-drill.sh"


class RestoreDrillTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        path = Path(self.tmp.name)
        for name, body in {
            "psql": "echo \"${DEOPT_TEST_TABLES:-0}\"\n",
            "rclone": "printf encrypted-fixture\n",
            "gpg": "cat\n",
            "pg_restore": "cat > \"$DEOPT_TEST_RESTORED\"\n",
        }.items():
            file = path / name
            file.write_text("#!/bin/sh\nset -eu\n" + body)
            file.chmod(0o700)
        self.target = path / "restored"
        self.env = dict(os.environ)
        self.env.update(PATH=f"{path}:{self.env['PATH']}",
                        DEOPT_BACKUP_OBJECT="remote:backups/fixture.dump.gpg",
                        DEOPT_RESTORE_DB="deopt_alpha_test", PGHOST="127.0.0.1",
                        PGPORT="5433", PGUSER="test", DEOPT_TEST_RESTORED=str(self.target))

    def run_script(self):
        return subprocess.run(["bash", str(SCRIPT)], env=self.env,
                              capture_output=True, text=True, check=False)

    def test_local_empty_target_receives_decrypted_stream(self):
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.target.read_bytes(), b"encrypted-fixture")
        self.assertIn("LOGICAL_INTEGRITY_REVIEW_REQUIRED", result.stdout)

    def test_refuses_remote_or_non_disposable_target(self):
        self.env["PGHOST"] = "db.production.example"
        self.assertEqual(self.run_script().returncode, 2)
        self.env["PGHOST"] = "127.0.0.1"
        self.env["DEOPT_RESTORE_DB"] = "deopt_production"
        self.assertEqual(self.run_script().returncode, 2)
        self.assertFalse(self.target.exists())

    def test_refuses_nonempty_target(self):
        self.env["DEOPT_TEST_TABLES"] = "1"
        result = self.run_script()
        self.assertEqual(result.returncode, 2)
        self.assertIn("not empty", result.stderr)
        self.assertFalse(self.target.exists())

    def test_local_ciphertext_restore_and_permissions(self):
        cipher = Path(self.tmp.name) / "download.dump.gpg"
        cipher.write_bytes(b"encrypted-local-fixture")
        cipher.chmod(0o600)
        self.env["DEOPT_BACKUP_LOCAL_CIPHERTEXT"] = str(cipher)
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.target.read_bytes(), b"encrypted-local-fixture")
        self.target.unlink()
        cipher.chmod(0o644)
        result = self.run_script()
        self.assertEqual(result.returncode, 2)
        self.assertIn("mode-0600", result.stderr)
        self.assertFalse(self.target.exists())


if __name__ == "__main__":
    unittest.main()
