import unittest
import ssl
from unittest.mock import patch

import monitor_public_https_v6 as monitor


class MonitorPolicyTests(unittest.TestCase):
    def test_ip_san_and_48h_threshold(self):
        cert = {"issuer": ((("organizationName", "Let's Encrypt"),),), "subjectAltName": (("IP Address", monitor.IP),), "notAfter": "Nov 10 06:20:04 2026 GMT"}
        expiry = ssl.cert_time_to_seconds(cert["notAfter"])
        self.assertEqual(monitor.certificate_status(cert, expiry - monitor.MIN_SECONDS - 1), expiry)
        with self.assertRaisesRegex(ValueError, "CERT_UNDER_48H"):
            monitor.certificate_status(cert, expiry - monitor.MIN_SECONDS + 1)
        with self.assertRaisesRegex(ValueError, "CERT_EXPIRED"):
            monitor.certificate_status(cert, expiry)
        cert["subjectAltName"] = (("DNS", monitor.IP),)
        with self.assertRaisesRegex(ValueError, "IP_SAN_MISSING"):
            monitor.certificate_status(cert, expiry - monitor.MIN_SECONDS - 1)

    def test_issuer_rejects_staging_and_unexpected_ca(self):
        cert = {"issuer": ((("organizationName", "Let's Encrypt"),), (("commonName", "YR1"),)), "subjectAltName": (("IP Address", monitor.IP),), "notAfter": "Nov 10 06:20:04 2026 GMT"}
        self.assertEqual(monitor.certificate_status(cert, ssl.cert_time_to_seconds(cert["notAfter"]) - monitor.MIN_SECONDS - 1), ssl.cert_time_to_seconds(cert["notAfter"]))
        cert["issuer"] = ((("organizationName", "(STAGING) Let's Encrypt"),),)
        with self.assertRaisesRegex(ValueError, "CERT_ISSUER_UNEXPECTED"):
            monitor.certificate_status(cert, 0)
        cert["issuer"] = ((("organizationName", "Other CA"),),)
        with self.assertRaisesRegex(ValueError, "CERT_ISSUER_UNEXPECTED"):
            monitor.certificate_status(cert, 0)

    def test_health_rejects_redirect_error_and_bad_body(self):
        monitor.health_status(200, b'{"ok":true,"service":"deopt-v2-backend"}')
        for status, body in ((302, b""), (503, b""), (200, b"not-json"), (200, b"[]"), (200, b'{"ok":false}')):
            with self.assertRaises(ValueError):
                monitor.health_status(status, body)

    def test_synthetic_alert_exits_nonzero(self):
        with patch("sys.argv", ["monitor", "--simulate-alert"]), patch.object(monitor, "check_certificate"), patch.object(monitor, "check_health"):
            self.assertEqual(monitor.main(), 1)

    def test_synthetic_expiring_certificate_exits_nonzero(self):
        with patch("sys.argv", ["monitor", "--simulate-expiring-certificate"]), patch.object(monitor, "check_certificate"), patch.object(monitor, "check_health"):
            self.assertEqual(monitor.main(), 1)


if __name__ == "__main__":
    unittest.main()
