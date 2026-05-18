#!/usr/bin/env python3
"""Safe local/runtime E2E harness for deopt-v2-backend.

The harness intentionally uses only the Python standard library.  It
orchestrates existing HTTP endpoints and PostgreSQL read checks; it does not
add protocol features, broadcast transactions, or edit .env files.
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from dataclasses import dataclass, field
from typing import Any, Callable, Dict, Iterable, List, Optional, Sequence


DEFAULT_BACKEND_URL = "http://127.0.0.1:8080"
DEFAULT_DATABASE_URL = "postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend"
DEFAULT_ADMIN_TOKEN = "local-admin-token-runtime-test"

MAKER_ACCOUNT = "0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3"
TAKER_ACCOUNT = "0xc0A76c2A6c6b70C0B065A05E64417886416cc976"

FLOW_CHOICES = (
    "admin",
    "fees-options",
    "fees-perps",
    "option-rfq",
    "mm-auth",
    "all-safe",
)


class E2EError(RuntimeError):
    pass


class E2ESkip(RuntimeError):
    pass


def now_ms() -> int:
    return int(time.time() * 1000)


def sanitize_text(value: str, ctx: "Context") -> str:
    redactions = [
        (ctx.admin_token, "<redacted-admin-token>"),
        (ctx.database_url, "<redacted-database-url>"),
        (os.environ.get("RPC_URL", ""), "<redacted-rpc-url>"),
        (os.environ.get("PRIVATE_KEY", ""), "<redacted-private-key>"),
        (os.environ.get("DEPLOYER_PRIVATE_KEY", ""), "<redacted-deployer-private-key>"),
        (os.environ.get("EXECUTOR_PRIVATE_KEY", ""), "<redacted-executor-private-key>"),
        (os.environ.get("MM_PRIVATE_KEY", ""), "<redacted-mm-private-key>"),
    ]
    output = value
    for secret, replacement in redactions:
        if secret:
            output = output.replace(secret, replacement)
    return output


def sql_literal(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def one_line(value: Any, limit: int = 500) -> str:
    text = str(value).replace("\n", " ").strip()
    if len(text) > limit:
        return text[: limit - 3] + "..."
    return text


@dataclass
class FlowReport:
    flow: str
    started_at_ms: int = field(default_factory=now_ms)
    finished_at_ms: Optional[int] = None
    checks: List[Dict[str, Any]] = field(default_factory=list)
    artifacts: Dict[str, Any] = field(default_factory=dict)
    errors: List[str] = field(default_factory=list)
    ok: bool = True

    def add_check(
        self,
        name: str,
        ok: bool,
        details: Optional[Dict[str, Any]] = None,
        *,
        error: Optional[str] = None,
        skipped: bool = False,
    ) -> None:
        check: Dict[str, Any] = {
            "name": name,
            "ok": ok,
            "details": details or {},
        }
        if skipped:
            check["skipped"] = True
        if error:
            check["error"] = error
        self.checks.append(check)
        if not ok:
            self.ok = False
            if error:
                self.errors.append(error)

    def finish(self) -> Dict[str, Any]:
        self.finished_at_ms = now_ms()
        return {
            "ok": self.ok,
            "flow": self.flow,
            "started_at_ms": self.started_at_ms,
            "finished_at_ms": self.finished_at_ms,
            "checks": self.checks,
            "artifacts": self.artifacts,
            "errors": self.errors,
        }


@dataclass
class Context:
    args: argparse.Namespace
    backend_url: str
    database_url: str
    admin_token: str
    repo_root: str
    run_id: str = field(default_factory=lambda: uuid.uuid4().hex[:12])
    backend_process: Optional[subprocess.Popen] = None
    backend_log_path: Optional[str] = None
    cleanup_actions: List[Callable[[], None]] = field(default_factory=list)
    current_report: Optional[FlowReport] = None


def step(report: FlowReport, ctx: Context, name: str, fn: Callable[[], Dict[str, Any]]) -> Dict[str, Any]:
    try:
        details = fn()
    except E2ESkip as exc:
        message = sanitize_text(str(exc), ctx)
        report.add_check(name, True, {"reason": message}, skipped=True)
        return {"skipped": True, "reason": message}
    except Exception as exc:  # noqa: BLE001 - report any flow failure as a check failure.
        message = sanitize_text(one_line(exc), ctx)
        report.add_check(name, False, error=message)
        raise
    report.add_check(name, True, details)
    return details


def require(condition: bool, message: str) -> None:
    if not condition:
        raise E2EError(message)


def backend_origin(backend_url: str) -> str:
    parsed = urllib.parse.urlparse(backend_url)
    if not parsed.scheme or not parsed.netloc:
        raise E2EError(f"invalid backend URL: {backend_url}")
    return f"{parsed.scheme}://{parsed.netloc}".rstrip("/")


def backend_host_port(backend_url: str) -> tuple[str, int]:
    parsed = urllib.parse.urlparse(backend_url)
    host = parsed.hostname or "127.0.0.1"
    if parsed.port:
        return host, parsed.port
    return host, 443 if parsed.scheme == "https" else 80


def http_json(
    ctx: Context,
    method: str,
    path: str,
    *,
    body: Optional[Dict[str, Any]] = None,
    admin_token: Optional[str] = None,
    expect_status: int = 200,
    timeout_sec: Optional[float] = None,
) -> Any:
    url = path if path.startswith("http://") or path.startswith("https://") else ctx.backend_url + path
    data = None
    headers = {"Accept": "application/json"}
    if body is not None:
        data = json.dumps(body).encode("utf-8")
        headers["Content-Type"] = "application/json"
    if admin_token is not None:
        headers["X-Admin-Token"] = admin_token
    request = urllib.request.Request(url, data=data, headers=headers, method=method)
    timeout = timeout_sec if timeout_sec is not None else min(ctx.args.timeout_sec, 30)
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            payload = response.read().decode("utf-8")
            status = response.status
    except urllib.error.HTTPError as error:
        payload = error.read().decode("utf-8", errors="replace")
        status = error.code
        if status != expect_status:
            raise E2EError(f"{method} {path} returned HTTP {status}: {payload}")
    except urllib.error.URLError as error:
        raise E2EError(f"{method} {path} failed: {error}") from error

    if status != expect_status:
        raise E2EError(f"{method} {path} returned HTTP {status}, expected {expect_status}")
    if not payload:
        return None
    try:
        return json.loads(payload)
    except json.JSONDecodeError:
        return {"raw": payload}


def admin_get(ctx: Context, path: str) -> Any:
    return http_json(ctx, "GET", path, admin_token=ctx.admin_token)


def psql_rows(ctx: Context, sql: str, columns: Sequence[str]) -> List[Dict[str, str]]:
    if not shutil.which("psql"):
        raise E2EError("psql is not available on PATH")
    cmd = [
        "psql",
        ctx.database_url,
        "-X",
        "-q",
        "-v",
        "ON_ERROR_STOP=1",
        "-t",
        "-A",
        "-F",
        "\t",
        "-c",
        sql,
    ]
    try:
        completed = subprocess.run(
            cmd,
            cwd=ctx.repo_root,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=ctx.args.timeout_sec,
            check=False,
        )
    except subprocess.TimeoutExpired as exc:
        raise E2EError("psql timed out") from exc
    if completed.returncode != 0:
        raise E2EError(f"psql query failed: {completed.stderr.strip()}")

    rows: List[Dict[str, str]] = []
    for line in completed.stdout.splitlines():
        if not line.strip():
            continue
        values = line.split("\t")
        while len(values) < len(columns):
            values.append("")
        rows.append(dict(zip(columns, values[: len(columns)])))
    return rows


def psql_scalar(ctx: Context, sql: str) -> str:
    rows = psql_rows(ctx, sql, ["value"])
    if not rows:
        return ""
    return rows[0]["value"]


def build_backend_env(ctx: Context, flow: str) -> Dict[str, str]:
    host, port = backend_host_port(ctx.backend_url)
    confirmation_enabled = "true" if flow in {"fees-perps", "all-safe"} else "false"
    rpc_url = os.environ.get("RPC_URL", "") if flow in {"fees-perps", "all-safe"} else ""
    env = os.environ.copy()
    env.update(
        {
            "HOST": host,
            "PORT": str(port),
            "RUST_LOG": env.get("RUST_LOG", "info"),
            "PERSISTENCE_ENABLED": "true",
            "DATABASE_URL": ctx.database_url,
            "ADMIN_API_ENABLED": "true",
            "ADMIN_API_REQUIRE_TOKEN": "true",
            "ADMIN_API_TOKEN": ctx.admin_token,
            "EXECUTION_ENABLED": "false",
            "EXECUTOR_DRY_RUN": "true",
            "EXECUTOR_REAL_BROADCAST_ENABLED": "false",
            "EXECUTOR_PRIVATE_KEY": "",
            "PRIVATE_KEY": "",
            "DEPLOYER_PRIVATE_KEY": "",
            "MM_PRIVATE_KEY": "",
            "RPC_URL": rpc_url,
            "SIMULATION_ENABLED": "false",
            "INDEXER_ENABLED": "false",
            "RECONCILIATION_ENABLED": "false",
            "CONFIRMATION_ENABLED": confirmation_enabled,
            "CONFIRMATION_REQUIRE_PERSISTENCE": "true",
            "CONFIRMATION_REQUIRE_RECONCILIATION": "true",
            "PERP_NONCE_SYNC_ENABLED": "false",
            "RFQ_ENABLED": "false",
            "RFQ_QUOTE_SIGNATURE_MODE": "disabled",
            "OPTIONS_ENABLED": "true",
            "OPTIONS_REQUIRE_PERSISTENCE": "true",
            "OPTIONS_ALLOW_MANUAL_SERIES": "true",
            "OPTIONS_SYNC_ONCHAIN_REGISTRY": "false",
            "OPTION_RFQ_ENABLED": "true",
            "OPTION_RFQ_REQUIRE_PERSISTENCE": "true",
            "OPTION_RFQ_QUOTE_SIGNATURE_MODE": "disabled",
            "FEES_ENABLED": "true",
            "FEES_REQUIRE_PERSISTENCE": "true",
            "FEES_REBATES_ENABLED": "true",
            "FEES_PROTOCOL_FEE_RECIPIENT": "treasury",
            "FEES_DEFAULT_FEE_ASSET": "USDC",
            "MM_GATEWAY_ENABLED": "false",
            "MM_GATEWAY_AUTH_MODE": "disabled",
            "MM_GATEWAY_REQUIRE_AUTH": "false",
            "MM_PERMISSIONS_ENABLED": "false",
            "MM_PERMISSIONS_REQUIRE_PERSISTENCE": "true",
            "SIGNATURE_VERIFICATION_MODE": "disabled",
        }
    )
    return env


def start_backend(ctx: Context, report: FlowReport) -> None:
    if ctx.args.flow == "mm-auth":
        return
    env = build_backend_env(ctx, ctx.args.flow)
    log_file = tempfile.NamedTemporaryFile(
        mode="w+b",
        prefix="deopt-e2e-backend-",
        suffix=".log",
        delete=False,
    )
    ctx.backend_log_path = log_file.name
    report.artifacts["backend_log_path"] = ctx.backend_log_path
    ctx.backend_process = subprocess.Popen(
        ["cargo", "run", "--bin", "deopt-v2-backend"],
        cwd=ctx.repo_root,
        env=env,
        stdout=log_file,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )

    deadline = time.time() + ctx.args.timeout_sec
    last_error = ""
    while time.time() < deadline:
        if ctx.backend_process.poll() is not None:
            raise E2EError(
                f"backend exited during startup with code {ctx.backend_process.returncode}; "
                f"log: {ctx.backend_log_path}"
            )
        try:
            health = http_json(ctx, "GET", "/health", timeout_sec=2)
            if health.get("ok") is True:
                return
        except Exception as exc:  # noqa: BLE001 - health may fail while server is booting.
            last_error = one_line(exc)
        time.sleep(0.5)
    raise E2EError(f"backend did not become healthy: {last_error}; log: {ctx.backend_log_path}")


def stop_backend(ctx: Context) -> None:
    process = ctx.backend_process
    if process is None:
        return
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
        process.wait(timeout=10)
    except Exception:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except Exception:
            pass
        try:
            process.wait(timeout=5)
        except Exception:
            pass


def run_cleanups(ctx: Context, report: FlowReport) -> None:
    failures: List[str] = []
    for cleanup in reversed(ctx.cleanup_actions):
        try:
            cleanup()
        except Exception as exc:  # noqa: BLE001 - cleanup failures are reported, not hidden.
            failures.append(sanitize_text(one_line(exc), ctx))
    if failures:
        report.add_check("cleanup", False, error="; ".join(failures))
    elif ctx.cleanup_actions:
        report.add_check("cleanup", True, {"temporary_db_state_restored": True})


def validate_config_for_options(config: Dict[str, Any]) -> None:
    features = config.get("features", {})
    require(features.get("options_enabled") is True, "OPTIONS_ENABLED must be true")
    require(features.get("option_rfq_enabled") is True, "OPTION_RFQ_ENABLED must be true")


def validate_config_for_fees(config: Dict[str, Any]) -> None:
    features = config.get("features", {})
    fees = config.get("fees", {})
    require(features.get("fees_enabled") is True, "FEES_ENABLED must be true")
    require(fees.get("require_persistence") is True, "FEES_REQUIRE_PERSISTENCE must be true")


def config_mm_permissions_enabled(config: Dict[str, Any]) -> bool:
    return bool(config.get("features", {}).get("mm_permissions_enabled"))


def seed_mm_permissions(
    ctx: Context,
    account: str,
    *,
    can_quote_option_rfq: bool = False,
    can_submit_option_orders: bool = False,
    can_submit_perp_orders: bool = False,
    can_quote_perp_rfq: bool = False,
    option_series_id: Optional[str] = None,
    market_id: Optional[str] = None,
) -> Dict[str, Any]:
    account_lit = sql_literal(account)
    existing = psql_rows(
        ctx,
        f"""
        SELECT mm_account, enabled::text, COALESCE(label, '') AS label,
               (label IS NULL)::text AS label_is_null,
               can_submit_perp_orders::text, can_quote_perp_rfq::text,
               can_quote_option_rfq::text, can_submit_option_orders::text,
               created_at_ms::text, updated_at_ms::text
        FROM mm_accounts
        WHERE lower(mm_account)=lower({account_lit})
        """,
        [
            "mm_account",
            "enabled",
            "label",
            "label_is_null",
            "can_submit_perp_orders",
            "can_quote_perp_rfq",
            "can_quote_option_rfq",
            "can_submit_option_orders",
            "created_at_ms",
            "updated_at_ms",
        ],
    )
    now = now_ms()
    bool_sql = lambda value: "true" if value else "false"
    psql_rows(
        ctx,
        f"""
        INSERT INTO mm_accounts (
            mm_account, enabled, label,
            can_submit_perp_orders, can_quote_perp_rfq,
            can_quote_option_rfq, can_submit_option_orders,
            created_at_ms, updated_at_ms
        ) VALUES (
            {account_lit}, true, 'e2e-harness',
            {bool_sql(can_submit_perp_orders)}, {bool_sql(can_quote_perp_rfq)},
            {bool_sql(can_quote_option_rfq)}, {bool_sql(can_submit_option_orders)},
            {now}, {now}
        )
        ON CONFLICT (mm_account) DO UPDATE SET
            enabled = true,
            can_submit_perp_orders = mm_accounts.can_submit_perp_orders OR EXCLUDED.can_submit_perp_orders,
            can_quote_perp_rfq = mm_accounts.can_quote_perp_rfq OR EXCLUDED.can_quote_perp_rfq,
            can_quote_option_rfq = mm_accounts.can_quote_option_rfq OR EXCLUDED.can_quote_option_rfq,
            can_submit_option_orders = mm_accounts.can_submit_option_orders OR EXCLUDED.can_submit_option_orders,
            updated_at_ms = EXCLUDED.updated_at_ms
        RETURNING mm_account
        """,
        ["mm_account"],
    )

    inserted_scope_ids: List[str] = []
    if option_series_id:
        inserted_scope_ids.extend(ensure_option_scope(ctx, account, option_series_id))
    if market_id is not None:
        inserted_scope_ids.extend(ensure_market_scope(ctx, account, market_id))

    def cleanup() -> None:
        for scope_id in inserted_scope_ids:
            psql_rows(
                ctx,
                f"DELETE FROM mm_market_permissions WHERE id={sql_literal(scope_id)} RETURNING id",
                ["id"],
            )
        if existing:
            row = existing[0]
            label_sql = "NULL" if row["label_is_null"] == "true" else sql_literal(row["label"])
            psql_rows(
                ctx,
                f"""
                UPDATE mm_accounts SET
                    enabled = {row['enabled']},
                    label = {label_sql},
                    can_submit_perp_orders = {row['can_submit_perp_orders']},
                    can_quote_perp_rfq = {row['can_quote_perp_rfq']},
                    can_quote_option_rfq = {row['can_quote_option_rfq']},
                    can_submit_option_orders = {row['can_submit_option_orders']},
                    created_at_ms = {row['created_at_ms']},
                    updated_at_ms = {row['updated_at_ms']}
                WHERE lower(mm_account)=lower({account_lit})
                RETURNING mm_account
                """,
                ["mm_account"],
            )
        else:
            psql_rows(
                ctx,
                f"DELETE FROM mm_accounts WHERE lower(mm_account)=lower({account_lit}) RETURNING mm_account",
                ["mm_account"],
            )

    ctx.cleanup_actions.append(cleanup)
    return {
        "account": account,
        "existing_account": bool(existing),
        "temporary_scope_ids": inserted_scope_ids,
    }


def ensure_option_scope(ctx: Context, account: str, option_series_id: str) -> List[str]:
    account_lit = sql_literal(account)
    rows = psql_rows(
        ctx,
        f"""
        SELECT id, enabled::text, COALESCE(option_series_id, '') AS option_series_id
        FROM mm_market_permissions
        WHERE lower(mm_account)=lower({account_lit}) AND market_id IS NULL
        """,
        ["id", "enabled", "option_series_id"],
    )
    has_option_scope = bool(rows)
    for row in rows:
        if row["enabled"] == "true" and (
            row["option_series_id"] == ""
            or row["option_series_id"].lower() == option_series_id.lower()
        ):
            return []
    if not has_option_scope:
        return []
    scope_id = f"e2e-{uuid.uuid4()}"
    psql_rows(
        ctx,
        f"""
        INSERT INTO mm_market_permissions (
            id, mm_account, market_id, option_series_id, enabled, created_at_ms, updated_at_ms
        ) VALUES (
            {sql_literal(scope_id)}, {account_lit}, NULL, {sql_literal(option_series_id)}, true,
            {now_ms()}, {now_ms()}
        )
        RETURNING id
        """,
        ["id"],
    )
    return [scope_id]


def ensure_market_scope(ctx: Context, account: str, market_id: str) -> List[str]:
    account_lit = sql_literal(account)
    rows = psql_rows(
        ctx,
        f"""
        SELECT id, enabled::text, COALESCE(market_id::text, '') AS market_id
        FROM mm_market_permissions
        WHERE lower(mm_account)=lower({account_lit}) AND option_series_id IS NULL
        """,
        ["id", "enabled", "market_id"],
    )
    has_market_scope = bool(rows)
    for row in rows:
        if row["enabled"] == "true" and (row["market_id"] == "" or row["market_id"] == market_id):
            return []
    if not has_market_scope:
        return []
    scope_id = f"e2e-{uuid.uuid4()}"
    psql_rows(
        ctx,
        f"""
        INSERT INTO mm_market_permissions (
            id, mm_account, market_id, option_series_id, enabled, created_at_ms, updated_at_ms
        ) VALUES (
            {sql_literal(scope_id)}, {account_lit}, {market_id}, NULL, true,
            {now_ms()}, {now_ms()}
        )
        RETURNING id
        """,
        ["id"],
    )
    return [scope_id]


def create_option_series(ctx: Context, suffix: str) -> Dict[str, Any]:
    expiry = int(time.time()) + 30 * 24 * 60 * 60
    strike = 300_000_000_000 + (now_ms() % 1_000_000)
    return http_json(
        ctx,
        "POST",
        "/options/series",
        body={
            "underlying": f"ETH-E2E-{suffix}",
            "base_asset": "ETH",
            "quote_asset": "USDC",
            "settlement_asset": "USDC",
            "expiry": expiry,
            "strike_1e8": str(strike),
            "is_call": True,
            "contract_size_1e8": "100000000",
            "onchain_product_id": None,
            "onchain_series_id": None,
        },
    )


def submit_option_order(
    ctx: Context,
    *,
    option_series_id: str,
    account: str,
    side: str,
    price_1e8: str,
    size_1e8: str,
    client_order_id: str,
) -> Dict[str, Any]:
    return http_json(
        ctx,
        "POST",
        "/options/orders",
        body={
            "option_series_id": option_series_id,
            "account": account,
            "side": side,
            "price_1e8": price_1e8,
            "size_1e8": size_1e8,
            "time_in_force": "gtc",
            "client_order_id": client_order_id,
            "nonce": None,
            "deadline_ms": None,
            "signature": None,
        },
    )


def create_option_order_fill(ctx: Context, option_series_id: str, suffix: str) -> Dict[str, Any]:
    maker_order = submit_option_order(
        ctx,
        option_series_id=option_series_id,
        account=MAKER_ACCOUNT,
        side="sell",
        price_1e8="1000000000",
        size_1e8="100000000",
        client_order_id=f"e2e-{suffix}-maker",
    )
    taker_order = submit_option_order(
        ctx,
        option_series_id=option_series_id,
        account=TAKER_ACCOUNT,
        side="buy",
        price_1e8="1000000000",
        size_1e8="100000000",
        client_order_id=f"e2e-{suffix}-taker",
    )
    fills = taker_order.get("fills", [])
    require(len(fills) == 1, f"expected one option order fill, got {len(fills)}")
    return {
        "maker_order_id": maker_order["order_id"],
        "taker_order_id": taker_order["order_id"],
        "fill_id": fills[0]["fill_id"],
        "fill": fills[0],
    }


def create_option_rfq_fill(ctx: Context, option_series_id: str, suffix: str) -> Dict[str, Any]:
    rfq = http_json(
        ctx,
        "POST",
        "/options/rfqs",
        body={
            "taker": TAKER_ACCOUNT,
            "option_series_id": option_series_id,
            "side": "buy",
            "size_1e8": "100000000",
            "limit_price_1e8": "1000000000",
            "ttl_ms": 30000,
        },
    )
    option_rfq_id = rfq["option_rfq_id"]
    quote = http_json(
        ctx,
        "POST",
        f"/options/rfqs/{option_rfq_id}/quotes",
        body={
            "mm_account": MAKER_ACCOUNT,
            "session_id": None,
            "client_quote_id": f"e2e-{suffix}-quote",
            "price_1e8": "1000000000",
            "size_1e8": "100000000",
            "quote_nonce": None,
            "quote_ttl_ms": 10000,
            "signature": None,
        },
    )
    quote_id = quote["quote_id"]
    accepted = http_json(ctx, "POST", f"/options/rfqs/{option_rfq_id}/accept/{quote_id}")
    return {
        "option_rfq_id": option_rfq_id,
        "quote_id": quote_id,
        "option_fill_id": accepted["option_fill_id"],
        "fill": accepted["fill"],
    }


def fee_events_for_source(ctx: Context, source_type: str, source_id: str) -> List[Dict[str, str]]:
    return psql_rows(
        ctx,
        f"""
        SELECT fee_event_id, source_type, source_id, market_type, flow_type,
               COALESCE(market_id::text, '') AS market_id,
               COALESCE(option_series_id, '') AS option_series_id,
               COALESCE(maker, '') AS maker,
               COALESCE(taker, '') AS taker,
               payer, recipient, fee_asset, notional_1e8, fee_rate_micro_bps::text,
               fee_amount_1e8, rebate_rate_micro_bps::text, rebate_amount_1e8,
               protocol_amount_1e8, status, created_at_ms::text
        FROM fee_events
        WHERE source_type={sql_literal(source_type)} AND source_id={sql_literal(source_id)}
        ORDER BY fee_event_id
        """,
        [
            "fee_event_id",
            "source_type",
            "source_id",
            "market_type",
            "flow_type",
            "market_id",
            "option_series_id",
            "maker",
            "taker",
            "payer",
            "recipient",
            "fee_asset",
            "notional_1e8",
            "fee_rate_micro_bps",
            "fee_amount_1e8",
            "rebate_rate_micro_bps",
            "rebate_amount_1e8",
            "protocol_amount_1e8",
            "status",
            "created_at_ms",
        ],
    )


def volume_rows_since(ctx: Context, market_type: str, start_ms: int) -> List[Dict[str, str]]:
    return psql_rows(
        ctx,
        f"""
        SELECT bucket_id, account, bucket_day, market_type, maker_volume_1e8,
               taker_volume_1e8, total_volume_1e8, updated_at_ms::text
        FROM volume_buckets
        WHERE market_type={sql_literal(market_type)} AND updated_at_ms >= {start_ms}
        ORDER BY updated_at_ms DESC, bucket_id
        """,
        [
            "bucket_id",
            "account",
            "bucket_day",
            "market_type",
            "maker_volume_1e8",
            "taker_volume_1e8",
            "total_volume_1e8",
            "updated_at_ms",
        ],
    )


def run_admin(ctx: Context) -> FlowReport:
    report = FlowReport("admin")
    ctx.current_report = report

    step(report, ctx, "health", lambda: http_json(ctx, "GET", "/health"))
    step(report, ctx, "admin_status", lambda: admin_get(ctx, "/admin/status"))

    def config_check() -> Dict[str, Any]:
        config = admin_get(ctx, "/admin/config")
        encoded = json.dumps(config, sort_keys=True)
        forbidden_values = [
            ctx.admin_token,
            ctx.database_url,
            "postgres://",
        ]
        leaked = [value for value in forbidden_values if value and value in encoded]
        string_values = collect_string_values(config)
        raw_urls = [
            value
            for value in string_values
            if value.startswith("http://") or value.startswith("https://")
        ]
        require(not leaked, "admin config exposed a forbidden secret-like value")
        require(not raw_urls, "admin config exposed raw URL string values")
        return {
            "features": config.get("features", {}),
            "configured": config.get("configured", {}),
            "secret_values_absent": True,
        }

    step(report, ctx, "admin_config_sanitized", config_check)
    step(report, ctx, "admin_db", lambda: admin_get(ctx, "/admin/db"))
    step(report, ctx, "admin_fees_summary", lambda: admin_get(ctx, "/admin/fees/summary"))

    def wrong_token() -> Dict[str, Any]:
        response = http_json(
            ctx,
            "GET",
            "/admin/status",
            admin_token="wrong-e2e-token",
            expect_status=403,
        )
        return {"http_status": 403, "response": response}

    step(report, ctx, "wrong_token_rejected", wrong_token)
    return report


def collect_string_values(value: Any) -> List[str]:
    if isinstance(value, str):
        return [value]
    if isinstance(value, dict):
        strings: List[str] = []
        for item in value.values():
            strings.extend(collect_string_values(item))
        return strings
    if isinstance(value, list):
        strings = []
        for item in value:
            strings.extend(collect_string_values(item))
        return strings
    return []


def prepare_option_flow(ctx: Context, report: FlowReport, suffix: str) -> tuple[Dict[str, Any], Dict[str, Any]]:
    config = step(
        report,
        ctx,
        "options_fee_config",
        lambda: admin_get(ctx, "/admin/config"),
    )
    validate_config_for_options(config)
    validate_config_for_fees(config)
    series = step(report, ctx, "create_option_series", lambda: create_option_series(ctx, suffix))
    if config_mm_permissions_enabled(config):
        require(
            config.get("features", {}).get("persistence_enabled") is True,
            "MM permissions are enabled without persistence; the harness cannot seed in-memory permissions",
        )
        seed_details = step(
            report,
            ctx,
            "seed_mm_permissions",
            lambda: seed_mm_permissions(
                ctx,
                MAKER_ACCOUNT,
                can_quote_option_rfq=True,
                can_submit_option_orders=True,
                option_series_id=series["option_series_id"],
            ),
        )
    else:
        seed_details = {"needed": False, "reason": "MM_PERMISSIONS_ENABLED=false"}
        report.add_check("seed_mm_permissions", True, seed_details, skipped=True)
    return series, seed_details


def run_fees_options(ctx: Context) -> FlowReport:
    report = FlowReport("fees-options")
    ctx.current_report = report
    test_start_ms = now_ms()
    report.artifacts["test_start_ms"] = test_start_ms
    suffix = f"{ctx.run_id}-fees-options"
    series, _seed = prepare_option_flow(ctx, report, suffix)
    option_series_id = series["option_series_id"]
    report.artifacts["option_series_id"] = option_series_id

    order_fill = step(
        report,
        ctx,
        "create_option_orderbook_fill",
        lambda: create_option_order_fill(ctx, option_series_id, suffix),
    )
    order_fill_id = order_fill["fill_id"]
    report.artifacts["option_order_fill_id"] = order_fill_id

    def verify_order_fee_events() -> Dict[str, Any]:
        rows = fee_events_for_source(ctx, "option_order_fill", order_fill_id)
        require(len(rows) == 2, f"expected two option_order_fill fee events, got {len(rows)}")
        require(all(row["market_type"] == "option" for row in rows), "option order fee market_type mismatch")
        require(all(row["flow_type"] == "orderbook" for row in rows), "option order fee flow_type mismatch")
        require(all(row["status"] == "accrued" for row in rows), "option order fee status mismatch")
        return {"fee_event_ids": [row["fee_event_id"] for row in rows]}

    step(report, ctx, "verify_option_order_fee_events", verify_order_fee_events)

    rfq_fill = step(
        report,
        ctx,
        "create_option_rfq_fill",
        lambda: create_option_rfq_fill(ctx, option_series_id, suffix),
    )
    option_rfq_fill_id = rfq_fill["option_fill_id"]
    report.artifacts["option_rfq_id"] = rfq_fill["option_rfq_id"]
    report.artifacts["option_rfq_fill_id"] = option_rfq_fill_id

    def verify_rfq_fee_events() -> Dict[str, Any]:
        rows = fee_events_for_source(ctx, "option_rfq_fill", option_rfq_fill_id)
        require(len(rows) == 2, f"expected two option_rfq_fill fee events, got {len(rows)}")
        require(all(row["market_type"] == "option" for row in rows), "option RFQ fee market_type mismatch")
        require(all(row["flow_type"] == "rfq" for row in rows), "option RFQ fee flow_type mismatch")
        require(all(row["status"] == "accrued" for row in rows), "option RFQ fee status mismatch")
        return {"fee_event_ids": [row["fee_event_id"] for row in rows]}

    step(report, ctx, "verify_option_rfq_fee_events", verify_rfq_fee_events)

    def verify_option_volumes() -> Dict[str, Any]:
        rows = volume_rows_since(ctx, "option", test_start_ms)
        accounts = {row["account"].lower() for row in rows}
        require(MAKER_ACCOUNT.lower() in accounts, "maker option volume bucket missing")
        require(TAKER_ACCOUNT.lower() in accounts, "taker option volume bucket missing")
        return {"rows": rows}

    step(report, ctx, "verify_option_volume_buckets", verify_option_volumes)

    def verify_admin_fees() -> Dict[str, Any]:
        summary = admin_get(ctx, "/admin/fees/summary")
        events = admin_get(ctx, "/admin/fees/events?limit=20")
        volumes = admin_get(ctx, f"/admin/fees/volumes?account={urllib.parse.quote(MAKER_ACCOUNT)}")
        return {
            "summary_source_type_counts": summary.get("ledger", {}).get("source_type_counts", {}),
            "recent_event_count": len(events.get("events", [])),
            "maker_volume_count": len(volumes.get("volumes", [])),
        }

    step(report, ctx, "verify_admin_fee_endpoints", verify_admin_fees)

    def verify_no_execution_transactions() -> Dict[str, Any]:
        count = psql_scalar(
            ctx,
            f"SELECT COUNT(*)::text FROM execution_transactions WHERE created_at_ms >= {test_start_ms}",
        )
        require(count == "0", f"expected zero new execution_transactions, got {count}")
        return {"new_execution_transactions_since_test_start": int(count)}

    step(report, ctx, "verify_no_execution_transactions", verify_no_execution_transactions)
    return report


def run_option_rfq(ctx: Context) -> FlowReport:
    report = FlowReport("option-rfq")
    ctx.current_report = report
    test_start_ms = now_ms()
    report.artifacts["test_start_ms"] = test_start_ms
    suffix = f"{ctx.run_id}-option-rfq"
    series, _seed = prepare_option_flow(ctx, report, suffix)
    option_series_id = series["option_series_id"]
    report.artifacts["option_series_id"] = option_series_id

    rfq_fill = step(
        report,
        ctx,
        "create_submit_accept_option_rfq",
        lambda: create_option_rfq_fill(ctx, option_series_id, suffix),
    )
    report.artifacts.update(rfq_fill)

    def verify_persisted_fill() -> Dict[str, Any]:
        fill_id = rfq_fill["option_fill_id"]
        rows = psql_rows(
            ctx,
            f"""
            SELECT fill_id, option_rfq_id, quote_id, option_series_id, taker, mm_account,
                   taker_side, price_1e8, size_1e8
            FROM option_rfq_fills
            WHERE fill_id={sql_literal(fill_id)}
            """,
            [
                "fill_id",
                "option_rfq_id",
                "quote_id",
                "option_series_id",
                "taker",
                "mm_account",
                "taker_side",
                "price_1e8",
                "size_1e8",
            ],
        )
        require(len(rows) == 1, f"expected one option_rfq_fills row, got {len(rows)}")
        return rows[0]

    step(report, ctx, "verify_option_rfq_fill_row", verify_persisted_fill)

    def verify_no_execution_transactions() -> Dict[str, Any]:
        count = psql_scalar(
            ctx,
            f"SELECT COUNT(*)::text FROM execution_transactions WHERE created_at_ms >= {test_start_ms}",
        )
        require(count == "0", f"expected zero new execution_transactions, got {count}")
        return {"new_execution_transactions_since_test_start": int(count)}

    step(report, ctx, "verify_no_execution_transactions", verify_no_execution_transactions)
    return report


def find_perp_candidate(ctx: Context) -> Optional[Dict[str, str]]:
    rows = psql_rows(
        ctx,
        """
        SELECT et.intent_id,
               et.tx_hash,
               COALESCE(et.onchain_intent_id, '') AS onchain_intent_id,
               COALESCE(et.confirmation_status, '') AS confirmation_status,
               ipt.event_id AS indexed_event_id,
               ipt.market_id,
               ipt.buyer,
               ipt.seller,
               ipt.size_delta_1e8,
               ipt.execution_price_1e8,
               CASE WHEN ipt.buyer_is_maker THEN 'true' ELSE 'false' END AS buyer_is_maker,
               ipt.created_at_ms::text,
               CASE
                   WHEN EXISTS (
                       SELECT 1 FROM rfqs
                       WHERE rfqs.execution_intent_id = et.intent_id
                         AND rfqs.status = 'accepted'
                   ) THEN 'rfq'
                   ELSE 'orderbook'
               END AS flow_type
        FROM execution_transactions et
        JOIN execution_reconciliations er
          ON er.intent_id = et.intent_id
         AND er.tx_hash = et.tx_hash
         AND er.status = 'matched'
        JOIN indexed_perp_trades ipt
          ON ipt.event_id = er.indexed_event_id
         AND ipt.tx_hash = et.tx_hash
        JOIN execution_intents ei
          ON ei.intent_id = et.intent_id
        WHERE et.confirmation_status = 'confirmed'
          AND et.tx_hash IS NOT NULL
        ORDER BY et.confirmed_at_ms DESC NULLS LAST, et.updated_at_ms DESC
        LIMIT 1
        """,
        [
            "intent_id",
            "tx_hash",
            "onchain_intent_id",
            "confirmation_status",
            "indexed_event_id",
            "market_id",
            "buyer",
            "seller",
            "size_delta_1e8",
            "execution_price_1e8",
            "buyer_is_maker",
            "created_at_ms",
            "flow_type",
        ],
    )
    if not rows:
        return None
    return rows[0]


def run_fees_perps(ctx: Context) -> FlowReport:
    report = FlowReport("fees-perps")
    ctx.current_report = report
    test_start_ms = now_ms()
    report.artifacts["test_start_ms"] = test_start_ms

    candidate_result = step(
        report,
        ctx,
        "find_confirmed_indexed_reconciled_trade",
        lambda: find_candidate_or_skip(ctx),
    )
    if candidate_result.get("skipped"):
        return report
    candidate = candidate_result
    report.artifacts["candidate"] = candidate

    buyer_is_maker = candidate["buyer_is_maker"] == "true"
    maker = candidate["buyer"] if buyer_is_maker else candidate["seller"]
    taker = candidate["seller"] if buyer_is_maker else candidate["buyer"]
    report.artifacts["maker"] = maker
    report.artifacts["taker"] = taker
    report.artifacts["expected_notional_1e8"] = (
        int(candidate["execution_price_1e8"]) * int(candidate["size_delta_1e8"]) // 100_000_000
    )

    config = step(report, ctx, "perp_fee_config", lambda: admin_get(ctx, "/admin/config"))
    validate_config_for_fees(config)
    if not config.get("features", {}).get("confirmation_enabled"):
        report.add_check(
            "trigger_confirmed_intent",
            True,
            {"reason": "CONFIRMATION_ENABLED=false; existing candidate found but hook cannot be re-run"},
            skipped=True,
        )
        return report

    before_count = psql_scalar(
        ctx,
        f"""
        SELECT COUNT(*)::text
        FROM fee_events
        WHERE market_type='perp'
          AND source_type='perp_trade'
          AND source_id={sql_literal(candidate['indexed_event_id'])}
        """,
    )
    report.artifacts["pre_trigger_perp_fee_event_count"] = int(before_count or "0")

    def trigger_confirm() -> Dict[str, Any]:
        response = http_json(ctx, "POST", f"/executor/confirm/{candidate['intent_id']}")
        require(response.get("confirmed") is True, "confirmation response did not report confirmed=true")
        require(response.get("indexed_event_found") is True, "confirmation response did not find indexed event")
        require(response.get("reconciliation_matched") is True, "confirmation response did not match reconciliation")
        return response

    step(report, ctx, "trigger_confirmed_intent", trigger_confirm)

    def verify_perp_fee_events() -> Dict[str, Any]:
        rows = fee_events_for_source(ctx, "perp_trade", candidate["indexed_event_id"])
        require(len(rows) == 2, f"expected two perp fee events, got {len(rows)}")
        require(all(row["market_type"] == "perp" for row in rows), "perp fee market_type mismatch")
        require(all(row["flow_type"] == candidate["flow_type"] for row in rows), "perp fee flow_type mismatch")
        require(all(row["market_id"] == candidate["market_id"] for row in rows), "perp fee market_id mismatch")
        require(all(row["option_series_id"] == "" for row in rows), "perp fee option_series_id must be null")
        require(all(row["status"] == "accrued" for row in rows), "perp fee status mismatch")
        require(all(row["notional_1e8"] == str(report.artifacts["expected_notional_1e8"]) for row in rows), "perp fee notional mismatch")
        return {"fee_event_ids": [row["fee_event_id"] for row in rows], "rows": rows}

    step(report, ctx, "verify_perp_fee_events", verify_perp_fee_events)

    def verify_perp_volumes() -> Dict[str, Any]:
        rows = psql_rows(
            ctx,
            f"""
            SELECT bucket_id, account, bucket_day, market_type, maker_volume_1e8,
                   taker_volume_1e8, total_volume_1e8, updated_at_ms::text
            FROM volume_buckets
            WHERE market_type='perp'
              AND lower(account) IN (lower({sql_literal(maker)}), lower({sql_literal(taker)}))
            ORDER BY updated_at_ms DESC, bucket_id
            """,
            [
                "bucket_id",
                "account",
                "bucket_day",
                "market_type",
                "maker_volume_1e8",
                "taker_volume_1e8",
                "total_volume_1e8",
                "updated_at_ms",
            ],
        )
        require(any(row["account"].lower() == maker.lower() and int(row["maker_volume_1e8"]) > 0 for row in rows), "maker perp volume missing")
        require(any(row["account"].lower() == taker.lower() and int(row["taker_volume_1e8"]) > 0 for row in rows), "taker perp volume missing")
        return {"rows": rows}

    first_volume_snapshot = step(report, ctx, "verify_perp_volume_buckets", verify_perp_volumes)

    step(report, ctx, "trigger_confirmed_intent_idempotency", trigger_confirm)

    def verify_idempotency() -> Dict[str, Any]:
        duplicate_rows = psql_rows(
            ctx,
            """
            SELECT source_type, source_id, payer, recipient, COUNT(*)::text
            FROM fee_events
            WHERE market_type='perp'
            GROUP BY source_type, source_id, payer, recipient
            HAVING COUNT(*) > 1
            """,
            ["source_type", "source_id", "payer", "recipient", "count"],
        )
        require(not duplicate_rows, "duplicate perp fee events found")
        second_volume_snapshot = verify_perp_volumes()
        require(
            second_volume_snapshot == first_volume_snapshot,
            "perp volume buckets changed after idempotency trigger",
        )
        return {"duplicate_rows": duplicate_rows, "volume_unchanged": True}

    step(report, ctx, "verify_idempotency", verify_idempotency)

    def verify_admin_fees() -> Dict[str, Any]:
        summary = admin_get(ctx, "/admin/fees/summary")
        events = admin_get(ctx, "/admin/fees/events?limit=20")
        volumes = admin_get(ctx, f"/admin/fees/volumes?account={urllib.parse.quote(maker)}")
        return {
            "market_type_counts": summary.get("ledger", {}).get("market_type_counts", {}),
            "source_type_counts": summary.get("ledger", {}).get("source_type_counts", {}),
            "recent_event_count": len(events.get("events", [])),
            "maker_volume_count": len(volumes.get("volumes", [])),
        }

    step(report, ctx, "verify_admin_fee_endpoints", verify_admin_fees)

    def verify_no_execution_transactions() -> Dict[str, Any]:
        count = psql_scalar(
            ctx,
            f"SELECT COUNT(*)::text FROM execution_transactions WHERE created_at_ms >= {test_start_ms}",
        )
        require(count == "0", f"expected zero new execution_transactions, got {count}")
        status = http_json(ctx, "GET", "/executor/status")
        require(status.get("realBroadcastEnabled") is False, "realBroadcastEnabled must be false")
        require(status.get("broadcastEnabled") is False, "broadcastEnabled must be false")
        return {
            "new_execution_transactions_since_test_start": int(count),
            "executor_status": status,
        }

    step(report, ctx, "verify_no_forbidden_mutation", verify_no_execution_transactions)
    return report


def find_candidate_or_skip(ctx: Context) -> Dict[str, str]:
    candidate = find_perp_candidate(ctx)
    if not candidate:
        raise E2ESkip("no confirmed/indexed/reconciled perp trade candidate found")
    return candidate


def run_mm_auth(ctx: Context) -> FlowReport:
    report = FlowReport("mm-auth")
    ctx.current_report = report
    detail = {
        "reason": (
            "MM WebTransport auth is deferred in E2E Harness V1A. "
            "The existing mm_wt_smoke auth path requires live WebTransport cert/key setup "
            "and an MM_PRIVATE_KEY; this harness does not generate or expose private keys."
        ),
        "enable_webtransport_requested": bool(ctx.args.enable_webtransport),
    }
    report.add_check("mm_auth_placeholder", True, detail, skipped=True)
    report.artifacts["deferred"] = "wrap cargo run --bin mm_wt_smoke -- auth in a future opt-in flow"
    return report


def run_all_safe(ctx: Context) -> FlowReport:
    report = FlowReport("all-safe")
    ctx.current_report = report
    child_flows = ["admin", "fees-options", "fees-perps", "option-rfq"]
    child_reports: Dict[str, Any] = {}
    for flow in child_flows:
        child = run_named_flow(ctx, flow)
        child_report = child.finish()
        child_reports[flow] = child_report
        report.add_check(
            f"flow_{flow}",
            child_report["ok"],
            {
                "checks": len(child_report["checks"]),
                "skipped": sum(1 for check in child_report["checks"] if check.get("skipped")),
            },
            error="; ".join(child_report["errors"]) if child_report["errors"] else None,
        )
    report.artifacts["reports"] = child_reports
    return report


def run_named_flow(ctx: Context, flow: str) -> FlowReport:
    ctx.current_report = None
    try:
        if flow == "admin":
            return run_admin(ctx)
        if flow == "fees-options":
            return run_fees_options(ctx)
        if flow == "fees-perps":
            return run_fees_perps(ctx)
        if flow == "option-rfq":
            return run_option_rfq(ctx)
        if flow == "mm-auth":
            return run_mm_auth(ctx)
        if flow == "all-safe":
            return run_all_safe(ctx)
        raise E2EError(f"unknown flow: {flow}")
    except Exception as exc:  # noqa: BLE001 - return partial report with failure details.
        report = ctx.current_report or FlowReport(flow)
        if not report.checks or report.checks[-1].get("ok") is not False:
            report.add_check("flow_error", False, error=sanitize_text(one_line(exc), ctx))
        return report
    finally:
        ctx.current_report = None


def print_human_summary(report: Dict[str, Any], *, verbose: bool) -> None:
    status = "OK" if report["ok"] else "FAIL"
    duration_ms = report["finished_at_ms"] - report["started_at_ms"]
    skipped = sum(1 for check in report["checks"] if check.get("skipped"))
    print(
        f"E2E {report['flow']}: {status} "
        f"({len(report['checks'])} checks, {skipped} skipped, {duration_ms} ms)",
        file=sys.stderr,
    )
    for check in report["checks"]:
        if check.get("skipped"):
            marker = "SKIP"
        else:
            marker = "OK" if check["ok"] else "FAIL"
        line = f"  {marker} {check['name']}"
        if not check["ok"] and check.get("error"):
            line += f": {check['error']}"
        elif verbose and check.get("details"):
            line += f": {one_line(json.dumps(check['details'], sort_keys=True), 300)}"
        print(line, file=sys.stderr)
    if report.get("artifacts", {}).get("backend_log_path"):
        print(f"  backend log: {report['artifacts']['backend_log_path']}", file=sys.stderr)


def write_json_report(report: Dict[str, Any], path: Optional[str]) -> None:
    if path:
        with open(path, "w", encoding="utf-8") as handle:
            json.dump(report, handle, indent=2, sort_keys=True)
            handle.write("\n")
    json.dump(report, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")


def parse_args(argv: Optional[Sequence[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run safe local/runtime E2E verification flows for deopt-v2-backend.",
    )
    parser.add_argument("--flow", choices=FLOW_CHOICES, required=True, help="E2E flow to run.")
    parser.add_argument(
        "--backend-url",
        default=DEFAULT_BACKEND_URL,
        help=f"Backend base URL. Default: {DEFAULT_BACKEND_URL}",
    )
    parser.add_argument(
        "--database-url",
        default=DEFAULT_DATABASE_URL,
        help="PostgreSQL URL used by psql checks and --start-backend.",
    )
    parser.add_argument(
        "--admin-token",
        default=DEFAULT_ADMIN_TOKEN,
        help="Admin token sent as X-Admin-Token. The token is redacted from errors.",
    )
    start_group = parser.add_mutually_exclusive_group()
    start_group.add_argument(
        "--start-backend",
        action="store_true",
        help="Start cargo run --bin deopt-v2-backend with safe process env and stop it at the end.",
    )
    start_group.add_argument(
        "--no-start-backend",
        action="store_true",
        help="Use an already-running backend. This is the default.",
    )
    parser.add_argument(
        "--timeout-sec",
        type=int,
        default=120,
        help="Timeout for backend startup and subprocess checks. Default: 120.",
    )
    parser.add_argument(
        "--json-out",
        help="Optional path to also write the JSON report.",
    )
    parser.add_argument(
        "--cleanup",
        action="store_true",
        help=(
            "Restore temporary harness scaffolding when safe. V1A always restores temporary "
            "MM permission seeds; ledger evidence is retained."
        ),
    )
    parser.add_argument("--verbose", action="store_true", help="Print check details in the human summary.")
    parser.add_argument(
        "--enable-webtransport",
        action="store_true",
        help="Reserved for the deferred mm-auth WebTransport flow; not used by safe flows.",
    )
    return parser.parse_args(argv)


def main(argv: Optional[Sequence[str]] = None) -> int:
    args = parse_args(argv)
    ctx = Context(
        args=args,
        backend_url=backend_origin(args.backend_url),
        database_url=args.database_url,
        admin_token=args.admin_token,
        repo_root=os.getcwd(),
    )
    report = FlowReport(args.flow)
    try:
        if args.start_backend:
            step(report, ctx, "start_backend", lambda: (start_backend(ctx, report) or {"started": True}))
        flow_report = run_named_flow(ctx, args.flow)
        report.checks.extend(flow_report.checks)
        report.artifacts.update(flow_report.artifacts)
        report.errors.extend(flow_report.errors)
        report.ok = report.ok and flow_report.ok
    except Exception as exc:  # noqa: BLE001 - top-level report must capture unexpected failures.
        message = sanitize_text(one_line(exc), ctx)
        report.add_check("flow_error", False, error=message)
    finally:
        run_cleanups(ctx, report)
        if args.start_backend:
            stop_backend(ctx)
            report.add_check("stop_backend", True, {"stopped": True})

    output = report.finish()
    print_human_summary(output, verbose=args.verbose)
    write_json_report(output, args.json_out)
    return 0 if output["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
