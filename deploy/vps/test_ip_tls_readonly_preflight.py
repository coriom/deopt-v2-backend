"""No-secrets tests for the IP TLS proxy evidence parser."""

from .ip_tls_readonly_preflight import proxy_evidence
import unittest


class ProxyEvidenceTests(unittest.TestCase):
    def test_proxy_evidence_filters_environment_and_secret_mounts(self):
        inspected = {
        "Config": {"Image": "traefik:v3.7", "Env": ["SECRET=never-print"],
                   "Cmd": ["--entrypoints.http.address=:80", "--entrypoints.https.address=:443",
                           "--providers.file.directory=/traefik/dynamic", "--providers.file.watch=true",
                           "--certificatesresolvers.letsencrypt.acme.email=private@example.invalid"]},
        "State": {"Running": True},
        "Mounts": [{"Source": "/data/coolify/proxy", "Destination": "/traefik"},
                   {"Source": "/root/secret", "Destination": "/run/secret"}],
        "NetworkSettings": {"Networks": {"coolify": {}, "other-project": {}}},
        }
        evidence = proxy_evidence(inspected)
        self.assertEqual(evidence["PROXY_RUNNING"], "True")
        self.assertEqual(evidence["PROXY_TRAEFIK_MOUNT"], "True")
        self.assertEqual(evidence["FILE_DIRECTORY"], "/traefik/dynamic")
        self.assertEqual(evidence["FILE_WATCH"], "True")
        self.assertEqual(evidence["ACME_RESOLVERS"], "letsencrypt")
        self.assertEqual(evidence["ENTRYPOINTS"], "http.address,https.address")
        self.assertNotIn("SECRET", str(evidence))
        self.assertNotIn("private@example.invalid", str(evidence))
        self.assertNotIn("/root/secret", str(evidence))


    def test_missing_provider_is_unknown_and_not_assumed_safe(self):
        evidence = proxy_evidence({"Config": {"Cmd": []}, "State": {"Running": False}})
        self.assertEqual(evidence["FILE_WATCH"], "False")
        self.assertEqual(evidence["FILE_DIRECTORY"], "UNKNOWN")
        self.assertEqual(evidence["PROXY_RUNNING"], "False")


if __name__ == "__main__":
    unittest.main()
