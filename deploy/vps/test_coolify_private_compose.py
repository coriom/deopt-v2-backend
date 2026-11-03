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
        self.assertEqual(self.fixture["services"]["backend"]["environment"]["WALLET_READ_AUTH_ENABLED"], "false")

    def test_wallet_auth_is_opt_in_and_bound_to_reviewed_frontend(self):
        candidate = copy.deepcopy(self.fixture)
        env = candidate["services"]["backend"]["environment"]
        env["WALLET_READ_AUTH_ENABLED"] = "true"
        check(candidate)
        env["WALLET_READ_AUTH_ORIGIN"] = "https://other.example"
        with self.assertRaisesRegex(PrivateComposeError, "WALLET_AUTH_ORIGIN"):
            check(candidate)
        env["WALLET_READ_AUTH_ORIGIN"] = "https://deoptv2-frontend.vercel.app"
        env["TRADING_ENABLED"] = "true"
        with self.assertRaisesRegex(PrivateComposeError, "BACKEND_UNEXPECTED_ENABLED_FLAG"):
            check(candidate)

    def test_exact_release_manifest_accepts_only_reviewed_images_and_secret_path(self):
        release = json.loads((ROOT / "deploy/vps/coolify-staging-release-v1.json").read_text())
        env = dict(os.environ)
        env.update({
            "DEOPT_BACKEND_IMAGE": release["backend_image"],
            "DEOPT_POSTGRES_IMAGE": release["postgres_image"],
            "DEOPT_CORS_ORIGIN": release["cors_origin"],
            "DEOPT_POSTGRES_PASSWORD_FILE": release["postgres_password_file"],
        })
        result = subprocess.run(
            ["docker", "compose", "-f", str(ROOT / "compose.coolify-readonly.yml"),
             "config", "--format", "json"],
            env=env, capture_output=True, text=True, check=True,
        )
        rendered = json.loads(result.stdout)
        check(rendered, release)
        for service, key, value, code in (
            ("backend", "image", "ghcr.io/coriom/other@sha256:" + "a" * 64, "BACKEND_RELEASE_DIGEST"),
            ("postgres", "image", "postgres:16@sha256:" + "b" * 64, "POSTGRES_RELEASE_DIGEST"),
        ):
            candidate = copy.deepcopy(rendered)
            candidate["services"][service][key] = value
            with self.assertRaisesRegex(PrivateComposeError, code):
                check(candidate, release)
        candidate = copy.deepcopy(rendered)
        candidate["services"]["backend"]["environment"]["CORS_ALLOWED_ORIGINS"] = "*"
        with self.assertRaisesRegex(PrivateComposeError, "BACKEND_CORS_ORIGIN"):
            check(candidate, release)
        candidate = copy.deepcopy(rendered)
        candidate["secrets"]["postgres_password"]["file"] = "/tmp/other"
        with self.assertRaisesRegex(PrivateComposeError, "POSTGRES_RELEASE_SECRET_PATH"):
            check(candidate, release)

    def test_production_manifest_is_private_and_separate_from_staging(self):
        staging = json.loads((ROOT / "deploy/vps/coolify-staging-release-v1.json").read_text())
        production = json.loads((ROOT / "deploy/vps/coolify-production-release-v1.json").read_text())
        self.assertNotEqual(production["postgres_password_file"], staging["postgres_password_file"])
        self.assertEqual(production["backend_image"], staging["backend_image"])
        self.assertEqual(production["postgres_image"], staging["postgres_image"])
        self.assertEqual(production["coolify_environment"], "production")
        env = dict(os.environ)
        env.update(DEOPT_BACKEND_IMAGE=production["backend_image"],
                   DEOPT_POSTGRES_IMAGE=production["postgres_image"],
                   DEOPT_CORS_ORIGIN=production["cors_origin"],
                   DEOPT_POSTGRES_PASSWORD_FILE=production["postgres_password_file"])
        result = subprocess.run(
            ["docker", "compose", "-f", str(ROOT / "compose.coolify-readonly.yml"),
             "config", "--format", "json"],
            env=env, capture_output=True, text=True, check=True)
        check(json.loads(result.stdout), production)

    def test_production_secret_setup_requires_reviewed_root_host(self):
        script = ROOT / "deploy/vps/operator-create-production-secret.sh"
        subprocess.run(["bash", "-n", str(script)], check=True)
        result = subprocess.run(["bash", str(script)], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 2)
        self.assertIn("Root on reviewed Netcup host required", result.stderr)

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
        self.reject(lambda x: x["services"]["backend"].update(networks={"db": None}),
                    "BACKEND_NETWORK_SET")
        self.reject(lambda x: x["networks"].update(extra={"internal": True}),
                    "NETWORK_SET")

    def test_rejects_unreviewed_env_file(self):
        self.reject(lambda x: x["services"]["backend"].update(env_file=["/data/coolify/generated/.env"]),
                    "BACKEND_UNREVIEWED_ENV_FILE")

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
        self.reject(lambda x: x["services"]["backend"]["environment"].update(EXECUTOR_DRY_RUN="false"),
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
