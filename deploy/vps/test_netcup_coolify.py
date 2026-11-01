"""Offline safety checks for the operator-run Netcup/Coolify preparation."""

import json
import hashlib
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
from tempfile import TemporaryDirectory
import unittest

import yaml

from .prepare_coolify_vendor import (
    download_curl,
    install_port_guard,
    patch_cdn,
    verify_reviewed_content,
)

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

    def test_curl_recovers_reviewed_content_after_urllib_403(self):
        content = b"reviewed Coolify fixture"
        expected = hashlib.sha256(content).hexdigest()

        def successful_curl(command, **kwargs):
            self.assertIn("--fail", command)
            self.assertIn("--proto-redir", command)
            self.assertIn("=https", command)
            self.assertEqual(command[command.index("--retry") + 1], "2")
            Path(command[command.index("--output") + 1]).write_bytes(content)
            return subprocess.CompletedProcess(command, 0, "https://cdn.coollabs.io/coolify/install.sh", "")

        with TemporaryDirectory() as directory:
            fetched = download_curl("install.sh", Path(directory), runner=successful_curl)
            verify_reviewed_content("install.sh", fetched, expected)
            self.assertEqual(fetched, content)
            self.assertEqual(list(Path(directory).iterdir()), [])

    def test_curl_http_403_fails_without_accepting_body(self):
        def denied_curl(command, **kwargs):
            Path(command[command.index("--output") + 1]).write_bytes(b"403 response")
            return subprocess.CompletedProcess(command, 22, "https://cdn.coollabs.io/coolify/install.sh", "HTTP 403")

        with TemporaryDirectory() as directory:
            with self.assertRaisesRegex(RuntimeError, "curl 22") as error:
                download_curl("install.sh", Path(directory), runner=denied_curl)
            self.assertNotIn("403 response", str(error.exception))
            self.assertEqual(list(Path(directory).iterdir()), [])

    def test_curl_incomplete_transfer_fails_and_removes_partial_file(self):
        def interrupted_curl(command, **kwargs):
            Path(command[command.index("--output") + 1]).write_bytes(b"partial")
            return subprocess.CompletedProcess(command, 18, "https://cdn.coollabs.io/coolify/install.sh", "")

        with TemporaryDirectory() as directory:
            with self.assertRaisesRegex(RuntimeError, "curl 18"):
                download_curl("install.sh", Path(directory), runner=interrupted_curl)
            self.assertEqual(list(Path(directory).iterdir()), [])

    def test_curl_rejects_non_https_redirect(self):
        def redirected_curl(command, **kwargs):
            Path(command[command.index("--output") + 1]).write_bytes(b"content")
            return subprocess.CompletedProcess(command, 0, "http://example.invalid/install.sh", "")

        with TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, "outside HTTPS"):
                download_curl("install.sh", Path(directory), runner=redirected_curl)

    def test_reviewed_content_rejects_wrong_hash_and_incomplete_download(self):
        complete = b"reviewed Coolify fixture"
        expected = hashlib.sha256(complete).hexdigest()
        for content in (b"changed Coolify fixture", complete[:-1], b""):
            with self.subTest(content=content):
                with self.assertRaisesRegex(ValueError, "reviewed Coolify input changed"):
                    verify_reviewed_content("install.sh", content, expected)

    def test_privileged_script_refuses_implicit_execution(self):
        script = ROOT / "deploy/vps/netcup-docker-coolify-bootstrap.sh"
        subprocess.run(["bash", "-n", str(script)], check=True)
        result = subprocess.run(["bash", str(script)], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 2)
        self.assertIn("Run the reviewed root-owned copy", result.stderr)

    def run_root_ssh_policy_fixture(self, address, output, sshd_exit=0, large=False):
        script = (ROOT / "deploy/vps/netcup-docker-coolify-bootstrap.sh").read_text()
        self.assertIn("set -euo pipefail", script)
        self.assertIn("/usr/sbin/sshd -t", script)
        self.assertIn('if [[ "$ssh_dropin_created" == true ]]; then\n  systemctl reload ssh', script)
        self.assertIn("for client_addr in 203.0.113.5 2001:db8::5; do", script)
        match = re.search(r"(?ms)^verify_root_ssh_policy\(\) \{\n.*?^\}", script)
        self.assertIsNotNone(match, "effective sshd check missing")
        function = match.group()
        self.assertIn('effective=$(/usr/sbin/sshd -T -C', function)
        self.assertNotIn("| grep", function)
        with TemporaryDirectory() as directory:
            fake_sshd = Path(directory) / "sshd"
            fake_sshd.write_text(
                '#!/usr/bin/env bash\n'
                'set -e\n'
                '[[ "$*" == *"addr=$FAKE_EXPECT_ADDR" ]] || exit 77\n'
                'printf "%s" "$FAKE_SSH_OUTPUT"\n'
                'if [[ "$FAKE_LARGE" == 1 ]]; then\n'
                '  for ((i=0;i<8000;i++)); do printf "otherconfig %s\\n" "$i"; done\n'
                'fi\n'
                'exit "$FAKE_SSH_EXIT"\n'
            )
            fake_sshd.chmod(0o700)
            function = function.replace("/usr/sbin/sshd -T", shlex.quote(str(fake_sshd)) + " -T")
            env = dict(os.environ, FAKE_EXPECT_ADDR=address, FAKE_SSH_OUTPUT=output,
                       FAKE_SSH_EXIT=str(sshd_exit), FAKE_LARGE="1" if large else "0")
            return subprocess.run(
                ["bash", "-e", "-u", "-o", "pipefail", "-c",
                 function + '\nverify_root_ssh_policy "$1"', "test", address],
                env=env, capture_output=True, text=True, check=False,
            )

    def test_root_ssh_password_check_both_ip_families(self):
        for address in ("203.0.113.5", "2001:db8::5"):
            for policy in ("prohibit-password", "without-password"):
                with self.subTest(address=address, policy=policy):
                    result = self.run_root_ssh_policy_fixture(
                        address, f"permitrootlogin {policy}\notherconfig value\n")
                    self.assertEqual(result.returncode, 0, result.stderr)
            for output in ("permitrootlogin yes\n", "", "permitrootlogin forced-commands-only\n",
                           "permitrootlogin without-password\npermitrootlogin prohibit-password\n"):
                with self.subTest(address=address, output=output):
                    self.assertNotEqual(self.run_root_ssh_policy_fixture(address, output).returncode, 0)
            with self.subTest(address=address, sshd_exit=7):
                self.assertNotEqual(self.run_root_ssh_policy_fixture(
                    address, "permitrootlogin without-password\n", sshd_exit=7).returncode, 0)

    def test_root_ssh_check_avoids_pipefail_sigpipe_on_full_output(self):
        for address in ("203.0.113.5", "2001:db8::5"):
            result = self.run_root_ssh_policy_fixture(
                address, "permitrootlogin without-password\n", large=True)
            self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
