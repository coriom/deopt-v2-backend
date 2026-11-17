"""Guard the public read route used by the physical stack status view."""

from pathlib import Path
import re
import unittest


ROUTE_FILE = Path(__file__).resolve().parent / "traefik/deopt-public-api-v5.yaml"


class DeoptPublicApiV5Tests(unittest.TestCase):
    def test_physical_stack_uses_existing_get_only_backend_route(self):
        content = ROUTE_FILE.read_text()
        read_rule = content.split("    deopt-v5-read:", 1)[1].split(
            "    deopt-v5-auth-write:", 1
        )[0]
        self.assertIn("Host(`94.16.113.122`) && Method(`GET`)", read_rule)
        self.assertEqual(
            re.findall(r"Path\(`/options/[^`]+`\)", read_rule),
            ["Path(`/options/physical-stack`)"]
        )
        self.assertIn("priority: 1000", read_rule)
        self.assertIn("service: deopt-v5-backend", read_rule)
        self.assertNotIn("PathPrefix(", read_rule)
        self.assertNotIn("Method(`POST`)", read_rule)
        auth_rule = content.split("    deopt-v5-auth-write:", 1)[1].split(
            "  middlewares:", 1
        )[0]
        self.assertNotIn("/options/", auth_rule)
        self.assertIn(
            "url: http://dxco90jwpdxyennmquv2elyj-backend-1:8080", content
        )


if __name__ == "__main__":
    unittest.main()
