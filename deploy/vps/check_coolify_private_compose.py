"""Fail-closed review of Coolify's rendered DeOpt Compose before private deploy.

Input is `docker compose config --format json` from the reviewed Coolify
resource. No values from that file are printed or retained by this tool.
"""

import json
from pathlib import Path
import re
import sys


class PrivateComposeError(ValueError):
    pass


def require(ok: bool, code: str) -> None:
    if not ok:
        raise PrivateComposeError(code)


def environment(service: dict) -> dict:
    value = service.get("environment", {})
    if isinstance(value, dict):
        return value
    if isinstance(value, list):
        return dict(item.split("=", 1) for item in value if isinstance(item, str) and "=" in item)
    raise PrivateComposeError("ENV_FORMAT")


def network_names(service: dict) -> set[str]:
    value = service.get("networks", {})
    if isinstance(value, list):
        return set(value)
    if isinstance(value, dict):
        return set(value)
    raise PrivateComposeError("NETWORK_FORMAT")


def label_keys(service: dict) -> set[str]:
    value = service.get("labels", {})
    if isinstance(value, dict):
        return set(value)
    if isinstance(value, list):
        return {item.split("=", 1)[0] for item in value if isinstance(item, str)}
    raise PrivateComposeError("LABEL_FORMAT")


def secret_names(service: dict) -> set[str]:
    return {item if isinstance(item, str) else item.get("source")
            for item in service.get("secrets", [])}


