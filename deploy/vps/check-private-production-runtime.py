#!/usr/bin/env python3
"""Read-only host checks for the first Coolify-managed DeOpt production stack.

Run as root on the reviewed VPS. Prints identifiers, counts and status codes,
never container environment values or secret contents.
"""

import json
import subprocess
import sys


BACKEND_IMAGE = "ghcr.io/coriom/deopt-v2-backend@sha256:b7ed384e9cae879579cf255fac79cce087de7f7c6922e65940c1147c78382436"
POSTGRES_IMAGE = "postgres:16@sha256:5db6dac2ef5dcff512f4f00a009c3c332b9f2f28f24277175bf3e4a3d02ef83a"
SECRET_FILE = "/srv/deopt/production/secrets/postgres_password"
SECRET_MOUNT = "/run/secrets/postgres_password"


def run(*args: str) -> str:
    result = subprocess.run(args, capture_output=True, text=True, check=False)
    if result.returncode:
        raise RuntimeError(f"COMMAND_FAILED:{args[0]}:{args[1] if len(args) > 1 else ''}:{result.returncode}")
    return result.stdout.strip()


def require(value: bool, code: str) -> None:
    if not value:
        raise RuntimeError(code)


def inspect(kind: str, identity: str) -> dict:
    result = json.loads(run("docker", kind, "inspect", identity))
    require(len(result) == 1, f"{kind.upper()}_INSPECT_COUNT")
    return result[0]


def environment(container: dict) -> dict[str, str]:
    return dict(item.split("=", 1) for item in container["Config"].get("Env", []) if "=" in item)


def no_published_ports(container: dict) -> bool:
    bindings = container["HostConfig"].get("PortBindings") or {}
    ports = container["NetworkSettings"].get("Ports") or {}
    return not bindings and all(value is None for value in ports.values())


def secret_is_mounted(container: dict) -> bool:
    return any(
        mount.get("Type") == "bind"
        and mount.get("Source") == SECRET_FILE
        and mount.get("Destination") == SECRET_MOUNT
        and mount.get("RW") is False
        for mount in container.get("Mounts", [])
    )


def http_status(container_id: str, path: str, method: str = "GET") -> str:
    return run(
        "docker", "exec", container_id, "curl", "--silent", "--output", "/dev/null",
        "--write-out", "%{http_code}", "--max-time", "5", "--request", method,
        f"http://127.0.0.1:8080{path}",
    )


