"""Pin the reviewed official Coolify installer inputs before privileged use.

The installed wrapper calls prepare() with the fixed root-owned directory.
Tests may supply an offline source directory; no secrets are read or logged.
"""

import hashlib
import json
from pathlib import Path
import subprocess
from tempfile import TemporaryDirectory
from urllib.parse import urlsplit

CDN = "https://cdn.coollabs.io/coolify"
VENDOR = Path("/root/deopt-bootstrap/vendor")
HASHES = {
    "install.sh": "7ef7ff6e5ee4738025d5003cacfca4fcc36df0a8844cc324f6ebb4a20b2dff04",
    "docker-compose.yml": "2532ee9258e10ad4e86cb3c2b46c29fe9329f5a0dace0f7da410744feec279bd",
    "docker-compose.prod.yml": "f8b104aadfca48d0033fdee6cb87fea13c68bc5c67c63ba18b1934e9f901ff5d",
    ".env.production": "7895e54f7c9d40063e76afec9b35839048d91e5c3b5107d64ea49ef6987c304a",
    "upgrade.sh": "a9ca9686e1b76b04b34995ae59f60aae3196774a84c23e46c8fcc74849d7fb44",
    "upgrade-postgres.sh": "aa65c277fc13ab5b27c349fbfef12defc59c084b244e4b258329e1f6c570ae8d",
    "versions.json": "3b4e05bb558b7fcca41fae174d750650b83277129dc0d5cbd11418934c292891",
}


def download_curl(name: str, directory: Path, runner=subprocess.run) -> bytes:
    """Fetch one pinned public input over TLS without exposing its body to logs."""
    with TemporaryDirectory(prefix=".coolify-fetch-", dir=directory) as temporary:
        output = Path(temporary) / "download"
        result = runner(
            [
                "curl", "--fail", "--silent", "--show-error", "--location",
                "--proto", "=https", "--proto-redir", "=https", "--tlsv1.2",
                "--connect-timeout", "5", "--max-time", "30",
                "--retry", "2", "--retry-all-errors", "--retry-delay", "1",
                "--retry-max-time", "60", "--max-filesize", "1000000",
                "--output", str(output), "--write-out", "%{url_effective}",
                f"{CDN}/{name}",
            ], capture_output=True, text=True, check=False,
        )
        if result.returncode != 0:
            raise RuntimeError(f"Coolify HTTPS download failed: {name} (curl {result.returncode})")
        if urlsplit(result.stdout.strip()).scheme != "https":
            raise ValueError(f"Coolify source redirected outside HTTPS: {name}")
        return output.read_bytes()


def verify_reviewed_content(name: str, content: bytes, expected: str) -> None:
    if len(content) > 1_000_000 or hashlib.sha256(content).hexdigest() != expected:
        raise ValueError(f"reviewed Coolify input changed: {name}")


def patch_cdn(source: bytes, directory: Path) -> bytes:
    needle = b'CDN="https://cdn.coollabs.io/coolify"'
    if source.count(needle) != 1:
        raise ValueError("reviewed CDN assignment changed")
    return source.replace(needle, f'CDN="file://{directory}"'.encode(), 1)


def install_port_guard(source: bytes) -> bytes:
    needle = 'step_start "Pulling images" "coolify · postgres · redis · helper"'.encode()
    if source.count(needle) != 1:
        raise ValueError("reviewed pre-start insertion point changed")
    guard = b'''# DeOpt first-start gate: fail before any Coolify container is started.
if ! docker compose --env-file /data/coolify/source/.env \\
  -f /data/coolify/source/docker-compose.yml \\
  -f /data/coolify/source/docker-compose.prod.yml \\
  -f /data/coolify/source/docker-compose.custom.yml \\
  config --format json | jq -e '
    ([.services.coolify.ports[]? | [.host_ip, (.published|tostring), (.target|tostring)]] | sort)
      == [["127.0.0.1","6001","6001"],["127.0.0.1","6002","6002"],["127.0.0.1","8000","8080"]]
    and ([.services[] | .ports[]?] | length) == 3
  ' >/dev/null; then
    echo 'Coolify private-port pre-start gate failed' >&2
    exit 1
fi
'''
    return source.replace(needle, guard + needle, 1)


def prepare(directory: Path = VENDOR, source_dir: Path | None = None) -> dict:
    directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    directory.chmod(0o700)
    reviewed = {}
    for name, expected in HASHES.items():
        if source_dir:
            content = (source_dir / name).read_bytes()
        else:
            content = download_curl(name, directory)
        verify_reviewed_content(name, content, expected)
        reviewed[name] = content
    versions = json.loads(reviewed["versions.json"])
    if versions["coolify"]["v4"]["version"] != "4.5.3" or versions["coolify"]["helper"]["version"] != "1.0.19":
        raise ValueError("reviewed Coolify release changed")
    reviewed["install.sh"] = install_port_guard(patch_cdn(reviewed["install.sh"], directory))
    reviewed["upgrade.sh"] = patch_cdn(reviewed["upgrade.sh"], directory)
    for name, content in reviewed.items():
        path = directory / name
        path.write_bytes(content)
        path.chmod(0o600)
    for name in ("install.sh", "upgrade.sh", "upgrade-postgres.sh"):
        subprocess.run(["bash", "-n", str(directory / name)], check=True, capture_output=True)
    return {name: hashlib.sha256(content).hexdigest() for name, content in reviewed.items()}


if __name__ == "__main__":
    result = prepare()
    print("COOLIFY_VENDOR_REVIEW=PASS version=4.5.3 helper=1.0.19")
    print("PATCHED_INSTALLER_SHA256=" + result["install.sh"])
