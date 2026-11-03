"""Offline tests; no real Discord webhook or production backup is used."""

import datetime as dt
from contextlib import redirect_stderr, redirect_stdout
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


HERE = Path(__file__).parent


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, HERE / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


alert = load("discord_backup_alert", "discord_backup_alert.py")
freshness = load("check_backup_freshness", "check_backup_freshness.py")
TEST_URL = "https://discord.com/api/webhooks/123456789012345678/" + "x" * 48


class Response:
    def __init__(self, status, body=b'{"id":"123456789012345678"}', retry_after=None):
        self.status = status
        self.body = body
        self.retry_after = retry_after

    def read(self, limit):
        return self.body[:limit]

    def getheader(self, name):
        return self.retry_after if name == "Retry-After" else None


class Connection:
    def __init__(self, response, calls):
        self.response = response
        self.calls = calls

    def request(self, method, path, body, headers):
        self.calls.append((method, path, body, headers))

    def getresponse(self):
        return self.response

    def close(self):
        pass


class BackupAlertTests(unittest.TestCase):
    def test_message_has_only_four_safe_fields(self):
        when = dt.datetime(2026, 11, 3, 1, 2, 3, tzinfo=dt.timezone.utc)
        body = json.loads(alert.message(alert.BACKUP_SERVICE, when, "a" * 32))
        self.assertEqual(body["allowed_mentions"], {"parse": []})
        self.assertEqual(
            body["content"].splitlines(),
            [f"service={alert.BACKUP_SERVICE}", "utc=2026-11-03T01:02:03Z", "state=FAIL", f"execution_id={'a' * 32}"],
        )
        self.assertEqual(len(body), 2)
        self.assertNotIn(TEST_URL, json.dumps(body))

    def test_synthetic_failure_message_is_fail(self):
        body = json.loads(alert.message(alert.TEST_SERVICE, dt.datetime.now(dt.timezone.utc), "b" * 32))
        self.assertIn("state=FAIL", body["content"])

    def test_rejects_unreviewed_service_and_url(self):
        with self.assertRaises(ValueError):
            alert.message("other.service", dt.datetime.now(dt.timezone.utc), "a" * 32)
        with self.assertRaises(ValueError):
            alert.post_alert("http://discord.com/api/webhooks/123/x", b"{}")

    def test_credential_file_permissions_and_no_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "discord_webhook"
            path.write_text(TEST_URL + "\n")
            path.chmod(0o600)
            self.assertEqual(alert.read_webhook_file(path, os.getuid()), TEST_URL)
            path.chmod(0o644)
            with self.assertRaises(ValueError):
                alert.read_webhook_file(path, os.getuid())
            path.chmod(0o600)
            link = Path(directory) / "link"
            link.symlink_to(path)
            with self.assertRaises(ValueError):
                alert.read_webhook_file(link, os.getuid())

    def test_waits_for_message_confirmation(self):
        calls = []
        factory = lambda *a, **k: Connection(Response(200), calls)
        self.assertTrue(alert.post_alert(TEST_URL, b"{}", connect=factory, sleep=lambda _: None))
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0][0], "POST")
        self.assertTrue(calls[0][1].endswith("?wait=true"))
        self.assertNotIn(TEST_URL, calls[0][2].decode())

    def test_retries_rate_limit_then_succeeds(self):
        responses = iter([Response(429, b"{}", "0.5"), Response(200)])
        calls, sleeps = [], []
        factory = lambda *a, **k: Connection(next(responses), calls)
        self.assertTrue(alert.post_alert(TEST_URL, b"{}", connect=factory, sleep=sleeps.append))
        self.assertEqual(len(calls), 2)
        self.assertEqual(sleeps, [0.5])

    def test_missing_confirmation_fails(self):
        factory = lambda *a, **k: Connection(Response(200, b"{}"), [])
        self.assertFalse(alert.post_alert(TEST_URL, b"{}", connect=factory, sleep=lambda _: None))

    def test_failed_delivery_is_queued_and_retried_without_secret(self):
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "state"
            with (patch.object(alert, "STATE_DIR", state),
                  patch.object(alert, "PENDING_DIR", state / "alerts-pending"),
                  patch.object(alert, "OWNER_UID", os.getuid()),
                  patch.object(alert.os, "geteuid", return_value=0),
                  patch.object(alert, "read_webhook_credential", return_value=TEST_URL),
                  patch.object(alert, "execution_id", return_value="d" * 32),
                  patch.object(alert, "post_alert", side_effect=[False, True]) as post):
                output = io.StringIO()
                with patch("sys.argv", ["alert", alert.BACKUP_SERVICE]), redirect_stderr(output):
                    self.assertEqual(alert.main(), 1)
                self.assertNotIn(TEST_URL, output.getvalue())
                pending = list((state / "alerts-pending").glob("*.json"))
                self.assertEqual(len(pending), 1)
                self.assertEqual(pending[0].stat().st_mode & 0o777, 0o600)
                fields = json.loads(pending[0].read_text())
                self.assertEqual(set(fields), {"service", "utc", "state", "execution_id"})
                with patch("sys.argv", ["alert", "--retry-pending"]), redirect_stdout(io.StringIO()):
                    self.assertEqual(alert.main(), 0)
                self.assertEqual(list((state / "alerts-pending").glob("*.json")), [])
                self.assertEqual(post.call_count, 2)

    def test_watchdog_alert_deduplicates_same_stale_backup(self):
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "state"
            state.mkdir(mode=0o700)
            (state / "last-success").write_text("1800000000 " + "e" * 32 + "\n")
            with (patch.object(alert, "STATE_DIR", state),
                  patch.object(alert, "PENDING_DIR", state / "alerts-pending"),
                  patch.object(alert, "OWNER_UID", os.getuid()),
                  patch.object(alert.os, "geteuid", return_value=0),
                  patch.object(alert, "read_webhook_credential", return_value=TEST_URL),
                  patch.object(alert, "execution_id", return_value="f" * 32),
                  patch.object(alert, "post_alert", return_value=True) as post,
                  patch("sys.argv", ["alert", alert.WATCHDOG_SERVICE]),
                  redirect_stdout(io.StringIO())):
                self.assertEqual(alert.main(), 0)
                self.assertEqual(alert.main(), 0)
                self.assertEqual(post.call_count, 1)

    def test_watchdog_boundary_and_bad_state(self):
        now = 1_800_000_000
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "last-success"
            path.write_text(f"{now - freshness.MAX_AGE_SECONDS} {'c' * 32}\n")
            path.chmod(0o600)
            self.assertTrue(freshness.is_fresh(path, now, os.getuid()))
            self.assertFalse(freshness.is_fresh(path, now + 1, os.getuid()))
            self.assertFalse(freshness.is_fresh(path, now, os.getuid() + 1))
            path.chmod(0o644)
            self.assertFalse(freshness.is_fresh(path, now, os.getuid()))
            path.chmod(0o600)
            path.write_text("invalid\n")
            self.assertFalse(freshness.is_fresh(path, now, os.getuid()))
            path.unlink()
            with self.assertRaises(FileNotFoundError):
                freshness.is_fresh(path, now, os.getuid())


if __name__ == "__main__":
    unittest.main()
