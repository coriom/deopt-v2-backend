"""Adversarial checks against the normalized future private Compose profile."""

import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import unittest

from .check_coolify_private_compose import PrivateComposeError, check

ROOT = Path(__file__).resolve().parents[2]


class CoolifyPrivateComposeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not shutil.which("docker"):
            raise unittest.SkipTest("Docker Compose CLI unavailable")
        env = dict(os.environ)
        env.update({
            "DEOPT_BACKEND_IMAGE": "ghcr.io/coriom/deopt-v2-backend@sha256:" + "a" * 64,
            "DEOPT_POSTGRES_IMAGE": "postgres:16@sha256:" + "b" * 64,
            "DEOPT_CORS_ORIGIN": "https://deoptv2-frontend.vercel.app",
            "DEOPT_POSTGRES_PASSWORD_FILE": "/tmp/nonsecret-placeholder",
        })
        rendered = subprocess.run(
            ["docker", "compose", "-f", str(ROOT / "compose.coolify-readonly.yml"),
             "config", "--format", "json"],
            env=env, capture_output=True, text=True, check=False,
        )
        if rendered.returncode:
            raise AssertionError("Compose render failed: " + rendered.stderr)
        cls.fixture = json.loads(rendered.stdout)

    def reject(self, change, code):
        candidate = copy.deepcopy(self.fixture)
        change(candidate)
        with self.assertRaisesRegex(PrivateComposeError, code):
            check(candidate)

    def test_exact_source_profile_passes_without_authorizing_deployment(self):
        check(self.fixture)

    def test_rejects_published_port_and_proxy_route(self):
        self.reject(lambda x: x["services"]["backend"].update(ports=[{"published": 8080, "target": 8080}]),
                    "BACKEND_PUBLISHED_PORT")
        self.reject(lambda x: x["services"]["backend"].update(labels={"traefik.http.routers.deopt.rule": "Host(`x`)"}),
                    "BACKEND_PROXY_ROUTE")

    def test_rejects_shared_network_and_database_exposure(self):
        self.reject(lambda x: x["networks"]["db"].update(external=True), "SHARED_NETWORK")
        self.reject(lambda x: x["networks"]["db"].update(internal=False), "POSTGRES_NETWORK_NOT_INTERNAL")
        self.reject(lambda x: x["services"]["postgres"].update(ports=[{"published": 5432, "target": 5432}]),
                    "POSTGRES_PUBLISHED_PORT")

    def test_rejects_missing_persistence_or_secret_mount(self):
        self.reject(lambda x: x["services"]["postgres"].update(volumes=[]), "POSTGRES_PERSISTENCE")
        self.reject(lambda x: x["services"]["backend"].update(secrets=[]), "POSTGRES_SECRET_MOUNT")
        self.reject(lambda x: x["services"]["postgres"]["environment"].update(POSTGRES_PASSWORD="inline"),
                    "POSTGRES_PASSWORD_SOURCE")

    def test_rejects_unpinned_image_and_wrong_postgres_major(self):
        self.reject(lambda x: x["services"]["backend"].update(image="ghcr.io/coriom/deopt:latest"),
                    "BACKEND_IMAGE_DIGEST")
        self.reject(lambda x: x["services"]["postgres"].update(image="postgres:17@sha256:" + "c" * 64),
                    "POSTGRES_MAJOR_VERSION")

    def test_rejects_trading_signing_or_wrong_chain(self):
        self.reject(lambda x: x["services"]["backend"]["environment"].update(EXECUTION_ENABLED="true"),
                    "BACKEND_READ_ONLY_FLAGS")
        self.reject(lambda x: x["services"]["backend"]["environment"].update(EXECUTOR_PRIVATE_KEY="dummy"),
                    "BACKEND_SIGNER_VARIABLE")
        self.reject(lambda x: x["services"]["backend"]["environment"].update(PERPS_CHAIN_ID="8453"),
                    "BACKEND_EXTRA_CHAIN")

    def test_rejects_privilege_or_docker_socket(self):
        self.reject(lambda x: x["services"]["backend"].update(privileged=True), "BACKEND_PRIVILEGED")
        self.reject(lambda x: x["services"]["backend"].update(volumes=["/var/run/docker.sock:/var/run/docker.sock"]),
                    "BACKEND_DOCKER_SOCKET")

    def test_rejects_extra_service_or_missing_ready_check(self):
        self.reject(lambda x: x["services"].update(worker={"image": "example@sha256:" + "a" * 64}), "SERVICES")
        self.reject(lambda x: x["services"]["backend"].update(healthcheck={"test": ["CMD", "true"]}),
                    "BACKEND_READINESS_CHECK")

    def test_local_image_builder_requires_explicit_mode_and_has_no_publish(self):
        script = ROOT / "deploy/vps/build-private-image-local.sh"
        subprocess.run(["bash", "-n", str(script)], check=True)
        result = subprocess.run(["bash", str(script)], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 2)
        contents = script.read_text()
        self.assertIn("--load --pull=false --platform linux/amd64", contents)
        self.assertNotIn("docker push", contents)

    def test_builder_and_runtime_bases_are_digest_pinned(self):
        dockerfile = (ROOT / "Dockerfile").read_text()
        self.assertIn("FROM rust:1.95-bookworm@sha256:", dockerfile)
        self.assertIn("FROM debian:bookworm-slim@sha256:", dockerfile)


if __name__ == "__main__":
    unittest.main()
