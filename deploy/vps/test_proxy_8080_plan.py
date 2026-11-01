"""Narrow local before/after/rollback fixture for the proxy port decision."""

import unittest

from .proxy_8080_plan import removal_candidate


class Proxy8080PlanTests(unittest.TestCase):
    def test_only_8080_removed_and_saved_original_restores_exactly(self):
        current = {
            "80/tcp": [{"HostIp": "0.0.0.0", "HostPort": "80"},
                       {"HostIp": "::", "HostPort": "80"}],
            "443/tcp": [{"HostIp": "0.0.0.0", "HostPort": "443"},
                        {"HostIp": "::", "HostPort": "443"}],
            "8080/tcp": [{"HostIp": "0.0.0.0", "HostPort": "8080"},
                         {"HostIp": "::", "HostPort": "8080"}],
        }
        saved, candidate = removal_candidate(current)
        self.assertEqual(set(candidate), {"80/tcp", "443/tcp"})
        self.assertEqual(candidate["80/tcp"], current["80/tcp"])
        self.assertEqual(candidate["443/tcp"], current["443/tcp"])
        self.assertEqual(saved, current)
        self.assertIsNot(saved, current)

    def test_missing_required_port_refuses_change(self):
        with self.assertRaisesRegex(ValueError, "EXPECTED_PROXY_PORT_MISSING"):
            removal_candidate({"80/tcp": [{"HostPort": "80"}]})


if __name__ == "__main__":
    unittest.main()
