"""Static safety checks for the future VPS release profile; no remote access."""

import pathlib
import os
import subprocess
import tempfile
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

    def test_r2_operator_setup_refuses_unprivileged_execution(self):
        result = subprocess.run(
            ["bash", str(ROOT / "deploy/vps/operator-setup-deopt-r2-vps.sh"), "0" * 40],
            capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 2)
        self.assertIn("Run as root", result.stderr)

    def test_r2_s3_http_cannot_prove_private_bucket_settings(self):
        script = str(ROOT / "deploy/vps/operator-setup-deopt-r2-vps.sh")
        for code, expected in (("400", "INCONCLUSIVE_HTTP_400"),
                               ("403", "INCONCLUSIVE_HTTP_403"),
                               ("404", "INCONCLUSIVE_HTTP_404"),
                               ("000", "INCONCLUSIVE_TRANSPORT")):
            with self.subTest(code=code):
                result = subprocess.run(["bash", script, "--classify-anonymous-s3-http", code],
                                        capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 0)
                self.assertEqual(result.stdout.strip(), expected)
        exposed = subprocess.run(["bash", script, "--classify-anonymous-s3-http", "200"],
                                 capture_output=True, text=True, check=False)
        self.assertNotEqual(exposed.returncode, 0)
        self.assertEqual(exposed.stdout.strip(), "PUBLIC_OBJECT_ACCESS_HTTP_200")

    def test_recovery_key_script_requires_explicit_operator_mode(self):
        result = subprocess.run(
            ["bash", str(ROOT / "deploy/vps/operator-deopt-backup-key-wsl.sh")],
            capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 2)
        self.assertIn("usage:", result.stderr)

    def test_coolify_backup_rejects_unreviewed_project_and_image(self):
        script = ROOT / "deploy/vps/backup-offsite.sh"
        with tempfile.TemporaryDirectory() as tmp:
            fake_docker = pathlib.Path(tmp) / "docker"
            fake_docker.write_text("#!/bin/sh\n"
                                   "case \"$1\" in\n"
                                   "ps) echo 0123456789ab;;\n"
                                   "inspect) echo postgres:16@sha256:wrong;;\n"
                                   "*) exit 99;;\n"
                                   "esac\n")
            fake_docker.chmod(0o700)
            env = os.environ.copy()
            env.update(PATH=f"{tmp}:{env['PATH']}", DEOPT_BACKUP_RECIPIENT="0" * 40,
                       DEOPT_BACKUP_REMOTE="remote:deopt", DEOPT_COMPOSE_PROJECT="coolify",
                       DEOPT_BACKUP_POSTGRES_IMAGE="postgres:16@sha256:5db6dac2ef5dcff512f4f00a009c3c332b9f2f28f24277175bf3e4a3d02ef83a")
            bad_project = subprocess.run(["bash", str(script)], env=env,
                                         capture_output=True, text=True, check=False)
            self.assertEqual(bad_project.returncode, 2)
            self.assertIn("differs from reviewed release", bad_project.stderr)
            env["DEOPT_COMPOSE_PROJECT"] = "../other"
            invalid = subprocess.run(["bash", str(script)], env=env,
                                     capture_output=True, text=True, check=False)
            self.assertEqual(invalid.returncode, 2)
            self.assertIn("Reviewed Coolify Compose project required", invalid.stderr)

    def test_coolify_backup_mock_retrieval_checks_uploaded_ciphertext(self):
        script = ROOT / "deploy/vps/backup-offsite.sh"
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            for name, body in {
                "docker": "case \"$1\" in\n"
                          "ps) echo 0123456789ab;;\n"
                          "inspect) echo \"$DEOPT_BACKUP_POSTGRES_IMAGE\";;\n"
                          "exec) printf fixture-pgdump;;\n"
                          "*) exit 99;;\n"
                          "esac\n",
                "gpg": "while [ \"$#\" -gt 0 ]; do\n"
                       "  if [ \"$1\" = --output ]; then shift; out=$1; fi\n"
                       "  shift\n"
                       "done\n"
                       "cat > \"$out\"\n",
                "rclone": "case \"$1\" in\n"
                          "copyto) printf '%s' \"$3\" > \"$DEOPT_TEST_BACKUP_PATH\"; cp \"$2\" \"$DEOPT_TEST_BACKUP_OBJECT\";;\n"
                          "cat) test \"$2\" = \"$(cat \"$DEOPT_TEST_BACKUP_PATH\")\" || exit 98; cat \"$DEOPT_TEST_BACKUP_OBJECT\";;\n"
                          "*) exit 99;;\n"
                          "esac\n",
            }.items():
                executable = root / name
                executable.write_text("#!/bin/sh\nset -eu\n" + body)
                executable.chmod(0o700)
            env = os.environ.copy()
            env.update(PATH=f"{tmp}:{env['PATH']}", DEOPT_BACKUP_RECIPIENT="0" * 40,
                       DEOPT_BACKUP_REMOTE="remote:deopt", DEOPT_COMPOSE_PROJECT="deopt_staging",
                       DEOPT_BACKUP_POSTGRES_IMAGE="postgres:16@sha256:5db6dac2ef5dcff512f4f00a009c3c332b9f2f28f24277175bf3e4a3d02ef83a",
                       DEOPT_TEST_BACKUP_OBJECT=str(root / "remote-ciphertext"),
                       DEOPT_TEST_BACKUP_PATH=str(root / "remote-path"))
            result = subprocess.run(["bash", str(script)], env=env,
                                    capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("UPLOADED_AND_RETRIEVED", result.stdout)
            self.assertEqual((root / "remote-ciphertext").read_bytes(), b"fixture-pgdump")
            self.assertRegex((root / "remote-path").read_text(),
                             r"^remote:deopt/\d{8}T\d{6}Z-[0-9a-f]{32}\.dump\.gpg$")


if __name__ == "__main__":
    unittest.main()
