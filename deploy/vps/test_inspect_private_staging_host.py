"""The operator-only host inspector must not disclose secrets or mutate state."""

from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "deploy/vps/inspect-private-staging-host.sh"


class InspectHostTests(unittest.TestCase):
    def test_syntax_and_read_only_scope(self):
        subprocess.run(["bash", "-n", str(SCRIPT)], check=True)
        source = SCRIPT.read_text()
        self.assertIn("POSTGRES_SECRET_HOST_PERMISSIONS=PASS_CONTENT_UNREAD", source)
        self.assertIn("COOLIFY_UI_SAVED_PROXY_CONFIGURATION=NOT_VERIFIED", source)
        self.assertNotIn('cat "$secret"', source)
        self.assertNotIn("docker run", source)
        self.assertNotIn("docker compose up", source)
        self.assertNotIn("docker restart", source)
        self.assertNotIn("sudo ", source)

    def test_requires_explicit_operator_mode(self):
        result = subprocess.run(["bash", str(SCRIPT)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)


if __name__ == "__main__":
    unittest.main()
