"""Guard the operator-only root preparation script against deployment actions."""

from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "deploy/vps/prepare-coolify-private-staging-host.sh"


class PrivateStagingHostTests(unittest.TestCase):
    def test_syntax_and_no_container_or_network_mutation(self):
        subprocess.run(["bash", "-n", str(SCRIPT)], check=True)
        source = SCRIPT.read_text()
        self.assertIn('[[ "$EUID" == 0 ]]', source)
        self.assertIn("docker pull", source)
        self.assertIn("containers=$(docker ps", source)
        self.assertNotIn("docker ps --format '{{.Names}}' |", source)
        self.assertIn("docker image inspect", source)
        self.assertIn("chmod 0640", source)
        self.assertIn("chown root:1001", source)
        self.assertNotIn("docker run", source)
        self.assertNotIn("docker compose up", source)
        self.assertNotIn("docker stack deploy", source)
        self.assertNotIn("ufw ", source)
        self.assertNotIn("iptables ", source)

    def test_requires_explicit_mode_before_root_action(self):
        result = subprocess.run(
            ["bash", str(SCRIPT)], capture_output=True, text=True, check=False,
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("usage:", result.stderr)


if __name__ == "__main__":
    unittest.main()