def main() -> None:
    require(run("hostname", "-s") == "v2202611448349549977", "HOST_IDENTITY")
    ids = run("docker", "ps", "-a", "--format", "{{.ID}}").splitlines()
    containers = [inspect("container", identity) for identity in ids]
    backend = [item for item in containers if item["Config"]["Image"] == BACKEND_IMAGE]
    require(len(backend) == 1, "BACKEND_CONTAINER_COUNT")
    backend = backend[0]
    project = backend["Config"].get("Labels", {}).get("com.docker.compose.project", "")
    require(bool(project) and "staging" not in project.lower(), "PRODUCTION_PROJECT_IDENTITY")
    require(backend["Config"]["Labels"].get("com.docker.compose.service") == "backend", "BACKEND_SERVICE")
    postgres = [item for item in containers if item["Config"].get("Labels", {}).get("com.docker.compose.project") == project
                and item["Config"].get("Labels", {}).get("com.docker.compose.service") == "postgres"]
    require(len(postgres) == 1, "POSTGRES_CONTAINER_COUNT")
    postgres = postgres[0]
    require(postgres["Config"]["Image"] == POSTGRES_IMAGE, "POSTGRES_DIGEST")
    project_containers = [item for item in containers if item["Config"].get("Labels", {}).get("com.docker.compose.project") == project]
    require(len(project_containers) == 2, "UNREVIEWED_PROJECT_SERVICE")
    for name, item in (("BACKEND", backend), ("POSTGRES", postgres)):
        require(item["State"].get("Running") is True, f"{name}_NOT_RUNNING")
        require(item["State"].get("Health", {}).get("Status") == "healthy", f"{name}_NOT_HEALTHY")
        require(no_published_ports(item), f"{name}_PUBLISHED_PORT")
        require(secret_is_mounted(item), f"{name}_SECRET_MOUNT")
        require(item["HostConfig"].get("NetworkMode") != "host", f"{name}_HOST_NETWORK")

    backend_networks = set(backend["NetworkSettings"]["Networks"])
    postgres_networks = set(postgres["NetworkSettings"]["Networks"])
    require(len(postgres_networks) == 1 and len(backend_networks) == 2
            and postgres_networks < backend_networks, "NETWORK_ISOLATION")
    for name in backend_networks:
        network = inspect("network", name)
        require(network.get("Labels", {}).get("com.docker.compose.project") == project, "SHARED_NETWORK")
        require(network.get("Internal") is (name in postgres_networks), "NETWORK_INTERNAL_FLAG")

    volumes = [mount for mount in postgres["Mounts"]
               if mount.get("Type") == "volume" and mount.get("Destination") == "/var/lib/postgresql/data"]
    require(len(volumes) == 1, "POSTGRES_VOLUME")
    volume_name = volumes[0].get("Name", "")
    volume = inspect("volume", volume_name)
    require(volume.get("Labels", {}).get("com.docker.compose.project") == project, "SHARED_VOLUME")

    config = backend["Config"]
    host = backend["HostConfig"]
    require(config.get("User") == "1001:1001" and host.get("ReadonlyRootfs") is True, "BACKEND_RUNTIME_IDENTITY")
    require("ALL" in (host.get("CapDrop") or []) and "no-new-privileges:true" in (host.get("SecurityOpt") or []),
            "BACKEND_PRIVILEGES")
    require(int(host.get("Memory") or 0) > 0 and int(host.get("NanoCpus") or 0) > 0, "BACKEND_RESOURCE_LIMITS")
    env = environment(backend)
    expected = {
        "CHAIN_ID": "84532", "NETWORK_NAME": "base-sepolia", "DEOPT_READ_ONLY_RELEASE": "true",
        "PERSISTENCE_ENABLED": "true", "PERPS_READ_ENABLED": "false", "EXECUTION_ENABLED": "false",
        "EXECUTOR_DRY_RUN": "true", "EXECUTOR_REAL_BROADCAST_ENABLED": "false",
        "HV2_BROADCAST_ENABLED": "false", "PERPS_PUBLIC_TRADING_ENABLED": "false",
        "PERPS_CLOSED_TEST_ENABLED": "false", "INDEXER_ENABLED": "false", "OPTIONS_ENABLED": "false",
        "PUBLIC_WS_ENABLED": "false", "METRICS_ENABLED": "false", "MM_GATEWAY_ENABLED": "false",
    }
    require(all(env.get(key) == value for key, value in expected.items()), "BACKEND_ECONOMIC_FLAGS")
    require(not any(token in key.upper() for key in env for token in ("PRIVATE_KEY", "SIGNER_KEY", "MNEMONIC", "KEYSTORE")),
            "SIGNER_VARIABLE")
    require("DATABASE_URL" not in env, "INLINE_DATABASE_SECRET")
    run("docker", "exec", backend["Id"], "test", "-r", SECRET_MOUNT)
    health = http_status(backend["Id"], "/health")
    ready = http_status(backend["Id"], "/ready")
    market = http_status(backend["Id"], "/perps/markets")
    history = http_status(backend["Id"], "/accounts/0x0000000000000000000000000000000000000000/perps/canonical-fees")
    write = http_status(backend["Id"], "/health", "POST")
    require((health, ready, market, history, write) == ("200", "200", "503", "403", "405"), "HTTP_READ_ONLY_GATES")
    migrations = run("docker", "exec", postgres["Id"], "psql", "-X", "-A", "-t", "-v", "ON_ERROR_STOP=1",
                     "-U", "deopt_readonly", "-d", "deopt_readonly", "-c", "SELECT count(*) FROM _sqlx_migrations")
    require(migrations == "71", "MIGRATION_COUNT")
    print("PRIVATE_PRODUCTION_RUNTIME=PASS")
    print(f"COMPOSE_PROJECT={project}")
    print(f"POSTGRES_VOLUME={volume_name}")
    print("BACKEND_IMAGE_DIGEST=PASS; POSTGRES_IMAGE_DIGEST=PASS")
    print("NETWORK_ISOLATION=PASS; PUBLISHED_PORTS=NONE; SECRET_MOUNT=PASS")
    print("ECONOMIC_FLAGS=DISABLED; MIGRATIONS=71")
    print(f"HTTP_STATUS=health:{health},ready:{ready},market:{market},history:{history},write:{write}")


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        print(f"PRIVATE_PRODUCTION_RUNTIME=FAIL:{error}", file=sys.stderr)
        raise SystemExit(2)
