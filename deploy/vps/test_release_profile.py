"""Static safety checks for the future VPS release profile; no remote access."""

import pathlib
import os
import subprocess
import unittest

import yaml

ROOT = pathlib.Path(__file__).resolve().parents[2]


class ReadOnlyProfile(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.profile = yaml.safe_load((ROOT / "compose.vps-readonly.yml").read_text())
        cls.services = cls.profile["services"]

    def test_only_proxy_has_public_ports(self):
        self.assertEqual(set(self.services), {"postgres", "backend", "caddy"})
        self.assertNotIn("ports", self.services["postgres"])
        self.assertNotIn("ports", self.services["backend"])
        self.assertEqual(self.services["postgres"]["networks"], ["db"])
        self.assertTrue(self.profile["networks"]["db"]["internal"])
        self.assertEqual(self.services["caddy"]["ports"], ["80:80", "443:443"])

    def test_backend_is_resource_bounded_and_non_root(self):
        backend = self.services["backend"]
        self.assertEqual(backend["user"], "1001:1001")
        self.assertTrue(backend["read_only"])
        self.assertEqual(backend["cap_drop"], ["ALL"])
        self.assertIn("no-new-privileges:true", backend["security_opt"])
        self.assertLessEqual(int(backend["mem_limit"].lower().removesuffix("m")), 2048)
        self.assertEqual(backend["logging"]["options"], {"max-size": "10m", "max-file": "5"})
        self.assertFalse(any("docker.sock" in str(v) for v in backend.values()))
        self.assertIn("/ready", backend["healthcheck"]["test"][-1])

    def test_mutation_flags_and_chain_are_explicitly_closed(self):
        env = self.services["backend"]["environment"]
        self.assertEqual(env["CHAIN_ID"], "84532")
        self.assertEqual(env["DEOPT_READ_ONLY_RELEASE"], "true")
        self.assertEqual(env["PERSISTENCE_ENABLED"], "true")
        for name in ("EXECUTION_ENABLED", "EXECUTOR_REAL_BROADCAST_ENABLED", "HV2_BROADCAST_ENABLED",
                     "PERPS_PUBLIC_TRADING_ENABLED", "PERPS_CLOSED_TEST_ENABLED", "OPTIONS_ENABLED",
                     "INDEXER_ENABLED", "PERPS_READ_ENABLED", "PUBLIC_WS_ENABLED",
                     "METRICS_ENABLED", "MM_GATEWAY_ENABLED"):
            self.assertEqual(env[name], "false", name)
        self.assertFalse(any("PRIVATE_KEY" in key or "SIGNER_KEY" in key for key in env))

    def test_images_and_secret_source_are_external(self):
        for service in self.services.values():
            self.assertIn("image", service)
            self.assertIn("${", service["image"])
        self.assertEqual(self.profile["secrets"]["postgres_password"]["file"],
                         "${DEOPT_POSTGRES_PASSWORD_FILE:?set absolute path to host-only password file}")

    def test_remote_script_refuses_unapproved_execution(self):
        env = os.environ.copy()
        env.pop("DEOPT_VPS_DEPLOY_AUTHORIZED", None)
        result = subprocess.run(["bash", str(ROOT / "deploy/vps/deploy-readonly.sh"), "--execute"],
                                env=env, capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 2)
        self.assertIn("Separate VPS deployment authorization required", result.stderr)

    def test_entrypoint_refuses_non_read_only_mode(self):
        env = os.environ.copy()
        env["DEOPT_READ_ONLY_RELEASE"] = "false"
        env["CHAIN_ID"] = "84532"
        result = subprocess.run(["sh", str(ROOT / "deploy/vps/backend-entrypoint.sh")],
                                env=env, capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 2)

    def test_backup_requires_operator_destination(self):
        env = os.environ.copy()
        env["DEOPT_BACKUP_RECIPIENT"] = "0" * 40
        env.pop("DEOPT_BACKUP_REMOTE", None)
        result = subprocess.run(["bash", str(ROOT / "deploy/vps/backup-offsite.sh")],
                                env=env, capture_output=True, text=True, check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("DEOPT_BACKUP_REMOTE", result.stderr)


if __name__ == "__main__":
    unittest.main()
