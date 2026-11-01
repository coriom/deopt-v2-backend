"""Review-only plan for removing Coolify's unused published 8080 mapping.

This transforms Docker inspect *evidence*, not Coolify's saved configuration.
The actual persistent edit must be reviewed in Coolify's Proxy UI separately.
"""

from copy import deepcopy


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
