"""Narrow local before/after/rollback fixture for the proxy port decision."""

import unittest

from .proxy_8080_plan import reconcile_listeners, removal_candidate


class Proxy8080PlanTests(unittest.TestCase):
    def test_ipv6_listener_can_exist_without_explicit_ipv6_binding(self):
        evidence = reconcile_listeners(
            {"8080/tcp": [{"HostIp": "0.0.0.0", "HostPort": "8080"}]},
            "LISTEN 0 4096 0.0.0.0:8080 0.0.0.0:*\n"
            "LISTEN 0 4096 [::]:8080 [::]:*\n",
        )
        self.assertEqual(evidence, {
            "docker_ipv4": True, "docker_ipv6": False,
            "listen_ipv4": True, "listen_ipv6": True,
        })

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
