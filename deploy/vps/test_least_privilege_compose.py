"""Focused checks for the candidate socket-only, least-privilege release."""

import copy
import json
import os
from pathlib import Path
import subprocess
import unittest

from .check_coolify_private_compose import PrivateComposeError, check

ROOT = Path(__file__).resolve().parents[2]


class LeastPrivilegeComposeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.release = {
            "backend_image": "ghcr.io/coriom/deopt-v2-backend@sha256:" + "a" * 64,
            "postgres_image": "postgres:16@sha256:" + "b" * 64,
            "cors_origin": "https://deoptv2-frontend.vercel.app",
            "postgres_password_file": "/tmp/deopt-admin-test",
            "runtime_password_file": "/tmp/deopt-runtime-test",
            "backup_password_file": "/tmp/deopt-backup-test",
        }
        env = dict(os.environ)
        env.update({
            "DEOPT_LEAST_PRIVILEGE_BACKEND_IMAGE": cls.release["backend_image"],
            "DEOPT_POSTGRES_IMAGE": cls.release["postgres_image"],
            "DEOPT_CORS_ORIGIN": cls.release["cors_origin"],
            "DEOPT_POSTGRES_PASSWORD_FILE": cls.release["postgres_password_file"],
            "DEOPT_RUNTIME_PASSWORD_FILE": cls.release["runtime_password_file"],
            "DEOPT_BACKUP_PASSWORD_FILE": cls.release["backup_password_file"],
        })
        output = subprocess.run(
            ["docker", "compose", "-f", str(ROOT / "compose.coolify-least-privilege-readonly.yml"),
             "config", "--format", "json"],
            env=env, capture_output=True, text=True, check=True,
        )
        cls.rendered = json.loads(output.stdout)

    def test_valid_candidate(self):
        check(self.rendered, self.release, least_privilege_mode=True)

    def test_runtime_secret_cannot_alias_admin_or_backup(self):
        for path in (self.release["postgres_password_file"], self.release["backup_password_file"]):
            candidate = copy.deepcopy(self.rendered)
            candidate["secrets"]["runtime_password"]["file"] = path
            with self.assertRaises(PrivateComposeError):
                check(candidate, self.release, least_privilege_mode=True)

    def test_runtime_secret_cannot_drift_from_manifest(self):
        candidate = copy.deepcopy(self.rendered)
        candidate["secrets"]["runtime_password"]["file"] = "/tmp/other-runtime"
        with self.assertRaisesRegex(PrivateComposeError, "RUNTIME_RELEASE_SECRET_PATH"):
            check(candidate, self.release, least_privilege_mode=True)

    def test_postgres_network_and_public_ports_rejected(self):
        for change in ("network", "port"):
            candidate = copy.deepcopy(self.rendered)
            if change == "network":
                candidate["services"]["postgres"]["network_mode"] = None
                candidate["services"]["postgres"]["networks"] = ["web"]
            else:
                candidate["services"]["backend"]["ports"] = ["8080:8080"]
            with self.assertRaises(PrivateComposeError):
                check(candidate, self.release, least_privilege_mode=True)


if __name__ == "__main__":
    unittest.main()
