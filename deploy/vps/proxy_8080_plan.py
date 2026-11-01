"""Review-only plan for removing Coolify's unused published 8080 mapping.

This transforms Docker inspect *evidence*, not Coolify's saved configuration.
The actual persistent edit must be reviewed in Coolify's Proxy UI separately.
"""

from copy import deepcopy


def reconcile_listeners(bindings: dict, ss_output: str) -> dict[str, bool]:
    """Compare Docker's recorded binds with actual host sockets.

    Docker may spawn an IPv6 proxy even if PortBindings only lists IPv4.
    A listener is exposure potential, not proof of external reachability.
    """
    if not isinstance(bindings, dict) or not isinstance(ss_output, str):
        raise ValueError("INVALID_PROXY_EVIDENCE")
    bound = bindings.get("8080/tcp") or []
    endpoints = {line.split()[3] for line in ss_output.splitlines()
                 if len(line.split()) >= 4 and line.split()[0] == "LISTEN"}
    return {
        "docker_ipv4": any(item.get("HostIp") in ("0.0.0.0", "") for item in bound),
        "docker_ipv6": any(item.get("HostIp") == "::" for item in bound),
        "listen_ipv4": "0.0.0.0:8080" in endpoints,
        "listen_ipv6": "[::]:8080" in endpoints,
    }


def removal_candidate(bindings: dict) -> tuple[dict, dict]:
    if not isinstance(bindings, dict):
        raise ValueError("INVALID_BINDINGS")
    for port in ("80/tcp", "443/tcp", "8080/tcp"):
        values = bindings.get(port)
        if not isinstance(values, list) or not values:
            raise ValueError("EXPECTED_PROXY_PORT_MISSING")
    before = deepcopy(bindings)
    after = deepcopy(bindings)
    del after["8080/tcp"]
    if any(before.get(key) != after.get(key) for key in before if key != "8080/tcp"):
        raise ValueError("UNRELATED_PROXY_PORT_CHANGED")
    return before, after
