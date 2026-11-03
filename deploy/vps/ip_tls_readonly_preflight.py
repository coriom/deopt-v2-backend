#!/usr/bin/env python3
"""Bounded root-only evidence for a Coolify/Traefik IP certificate review.

Never emits container environment, private keys, ACME storage, or Compose content.
This script does not modify the host.
"""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

EXPECTED_HOST = "v2202611448349549977"
PROXY_COMPOSE = Path("/data/coolify/proxy/docker-compose.yml")
DYNAMIC = Path("/data/coolify/proxy/dynamic")


def proxy_evidence(inspected: dict) -> dict[str, str]:
    cmd = inspected.get("Config", {}).get("Cmd") or []
    mounts = inspected.get("Mounts") or []
    networks = inspected.get("NetworkSettings", {}).get("Networks") or {}
    flags = [part for part in cmd if isinstance(part, str) and part.startswith("--")]
    def setting(name: str) -> str:
        prefix = f"--certificatesresolvers.letsencrypt.acme.{name}="
        return next((f[len(prefix):] for f in flags if f.lower().startswith(prefix)), "UNSET")

    ca = setting("caserver")
    ca_kind = ("DEFAULT" if ca == "UNSET" else
               "LE_STAGING" if ca == "https://acme-staging-v02.api.letsencrypt.org/directory" else
               "OTHER")
    return {
        "PROXY_IMAGE": inspected.get("Config", {}).get("Image", "UNKNOWN"),
        "PROXY_RUNNING": str(inspected.get("State", {}).get("Running") is True),
        "PROXY_NETWORKS": ",".join(sorted(networks)),
        "PROXY_TRAEFIK_MOUNT": str(any(m.get("Destination") == "/traefik"
                                        and m.get("Source") == "/data/coolify/proxy"
                                        for m in mounts)),
        "FILE_WATCH": str(any("--providers.file.watch=true" == f.lower() for f in flags)),
        "FILE_DIRECTORY": next((f.split("=", 1)[1] for f in flags
                                if f.startswith("--providers.file.directory=")), "UNKNOWN"),
        "ENTRYPOINTS": ",".join(sorted(f.split("=", 1)[0][14:] for f in flags
                                     if f.startswith("--entrypoints.") and ".address=" in f)),
        "ACME_RESOLVERS": ",".join(sorted({f.split(".")[1] for f in flags
                                            if f.startswith("--certificatesresolvers.")})),
        "ACME_HTTP_ENTRYPOINT": setting("httpchallenge.entrypoint"),
        "ACME_TLS_CHALLENGE": setting("tlschallenge"),
        "ACME_PROFILE": setting("profile"),
        "ACME_CERTIFICATES_DURATION": setting("certificatesduration"),
        "ACME_DISABLE_COMMON_NAME": setting("disablecommonname"),
        "ACME_CA_SERVER_KIND": ca_kind,
        "ACME_STORAGE_CONFIGURED": str(setting("storage") != "UNSET"),
    }


def run(*args: str) -> str:
    completed = subprocess.run(args, text=True, capture_output=True, timeout=15, check=False)
    if completed.returncode:
        raise RuntimeError(f"{args[0]} exited {completed.returncode}")
    return completed.stdout.strip()


def main() -> int:
    if os.geteuid() != 0:
        print("ROOT_REQUIRED", file=sys.stderr)
        return 2
    if run("hostname") != EXPECTED_HOST:
        print("WRONG_HOST", file=sys.stderr)
        return 2
    try:
        proxy = json.loads(run("docker", "inspect", "coolify-proxy"))[0]
        evidence = proxy_evidence(proxy)
        evidence["TRAEFIK_VERSION"] = run("docker", "exec", "coolify-proxy", "traefik", "version").splitlines()[0]
        evidence["PROXY_COMPOSE_SHA256"] = (hashlib.sha256(PROXY_COMPOSE.read_bytes()).hexdigest()
                                             if PROXY_COMPOSE.is_file() else "UNAVAILABLE")
        evidence["DYNAMIC_DIRECTORY"] = str(DYNAMIC.is_dir())
        names = []
        has_default = False
        if DYNAMIC.is_dir():
            for path in sorted(DYNAMIC.iterdir()):
                if path.is_file() and path.suffix in (".yaml", ".yml"):
                    names.append(path.name)
                    contents = path.read_text(errors="replace")
                    has_default |= "defaultCertificate:" in contents or "defaultGeneratedCert:" in contents
        evidence["DYNAMIC_FILES"] = ",".join(names)
        evidence["DEFAULT_TLS_CONFIG_PRESENT"] = str(has_default)
        for key, value in evidence.items():
            print(f"{key}={value}")
        return 0
    except (ValueError, RuntimeError, OSError, subprocess.TimeoutExpired) as exc:
        print(f"PREFLIGHT_FAILED={type(exc).__name__}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
