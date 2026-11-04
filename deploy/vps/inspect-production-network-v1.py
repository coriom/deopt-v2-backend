#!/usr/bin/env python3
"""Bounded, secret-free inspection of the one reviewed Coolify DeOpt stack.

Run as root on the reviewed VPS. It reads Docker metadata and performs one
credential-free TCP connect probe from the proxy's network namespace.
"""

import json
import os
import shutil
import subprocess
import sys

PROJECT = "dxco90jwpdxyennmquv2elyj"
POSTGRES_IMAGE = "postgres:16@sha256:5db6dac2ef5dcff512f4f00a009c3c332b9f2f28f24277175bf3e4a3d02ef83a"


def run(*args: str, timeout: int = 12) -> str:
    result = subprocess.run(args, capture_output=True, text=True, timeout=timeout, check=True)
    return result.stdout.strip()


def container(service: str) -> dict:
    ids = run("docker", "ps", "--filter", f"label=com.docker.compose.project={PROJECT}",
              "--filter", f"label=com.docker.compose.service={service}", "--format", "{{.ID}}")
    if len(ids.splitlines()) != 1:
        raise RuntimeError(f"{service.upper()}_CONTAINER_COUNT")
    return json.loads(run("docker", "inspect", ids))[0]


def networks(item: dict) -> set[str]:
    return set(item["NetworkSettings"]["Networks"])


def volume(item: dict, target: str) -> str | None:
    matches = [m.get("Name") for m in item["Mounts"]
               if m.get("Type") == "volume" and m.get("Destination") == target]
    return matches[0] if len(matches) == 1 else None


def main() -> int:
    if os.geteuid() != 0 or run("hostname", "-s") != "v2202611448349549977":
        print("REVIEWED_ROOT_HOST_REQUIRED", file=sys.stderr)
        return 2
    proxy = json.loads(run("docker", "inspect", "coolify-proxy"))[0]
    backend = container("backend")
    postgres = container("postgres")
    if postgres["Config"]["Image"] != POSTGRES_IMAGE:
        raise RuntimeError("POSTGRES_IMAGE_MISMATCH")
    pg_networks = networks(postgres)
    proxy_networks = networks(proxy)
    backend_networks = networks(backend)
    pgdata = volume(postgres, "/var/lib/postgresql/data")
    if not pgdata:
        raise RuntimeError("POSTGRES_VOLUME_MISSING")
    published = [port for port, bindings in postgres["NetworkSettings"]["Ports"].items() if bindings]
    ready = subprocess.run(["docker", "exec", backend["Id"], "curl", "-fsS", "--max-time", "3",
                            "http://127.0.0.1:8080/ready", "-o", "/dev/null"],
                           capture_output=True, timeout=8).returncode == 0
    tcp = "NO_POSTGRES_TCP_ADDRESS"
    ips = [spec.get("IPAddress") for spec in postgres["NetworkSettings"]["Networks"].values()
           if spec.get("IPAddress")]
    if ips:
        if not shutil.which("nsenter"):
            tcp = "INCONCLUSIVE_NSENTER_UNAVAILABLE"
        else:
            probe = ("import socket,sys; s=socket.socket(); s.settimeout(3); "
                     "print(s.connect_ex((sys.argv[1],5432)))")
            result = subprocess.run([
                "nsenter", "--target", str(proxy["State"]["Pid"]), "--net", "--",
                "python3", "-c", probe, ips[0],
            ], capture_output=True, timeout=8)
            errno = result.stdout.decode().strip() if result.returncode == 0 else ""
            if errno == "0":
                tcp = "REACHABLE"
            elif errno in ("13", "101", "110", "111", "113"):
                tcp = "NOT_REACHABLE"
            else:
                tcp = "INCONCLUSIVE_PROBE_ERROR"
    pg_socket = volume(postgres, "/var/run/postgresql")
    backend_socket = volume(backend, "/var/run/postgresql")
    proxy_socket = volume(proxy, "/var/run/postgresql")
    isolated = (not pg_networks and not published and pg_socket is not None
                and pg_socket == backend_socket and proxy_socket is None
                and backend_networks and backend_networks <= proxy_networks and ready)
    print(f"PROXY_NETWORKS={','.join(sorted(proxy_networks))}")
    print(f"BACKEND_NETWORKS={','.join(sorted(backend_networks))}")
    print(f"POSTGRES_NETWORKS={','.join(sorted(pg_networks)) or 'NONE'}")
    print(f"PROXY_POSTGRES_TCP={tcp}")
    print(f"POSTGRES_PUBLISHED_PORTS={','.join(published) or 'NONE'}")
    print(f"POSTGRES_VOLUME={pgdata}")
    print(f"BACKEND_READY={'PASS' if ready else 'FAIL'}")
    print(f"SOCKET_MOUNT_ISOLATION={'PASS' if isolated else 'FAIL'}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as exc:
        print(f"NETWORK_INSPECTION_FAILED={type(exc).__name__}", file=sys.stderr)
        sys.exit(2)
