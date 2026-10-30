"""Bounded genuine-Anvil receipt ingestion drill; disposable loopback only.

Requires the existing Anvil fixture setup and a disposable PostgreSQL container
named deopt-vps-capacity-pg. Never reads a signing key or sends a public RPC
transaction. Generated receipt bundles stay in /tmp and are not committed.
"""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
from urllib.request import Request, urlopen

REPO = Path(__file__).resolve().parents[2]
RPC = "http://127.0.0.1:18545"
PG_URL = "postgres://postgres@127.0.0.1:15439/deopt_anvil_fees"
FIXTURE = Path("/tmp/deopt_anvil_multifill_actual.json")
PG_CONTAINER = "deopt-vps-capacity-pg"


def rpc(method):
    payload = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": []}).encode()
    with urlopen(Request(RPC, data=payload, headers={"Content-Type": "application/json"}), timeout=10) as response:
        return json.load(response)["result"]


def run(args, timeout):
    started = time.monotonic()
    result = subprocess.run(args, cwd=REPO, capture_output=True, text=True, timeout=timeout, check=False)
    if result.returncode:
        # Do not dump possible signatures, wallet data or provider details.
        raise RuntimeError(f"local command failed: {args[0]} exit={result.returncode}")
    return result.stdout, round(time.monotonic() - started, 3)


def database_stats():
    present, _ = run(["docker", "exec", PG_CONTAINER, "psql", "-U", "postgres", "-d", "deopt_anvil_fees", "-Atc", "SELECT to_regclass('public.canonical_v2_perp_fees') IS NOT NULL"], 20)
    if present.strip() == "f":
        size, _ = run(["docker", "exec", PG_CONTAINER, "psql", "-U", "postgres", "-d", "deopt_anvil_fees", "-Atc", "SELECT pg_database_size(current_database())"], 20)
        return [int(size.strip()), 0, 0, 0]
    if present.strip() != "t":
        raise RuntimeError("unexpected disposable database schema state")
    statement = (
        "SELECT pg_database_size(current_database()),"
        "(SELECT COUNT(*) FROM canonical_v2_perp_fees),"
        "(SELECT COUNT(*) FROM canonical_v2_fill_provenance),"
        "(SELECT COUNT(*) FROM canonical_v2_intent_proofs)"
    )
    out, _ = run(["docker", "exec", PG_CONTAINER, "psql", "-U", "postgres", "-d", "deopt_anvil_fees", "-Atc", statement], 20)
    return [int(value) for value in out.strip().split("|")]


def restart_database():
    run(["docker", "restart", "-t", "10", PG_CONTAINER], 45)
    for _ in range(30):
        try:
            run(["docker", "exec", PG_CONTAINER, "pg_isready", "-U", "postgres", "-d", "deopt_anvil_fees"], 5)
            return
        except RuntimeError:
            time.sleep(1)
    raise RuntimeError("disposable PostgreSQL failed to restart")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--start-index", type=int, default=1)
    parser.add_argument("--batches", type=int, default=4)
    parser.add_argument("--restart-after-index", type=int, default=0)
    args = parser.parse_args()
    if not (1 <= args.start_index <= 20 and 1 <= args.batches <= 10 and args.start_index + args.batches <= 21):
        parser.error("1..10 batches, numbered 1..20, required")
    if int(rpc("eth_chainId"), 16) != 84532 or "anvil" not in rpc("web3_clientVersion").lower():
        raise RuntimeError("disposable Base Sepolia Anvil required")
    before = database_stats()
    for index in range(args.start_index, args.start_index + args.batches):
        stdout, fixture_seconds = run(["node", "tests/fixtures/anvil_real_multifill.cjs"], 360)
        proof_lines = [line.split(" ", 1)[1] for line in stdout.splitlines() if line.startswith("REAL_LOCAL_MULTIFILL_PROOF ")]
        if len(proof_lines) != 1:
            raise RuntimeError("genuine local receipt proof missing")
        proof = json.loads(proof_lines[0])
        if proof.get("emitterCounts", {}).get("0xeea4e5d891b90d3d99e9c4e2c61069911477110e") != 2:
            raise RuntimeError("D6 two-fill proof missing")
        saved = Path(f"/tmp/deopt_vps_capacity_fixture_{index:02d}.json")
        shutil.copyfile(FIXTURE, saved)
        env = {
            "DEOPT_REQUIRE_NEW_INGEST": "1",
            "DEOPT_ANVIL_MULTIFILL_FIXTURE": str(FIXTURE),
            "DEOPT_ANVIL_MULTIFILL_PG_URL": PG_URL,
            "CARGO_BUILD_JOBS": "1",
        }
        with_env = os.environ.copy()
        with_env.update(env)
        started = time.monotonic()
        result = subprocess.run([
            "cargo", "test", "--test", "anvil_real_multifill_fee_history",
            "real_anvil_multifill_receipt_to_postgres_and_api", "--", "--ignored", "--test-threads=1",
        ], cwd=REPO, env=with_env, capture_output=True, text=True, timeout=300, check=False)
        if result.returncode or "1 passed; 0 failed" not in result.stdout:
            raise RuntimeError(f"authenticated receipt ingestion failed at batch {index}; exit={result.returncode}")
        after = database_stats()
        if after[1:] != [value + 2 for value in before[1:]]:
            raise RuntimeError(f"fee/provenance count mismatch at batch {index}")
        print(json.dumps({
            "batch": index,
            "local_tx_hash": proof["txHash"],
            "local_block_hash": proof["blockHash"],
            "fills": 2,
            "fixture_seconds": fixture_seconds,
            "ingest_test_seconds": round(time.monotonic() - started, 3),
            "database_bytes": after[0],
            "fee_rows": after[1],
            "provenance_rows": after[2],
            "intent_proof_rows": after[3],
        }, sort_keys=True), flush=True)
        before = after
        if index == args.restart_after_index:
            restart_database()
            if database_stats() != after:
                raise RuntimeError("database changed across disposable restart")
            print(json.dumps({"postgres_restart_after_batch": index, "state_preserved": True}), flush=True)


if __name__ == "__main__":
    main()