def check(rendered: dict, release: dict | None = None) -> None:
    services = rendered.get("services", {})
    require(set(services) == {"postgres", "backend"}, "SERVICES")
    require(isinstance(rendered.get("networks"), dict), "NETWORKS")
    networks = rendered["networks"]
    require(set(networks) == {"db", "web"}, "NETWORK_SET")
    for name, service in services.items():
        require(not service.get("env_file"), f"{name.upper()}_UNREVIEWED_ENV_FILE")
        require(not service.get("ports"), f"{name.upper()}_PUBLISHED_PORT")
        require(not service.get("build"), f"{name.upper()}_BUILD")
        require(not service.get("privileged"), f"{name.upper()}_PRIVILEGED")
        require(service.get("network_mode") != "host", f"{name.upper()}_HOST_NETWORK")
        require(service.get("pid") != "host" and service.get("ipc") != "host", f"{name.upper()}_HOST_NAMESPACE")
        require(not service.get("devices") and not service.get("cap_add"), f"{name.upper()}_HOST_ACCESS")
        require(not any(key.startswith(("traefik.", "caddy.")) for key in label_keys(service)),
                f"{name.upper()}_PROXY_ROUTE")
        image = service.get("image", "")
        require(bool(re.fullmatch(r"[^\s]+@sha256:[0-9a-f]{64}", image)), f"{name.upper()}_IMAGE_DIGEST")
        require(not any("docker.sock" in str(mount) for mount in service.get("volumes", [])),
                f"{name.upper()}_DOCKER_SOCKET")
        for network in network_names(service):
            require(network in networks, f"{name.upper()}_UNKNOWN_NETWORK")
            spec = networks[network] or {}
            require(not spec.get("external") and network != "coolify" and spec.get("name") != "coolify",
                    f"{name.upper()}_SHARED_NETWORK")

    postgres = services["postgres"]
    backend = services["backend"]
    require(bool(re.fullmatch(r"postgres:16(?:[.-][^@]+)?@sha256:[0-9a-f]{64}", postgres["image"])),
            "POSTGRES_MAJOR_VERSION")
    pg_networks = network_names(postgres)
    require(pg_networks == {"db"}, "POSTGRES_NETWORK_COUNT")
    db_network = "db"
    require(bool(networks[db_network].get("internal")), "POSTGRES_NETWORK_NOT_INTERNAL")
    require(network_names(backend) == {"db", "web"}, "BACKEND_NETWORK_SET")
    volumes = rendered.get("volumes", {})
    require(isinstance(volumes, dict), "VOLUME_DEFINITIONS")
    require(any(mount.get("type") == "volume" and mount.get("target") == "/var/lib/postgresql/data"
                and mount.get("source") in volumes
                for mount in postgres.get("volumes", []) if isinstance(mount, dict)),
            "POSTGRES_PERSISTENCE")
    pg_env = environment(postgres)
    require(pg_env.get("POSTGRES_USER") == "deopt_readonly" and pg_env.get("POSTGRES_DB") == "deopt_readonly",
            "POSTGRES_IDENTITY")
    require("POSTGRES_PASSWORD" not in pg_env and
            pg_env.get("POSTGRES_PASSWORD_FILE") == "/run/secrets/postgres_password",
            "POSTGRES_PASSWORD_SOURCE")
    secret = rendered.get("secrets", {}).get("postgres_password", {})
    require(isinstance(secret, dict) and str(secret.get("file", "")).startswith("/") and
            "postgres_password" in secret_names(postgres) and
            "postgres_password" in secret_names(backend), "POSTGRES_SECRET_MOUNT")
    require(postgres.get("restart") == "unless-stopped", "POSTGRES_RESTART")

    env = environment(backend)
    required = {
        "CHAIN_ID": "84532", "NETWORK_NAME": "base-sepolia", "DEOPT_READ_ONLY_RELEASE": "true",
        "PERSISTENCE_ENABLED": "true", "EXECUTION_ENABLED": "false",
        "EXECUTOR_DRY_RUN": "true",
        "EXECUTOR_REAL_BROADCAST_ENABLED": "false", "HV2_BROADCAST_ENABLED": "false",
        "PERPS_PUBLIC_TRADING_ENABLED": "false", "PERPS_CLOSED_TEST_ENABLED": "false",
        "INDEXER_ENABLED": "false", "PERPS_READ_ENABLED": "false", "OPTIONS_ENABLED": "false",
        "PUBLIC_WS_ENABLED": "false", "METRICS_ENABLED": "false", "MM_GATEWAY_ENABLED": "false",
    }
    require(all(str(env.get(key)).lower() == value for key, value in required.items()), "BACKEND_READ_ONLY_FLAGS")
    require(not any(token in key.upper() for key in env for token in
                    ("PRIVATE_KEY", "SIGNER_KEY", "MNEMONIC", "KEYSTORE")), "BACKEND_SIGNER_VARIABLE")
    require(not any(key.upper().endswith("_CHAIN_ID") and str(value) != "84532"
                    for key, value in env.items()), "BACKEND_EXTRA_CHAIN")
    require("DATABASE_URL" not in env, "BACKEND_INLINE_DATABASE_SECRET")
    require(env.get("WALLET_READ_AUTH_ENABLED", "false") in {"true", "false"},
            "WALLET_AUTH_FLAG")
    require(env.get("WALLET_READ_AUTH_ORIGIN") in (None, "https://deoptv2-frontend.vercel.app"),
            "WALLET_AUTH_ORIGIN")
    if env.get("WALLET_READ_AUTH_ENABLED", "false") == "true":
        require(env.get("WALLET_READ_AUTH_ORIGIN") == "https://deoptv2-frontend.vercel.app",
                "WALLET_AUTH_ORIGIN")
    require(not any(key.upper().endswith("_ENABLED") and str(value).lower() == "true"
                    and key.upper() not in {"PERSISTENCE_ENABLED", "WALLET_READ_AUTH_ENABLED"}
                    for key, value in env.items()),
            "BACKEND_UNEXPECTED_ENABLED_FLAG")
    require(backend.get("user") == "1001:1001" and backend.get("read_only") is True,
            "BACKEND_RUNTIME_IDENTITY")
    require("ALL" in backend.get("cap_drop", []) and "no-new-privileges:true" in backend.get("security_opt", []),
            "BACKEND_PRIVILEGE_GUARD")
    require(backend.get("restart") == "unless-stopped", "BACKEND_RESTART")
    require("/ready" in str(backend.get("healthcheck", {}).get("test", "")), "BACKEND_READINESS_CHECK")
    require(backend.get("entrypoint") == ["/usr/local/bin/deopt-readonly-entrypoint"],
            "BACKEND_ENTRYPOINT")
    if release is not None:
        require(backend["image"] == release["backend_image"], "BACKEND_RELEASE_DIGEST")
        require(postgres["image"] == release["postgres_image"], "POSTGRES_RELEASE_DIGEST")
        require(env.get("CORS_ALLOWED_ORIGINS") == release["cors_origin"], "BACKEND_CORS_ORIGIN")
        require(secret["file"] == release["postgres_password_file"], "POSTGRES_RELEASE_SECRET_PATH")


def main() -> int:
    if len(sys.argv) == 2:
        release_path = None
        rendered_path = sys.argv[1]
    elif len(sys.argv) == 4 and sys.argv[1] == "--release-manifest":
        release_path = sys.argv[2]
        rendered_path = sys.argv[3]
    else:
        print("usage: check_coolify_private_compose.py [--release-manifest manifest.json] rendered-compose.json",
              file=sys.stderr)
        return 2
    try:
        rendered = json.loads(Path(rendered_path).read_text())
        release = json.loads(Path(release_path).read_text()) if release_path else None
        check(rendered, release)
    except (OSError, ValueError, KeyError, TypeError) as error:
        code = str(error) if isinstance(error, PrivateComposeError) else "INVALID_RENDERED_COMPOSE"
        print(f"PRIVATE_COMPOSE=FAIL {code}", file=sys.stderr)
        return 2
    print("PRIVATE_COMPOSE=PASS; DEPLOYMENT_NOT_AUTHORIZED")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
