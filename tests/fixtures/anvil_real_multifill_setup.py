"""Disposable local Anvil fork setup. Refuses non-loopback/non-Anvil endpoints."""
import json
import hashlib
from pathlib import Path
import subprocess
import time
import requests

RPC = "http://127.0.0.1:18545"
SOL = "/home/corio/DEOPT/deopt-v2-sol"
SAFE = "0xA6B9Bb5c7B26B33cfD28C6F5A79B3c527fDdcD46"
TL = "0xa67f8e8e673ce4bb2fb563b0e6e9fa8f70e3b588"
D1 = "0x6b3d846536116082dc4e7227861c341bb85ee963"
D2 = "0x9947f42cc29a32992bbf2030ee49e35c839eded5"
D4 = "0xd0901de8f6de72aecc716ca1465c4cf58a0b99fb"
D5 = "0x90caa6d5bc628577bc76d6cc8b775f983c75e110"
D6 = "0xeea4e5d891b90d3d99e9c4e2c61069911477110e"
VAULT = "0x00340c360353a5ab784c5bc5c44322a6af0625d3"
USDC = "0x6eae407f5640b006fac9965182e238582a3b412e"
USDC_OWNER = "0xc35f7a8a103a9a4464adfaa76b9b514093d23c27"


def rpc(method, params):
    for attempt in range(3):
        try:
            result = requests.post(RPC, json={"jsonrpc": "2.0", "id": 1, "method": method, "params": params}, timeout=40).json()
            if "error" in result:
                raise RuntimeError(f"{method}: {result['error']}")
            return result["result"]
        except requests.RequestException:
            if attempt == 2:
                raise
            time.sleep(1)


def calldata(signature, *args):
    return subprocess.check_output(["cast", "calldata", signature, *map(str, args)], text=True).strip()


def send(sender, target, data, label, gas=3_000_000):
    tx_hash = rpc("eth_sendTransaction", [{"from": sender, "to": target, "data": data, "gas": hex(gas)}])
    for _ in range(30):
        receipt = rpc("eth_getTransactionReceipt", [tx_hash])
        if receipt is not None:
            status = int(receipt["status"], 16)
            print(label, tx_hash, "PASS" if status == 1 else "FAIL", flush=True)
            if status != 1:
                raise RuntimeError(f"{label} reverted")
            return receipt
        time.sleep(0.2)
    raise RuntimeError(f"{label} receipt timeout: {tx_hash}")


assert "anvil" in rpc("web3_clientVersion", []).lower()
assert int(rpc("eth_chainId", []), 16) == 84532
assert int(rpc("eth_call", [{"to": D1, "data": calldata("totalMarkets()")}, "latest"]), 16) == 0, "fresh unmodified fork required"
for owner in (SAFE, TL, USDC_OWNER):
    rpc("anvil_impersonateAccount", [owner])
    rpc("anvil_setBalance", [owner, hex(10**20)])

# Only the isolated fork executes the reviewed, already-queued C02 operation.
# Never point this script at a non-loopback RPC. The real market stays absent.
builder_path = Path(SOL) / "artifacts/perps_v2_replacement_deployment/c02_market_creation_s1_safe_builder_review_v1.json"
builder_bytes = builder_path.read_bytes()
assert hashlib.sha256(builder_bytes).hexdigest() == "32275799536bc5bdcbea5effe58f3a3fd6dba608af79c2dd39c0999bee7a8b9c"
intent = json.loads(builder_bytes)["transactions"][0]["contractInputsValues"]
assert intent["target"].lower() == D1 and intent["value"] == "0" and intent["eta"] == "1792929600"
send(SAFE, TL, calldata("executeTransaction(address,uint256,bytes,uint256)", D1, 0, intent["data"], intent["eta"]), "C02_FORK_ONLY", 5_000_000)
assert int(rpc("eth_call", [{"to": D1, "data": calldata("totalMarkets()")}, "latest"]), 16) == 1

users = rpc("eth_accounts", [])
assert len(users) >= 4
buyer, seller, executor = users[0:3]
assert len({buyer.lower(), seller.lower(), executor.lower()}) == 3

bytecode = json.load(open(f"{SOL}/out/PerpClearingAccountV2.sol/PerpClearingAccountV2.json"))["bytecode"]["object"]
constructor = VAULT.lower().removeprefix("0x").rjust(64, "0")
deployed = rpc("eth_sendTransaction", [{"from": TL, "data": bytecode + constructor, "gas": hex(3_000_000)}])
for _ in range(30):
    receipt = rpc("eth_getTransactionReceipt", [deployed])
    if receipt:
        break
    time.sleep(0.2)
assert receipt and int(receipt["status"], 16) == 1
clearing = receipt["contractAddress"]
print("LOCAL_CLEARING_DEPLOY", deployed, clearing, flush=True)

steps = [
    (TL, D4, "setMatchingEngine(address)", (D6,), "D4_matching"),
    (TL, D4, "setClearingAccount(address)", (clearing,), "D4_clearing"),
    (TL, D4, "setFeesManagerV2(address)", (D2,), "D4_fee_pointer"),
    (TL, D4, "setUseFeesManagerV2(bool)", ("true",), "D4_D2_enabled"),
    (TL, D4, "sealMigration(bytes32)", ("0x" + "01" * 32,), "D4_local_seal"),
    (TL, D4, "setEmergencyModes(bool,bool,bool,bool)", ("false",) * 4, "D4_local_unpause"),
    (TL, D6, "setExecutor(address,bool)", (executor, "true"), "D6_local_executor"),
    (TL, D6, "unpause()", (), "D6_local_unpause"),
    (TL, D2, "setFeeConsumer(address,bool)", (D4, "true"), "D2_local_consumer"),
    (TL, VAULT, "setAuthorizedEngine(address,bool)", (D4, "true"), "Vault_local_engine"),
]
for sender, target, sig, args, label in steps:
    send(sender, target, calldata(sig, *args), label)

print("LOCAL_FIXTURE_USERS", buyer, seller, executor, flush=True)
print("LOCAL_FIXTURE_READY", flush=True)
