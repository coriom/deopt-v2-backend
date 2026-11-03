"""Offline tests for authenticated package inventory classification."""

import fcntl
import os
from pathlib import Path
import pty
import select
import subprocess
import tempfile
import termios
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "deploy/vps/ghcr-private-first-publish.sh"


class FirstPublishTests(unittest.TestCase):
    def classify(self, list_status="200", names="", detail_status="200",
                 visibility="private"):
        with tempfile.TemporaryDirectory() as folder:
            gh = Path(folder) / "gh"
            gh.write_text(
                "#!/usr/bin/env python3\n"
                "import os, sys\n"
                "args = sys.argv[1:]\n"
                "endpoint = next((x for x in args if x.startswith('user/packages')), '')\n"
                "if '-i' in args:\n"
                "  status = os.environ['MOCK_DETAIL_STATUS'] if '/container/' in endpoint "
                "else os.environ['MOCK_LIST_STATUS']\n"
                "  print('HTTP/2 ' + status)\n"
                "  sys.exit(0 if status == '200' else 1)\n"
                "if '--paginate' in args:\n"
                "  print(os.environ['MOCK_NAMES'])\n"
                "  sys.exit(0)\n"
                "if '/container/' in endpoint:\n"
                "  print(os.environ['MOCK_VISIBILITY'])\n"
                "  sys.exit(0)\n"
                "sys.exit(9)\n"
            )
            gh.chmod(0o700)
            env = dict(os.environ)
            env.update(
                PATH=f"{folder}:{env['PATH']}",
                TEST_SESSION_DIR=folder,
                MOCK_LIST_STATUS=list_status,
                MOCK_NAMES=names,
                MOCK_DETAIL_STATUS=detail_status,
                MOCK_VISIBILITY=visibility,
            )
            command = (
                'source "$1"; pat=test-only; session_dir="$TEST_SESSION_DIR"; '
                'classify_package'
            )
            result = subprocess.run(
                ["bash", "-c", command, "bash", str(SCRIPT)],
                env=env, text=True, capture_output=True, check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            return result.stdout.strip()

    def test_authenticated_empty_inventory_is_absent(self):
        self.assertEqual(self.classify(), "ABSENT")

    def test_access_denied_is_distinct(self):
        self.assertEqual(self.classify(list_status="403"), "ACCESS_DENIED")
        self.assertEqual(self.classify(list_status="401"), "ACCESS_DENIED")

    def test_existing_private_package(self):
        self.assertEqual(self.classify(names="deopt-v2-backend"), "EXISTS_PRIVATE")

    def test_existing_public_package(self):
        self.assertEqual(
            self.classify(names="deopt-v2-backend", visibility="public"),
            "EXISTS_PUBLIC",
        )

    def test_inconsistent_or_unknown_inventory_fails_closed(self):
        self.assertEqual(self.classify(list_status="500"), "UNKNOWN")
        self.assertEqual(
            self.classify(names="deopt-v2-backend", detail_status="404"),
            "UNKNOWN",
        )
        self.assertEqual(
            self.classify(names="deopt-v2-backend", visibility="internal"),
            "UNKNOWN",
        )

    def test_no_public_source_label_in_first_publish(self):
        script = SCRIPT.read_text()
        self.assertIn("--provenance=false", script)
        self.assertIn("PUBLIC_REPOSITORY_SOURCE_LABEL_FORBIDDEN", script)
        self.assertNotIn('--label "org.opencontainers.image.source=', script)
        self.assertIn("DOCKER_CONFIG=\"$session_dir/docker\"", script)
        self.assertIn("ANONYMOUS_PULL_INCONCLUSIVE", script)

    def exercise_interactive_path(self, update=False):
        commit = "a" * 40
        digest = "sha256:" + "b" * 64
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            state = root / "package-state"
            if update:
                state.write_text("present")
            config_path = root / "config-path"
            (root / "git").write_text(
                "#!/bin/sh\n"
                "case \"$*\" in\n"
                "  'rev-parse --show-toplevel') echo \"$MOCK_REPO_ROOT\" ;;\n"
                "  'remote get-url origin') echo git@github.com:coriom/deopt-v2-backend.git ;;\n"
                "  'branch --show-current') echo main ;;\n"
                "  'status --porcelain') : ;;\n"
                "  'rev-parse HEAD') echo \"$MOCK_COMMIT\" ;;\n"
                "  'ls-remote origin refs/heads/main') "
                "echo \"$MOCK_COMMIT refs/heads/main\" ;;\n"
                "  *) exit 9 ;;\n"
                "esac\n"
            )
            (root / "gh").write_text(
                "#!/usr/bin/env python3\n"
                "import os, pathlib, sys\n"
                "a = sys.argv[1:]\n"
                "state = pathlib.Path(os.environ['MOCK_PACKAGE_STATE'])\n"
                "endpoint = next((x for x in a if x.startswith('user')), '')\n"
                "if '-i' in a and endpoint == 'user':\n"
                "  print('HTTP/2 200\\nX-OAuth-Scopes: read:packages, write:packages')\n"
                "elif endpoint == 'user' and '--jq' in a:\n"
                "  print('coriom')\n"
                "elif '-i' in a and 'user/packages' in endpoint:\n"
                "  print('HTTP/2 200')\n"
                "elif '--paginate' in a:\n"
                "  if state.exists(): print('deopt-v2-backend')\n"
                "elif '/container/' in endpoint and '--jq' in a:\n"
                "  q = a[a.index('--jq') + 1]\n"
                "  print('private' if q == '.visibility' else '')\n"
                "else: sys.exit(9)\n"
            )
            (root / "docker").write_text(
                "#!/usr/bin/env python3\n"
                "import os, pathlib, sys\n"
                "a = sys.argv[1:]\n"
                "cmd = ' '.join(a)\n"
                "if a[:2] == ['image', 'inspect']:\n"
                "  if '.Os' in cmd: print('linux/amd64')\n"
                "  elif 'revision' in cmd: print(os.environ['MOCK_COMMIT'])\n"
                "  else: print('')\n"
                "elif a[0] == 'login':\n"
                "  sys.stdin.read()\n"
                "  pathlib.Path(os.environ['MOCK_CONFIG_PATH']).write_text(os.environ['DOCKER_CONFIG'])\n"
                "elif a[0] == 'push':\n"
                "  pathlib.Path(os.environ['MOCK_PACKAGE_STATE']).write_text('present')\n"
                f"  print('digest: {digest} size: 42')\n"
                "elif a[:2] == ['manifest', 'inspect']:\n"
                "  print('manifest unknown' if ':sha-' in a[-1] else 'unauthorized', file=sys.stderr)\n"
                "  sys.exit(1)\n"
                "elif a[0] in ('buildx', 'run', 'pull', 'logout'): pass\n"
                "else: sys.exit(9)\n"
            )
            for executable in ("git", "gh", "docker"):
                (root / executable).chmod(0o700)
            env = dict(os.environ)
            env.update(
                PATH=f"{root}:{env['PATH']}",
                MOCK_REPO_ROOT=str(ROOT),
                MOCK_COMMIT=commit,
                MOCK_PACKAGE_STATE=str(state),
                MOCK_CONFIG_PATH=str(config_path),
            )
            master, slave = pty.openpty()

            def attach_tty():
                os.setsid()
                fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

            process = subprocess.Popen(
                ["bash", str(SCRIPT), "--publish-private-update" if update else "--publish-private"],
                stdin=slave, stdout=slave, stderr=slave, env=env,
                preexec_fn=attach_tty,
            )
            os.close(slave)
            output = bytearray()

            def wait_for(marker):
                end = time.monotonic() + 10
                while marker.encode() not in output and time.monotonic() < end:
                    ready, _, _ = select.select([master], [], [], 0.2)
                    if ready:
                        try:
                            output.extend(os.read(master, 4096))
                        except OSError:
                            break
                self.assertIn(marker, output.decode(errors="replace"))

            try:
                wait_for("GitHub PAT classic temporaire")
                os.write(master, b"test-only-secret\n")
                wait_for("Saisir PUBLISH_PRIVATE_UPDATE" if update else "Saisir PUBLISH_PRIVATE")
                os.write(master, b"PUBLISH_PRIVATE_UPDATE\n" if update else b"PUBLISH_PRIVATE\n")
                self.assertEqual(process.wait(timeout=10), 0, output.decode(errors="replace"))
                while True:
                    ready, _, _ = select.select([master], [], [], 0.1)
                    if not ready:
                        break
                    try:
                        output.extend(os.read(master, 4096))
                    except OSError:
                        break
                self.assertNotIn("test-only-secret", output.decode(errors="replace"))
                self.assertIn(f"REGISTRY_DIGEST={digest}", output.decode(errors="replace"))
                self.assertFalse(Path(config_path.read_text()).exists())
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
                os.close(master)

    def test_full_interactive_path_uses_temporary_credentials(self):
        self.exercise_interactive_path()

    def test_existing_private_package_update_keeps_temporary_credentials(self):
        self.exercise_interactive_path(update=True)


if __name__ == "__main__":
    unittest.main()
