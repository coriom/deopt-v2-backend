"""Guard the reviewed Traefik IP ACME configuration against drift."""

from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2] / "deploy/vps"


class IpAcmeCandidateTests(unittest.TestCase):
    def test_staging_and_production_resolvers_remain_separate(self):
        flags = (ROOT / "traefik-ip-acme-flags-v1.txt").read_text().splitlines()
        self.assertEqual(len(flags), 11)
        self.assertEqual(len(set(flags)), len(flags))
        self.assertTrue(all(x.startswith("--certificatesresolvers.deoptip") for x in flags))
        for name in ("deoptipstaging", "deoptipproduction"):
            prefix = f"--certificatesresolvers.{name}.acme."
            values = {x[len(prefix):] for x in flags if x.startswith(prefix)}
            self.assertIn("httpchallenge.entrypoint=http", values)
            self.assertIn("profile=shortlived", values)
            self.assertIn("disablecommonname=true", values)
            self.assertIn("certificatesduration=160", values)
        self.assertIn(
            "--certificatesresolvers.deoptipstaging.acme.caserver=https://acme-staging-v02.api.letsencrypt.org/directory",
            flags,
        )
        self.assertFalse(any("deoptipproduction.acme.caserver" in x for x in flags))
        self.assertIn("--certificatesresolvers.deoptipstaging.acme.storage=/traefik/acme-ip-staging.json", flags)
        self.assertIn("--certificatesresolvers.deoptipproduction.acme.storage=/traefik/acme-ip-production.json", flags)
        self.assertFalse(any("insecure" in x or ":8080" in x for x in flags))

    def test_dynamic_file_requests_only_staging_ip_default(self):
        content = (ROOT / "traefik-ip-staging-dynamic-v1.yaml").read_text()
        self.assertIn("defaultGeneratedCert:", content)
        self.assertIn("resolver: deoptipstaging", content)
        self.assertIn('main: "94.16.113.122"', content)
        self.assertNotIn("deoptipproduction", content)
        self.assertNotIn("http:", content)

    def test_production_default_differs_only_by_resolver(self):
        staging = (ROOT / "traefik-ip-staging-dynamic-v1.yaml").read_text()
        production = (ROOT / "traefik-ip-production-dynamic-v1.yaml").read_text()
        self.assertEqual(production, staging.replace("deoptipstaging", "deoptipproduction"))
        self.assertEqual(production.count("deoptipproduction"), 1)
        self.assertNotIn("http:", production)


if __name__ == "__main__":
    unittest.main()
