"""Offline safety checks for the operator-run Netcup/Coolify preparation."""

import json
from pathlib import Path
import re
import shutil
import subprocess
from tempfile import TemporaryDirectory
import unittest

import yaml

from .prepare_coolify_vendor import install_port_guard, patch_cdn

ROOT = Path(__file__).resolve().parents[2]


class NetcupCoolifyTests(unittest.TestCase):
    def test_future_deopt_services_preserve_original_read_only_guards(self):
        original = yaml.safe_load((ROOT / "compose.vps-readonly.yml").read_text())
        future = yaml.safe_load((ROOT / "compose.coolify-readonly.yml").read_text())
        self.assertEqual(set(future["services"]), {"postgres", "backend"})
        for name in future["services"]:
            self.assertEqual(future["services"][name], original["services"][name])
            self.assertNotIn("ports", future["services"][name])
        self.assertEqual(future["networks"]["db"], {"internal": True})
        self.assertEqual(future["volumes"], {"postgres_data": {}})
        self.assertEqual(future["services"]["backend"]["environment"]["CHAIN_ID"], "84532")
        self.assertEqual(future["services"]["backend"]["environment"]["EXECUTION_ENABLED"], "false")

    def test_admin_override_replaces_public_ports_before_start(self):
        if not shutil.which("docker"):
            self.skipTest("Docker Compose unavailable")
        with TemporaryDirectory() as directory:
            base = Path(directory) / "base.yml"
            base.write_text('services:\n  coolify:\n    image: alpine:3\n    ports:\n      - "8000:8080"\n      - "6001:6001"\n      - "6002:6002"\n')
            result = subprocess.run([
                "docker", "compose", "-f", str(base),
                "-f", str(ROOT / "deploy/vps/coolify-loopback.override.yml"),
                "config", "--format", "json",
            ], text=True, capture_output=True, check=False)
            if result.returncode and "not a docker command" in result.stderr:
                self.skipTest("Docker Compose plugin unavailable")
            self.assertEqual(result.returncode, 0, result.stderr)
            ports = json.loads(result.stdout)["services"]["coolify"]["ports"]
            self.assertEqual(
                sorted((port["host_ip"], str(port["published"]), port["target"]) for port in ports),
                [("127.0.0.1", "6001", 6001), ("127.0.0.1", "6002", 6002), ("127.0.0.1", "8000", 8080)],
            )
            if shutil.which("jq"):
                marker = 'step_start "Pulling images" "coolify · postgres · redis · helper"'.encode()
                patched = install_port_guard(marker).decode()
                expression = patched.split("jq -e '\n", 1)[1].split("\n  ' >/dev/null", 1)[0]
                accepted = subprocess.run(["jq", "-e", expression], input=result.stdout,
                                          text=True, capture_output=True, check=False)
                self.assertEqual(accepted.returncode, 0, accepted.stderr)
                public = subprocess.run(["docker", "compose", "-f", str(base), "config", "--format", "json"],
                                        text=True, capture_output=True, check=True)
                rejected = subprocess.run(["jq", "-e", expression], input=public.stdout,
                                          text=True, capture_output=True, check=False)
                self.assertNotEqual(rejected.returncode, 0)

    def test_vendor_patches_fail_on_unreviewed_input(self):
        with self.assertRaises(ValueError):
            patch_cdn(b"CDN=other", Path("/root/reviewed"))
        with self.assertRaises(ValueError):
            install_port_guard(b"unreviewed installer")
        patched = install_port_guard('step_start "Pulling images" "coolify · postgres · redis · helper"'.encode())
        self.assertIn(b"Coolify private-port pre-start gate failed", patched)

    def test_privileged_script_refuses_implicit_execution(self):
        script = ROOT / "deploy/vps/netcup-docker-coolify-bootstrap.sh"
        subprocess.run(["bash", "-n", str(script)], check=True)
        result = subprocess.run(["bash", str(script)], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 2)
        self.assertIn("Run the reviewed root-owned copy", result.stderr)

    def test_root_ssh_password_check_accepts_only_key_only_values(self):
        script = (ROOT / "deploy/vps/netcup-docker-coolify-bootstrap.sh").read_text()
        self.assertIn("set -euo pipefail", script)
        self.assertIn("/usr/sbin/sshd -t", script)
        match = re.search(
            r"/usr/sbin/sshd -T -C \"user=root,host=localhost,addr=\$client_addr\" \|\s*"
            r"(grep -Eqx '[^']+') \|\|",
            script,
        )
        self.assertIsNotNone(match, "effective sshd check missing")
        command = match.group(1)
        for value, expected in (
            ("permitrootlogin prohibit-password\n", 0),
            ("permitrootlogin without-password\n", 0),
            ("permitrootlogin yes\n", 1),
            ("", 1),
        ):
            with self.subTest(value=value):
                result = subprocess.run(
                    ["bash", "-o", "pipefail", "-c", f"printf '%s' \"$1\" | {command}", "test", value],
                    capture_output=True, text=True, check=False,
                )
                self.assertEqual(result.returncode, expected)
        failed_sshd = subprocess.run(
            ["bash", "-o", "pipefail", "-c",
             f"{{ printf 'permitrootlogin prohibit-password\\n'; exit 7; }} | {command}"],
            capture_output=True, text=True, check=False,
        )
        self.assertNotEqual(failed_sshd.returncode, 0)


if __name__ == "__main__":
    unittest.main()
