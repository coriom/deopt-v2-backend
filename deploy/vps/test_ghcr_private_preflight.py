"""Privacy gate tests: a public source cannot create a new GHCR package."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "deploy/vps/ghcr-private-preflight.sh"


class GhcrPrivatePreflightTests(unittest.TestCase):
    def test_workflow_is_manual_and_uses_minimum_package_permissions(self):
        workflow = (ROOT / ".github/workflows/publish-private-ghcr.yml").read_text()
        self.assertIn("  workflow_dispatch:", workflow)
        self.assertNotIn("\n  push:", workflow)
        self.assertNotIn("\n  schedule:", workflow)
        self.assertIn("  contents: read", workflow)
        self.assertIn("  packages: write", workflow)
        self.assertIn("ghcr-private-preflight.sh pre", workflow)
        self.assertIn("ghcr-private-preflight.sh post", workflow)
        self.assertIn("github.event.repository.private == true", workflow)
        self.assertNotIn(":latest", workflow)

    def run_gate(self, repo_visibility, package_status, package_visibility="", mode="pre",
                 linked_repository="coriom/deopt-v2-backend"):
        with tempfile.TemporaryDirectory() as folder:
            gh = Path(folder) / "gh"
            gh.write_text(
                "#!/bin/sh\n"
                "case \"$*\" in\n"
                "  *repos/coriom/deopt-v2-backend*) echo \"$MOCK_REPO_VISIBILITY\" ;;\n"
                "  *'-i users/coriom/packages/container/deopt-v2-backend'*) "
                "echo \"HTTP/2 $MOCK_PACKAGE_STATUS\"; "
                "test \"$MOCK_PACKAGE_STATUS\" = 200 ;;\n"
                "  *'--jq .repository.full_name // empty'*) "
                "echo \"$MOCK_LINKED_REPOSITORY\" ;;\n"
                "  *users/coriom/packages/container/deopt-v2-backend*) "
                "echo \"$MOCK_PACKAGE_VISIBILITY\" ;;\n"
                "  *) exit 9 ;;\n"
                "esac\n"
            )
            gh.chmod(0o755)
            env = dict(os.environ)
            env.update(
                PATH=f"{folder}:{env['PATH']}",
                GITHUB_REPOSITORY="coriom/deopt-v2-backend",
                GITHUB_REF="refs/heads/main",
                MOCK_REPO_VISIBILITY=repo_visibility,
                MOCK_PACKAGE_STATUS=str(package_status),
                MOCK_PACKAGE_VISIBILITY=package_visibility,
                MOCK_LINKED_REPOSITORY=linked_repository,
            )
            return subprocess.run(
                ["bash", str(SCRIPT), mode], env=env,
                text=True, capture_output=True, check=False,
            )

    def test_public_repo_cannot_create_package(self):
        self.assertNotEqual(self.run_gate("public", 404).returncode, 0)

    def test_private_repo_can_create_package(self):
        self.assertEqual(self.run_gate("private", 404).returncode, 0)

    def test_public_repo_cannot_access_private_package_via_workflow(self):
        self.assertNotEqual(self.run_gate("public", 200, "private").returncode, 0)
        self.assertEqual(self.run_gate("private", 200, "private").returncode, 0)

    def test_existing_public_package_is_rejected(self):
        self.assertNotEqual(self.run_gate("private", 200, "public").returncode, 0)

    def test_unknown_package_status_is_rejected(self):
        self.assertNotEqual(self.run_gate("private", 403).returncode, 0)
        self.assertNotEqual(self.run_gate("private", "").returncode, 0)

    def test_post_publish_requires_private_package(self):
        self.assertNotEqual(self.run_gate("private", 404, mode="post").returncode, 0)
        self.assertEqual(self.run_gate("private", 200, "private", mode="post").returncode, 0)
        self.assertNotEqual(self.run_gate("private", 200, "private", mode="post",
                                         linked_repository="another/repo").returncode, 0)


if __name__ == "__main__":
    unittest.main()
