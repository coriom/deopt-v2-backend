#!/usr/bin/env python3
"""External, read-only DeOpt IP TLS and Vercel gateway health check."""

import argparse
import hashlib
import http.client
import json
import socket
import ssl
import sys
import time

IP = "94.16.113.122"
VERCEL = "deoptv2-frontend.vercel.app"
MIN_SECONDS = 48 * 60 * 60
TIMEOUT = 8


def certificate_status(cert, now):
    ip_sans = [value for kind, value in cert.get("subjectAltName", ()) if kind == "IP Address"]
    if IP not in ip_sans:
        raise ValueError("IP_SAN_MISSING")
    expiry = ssl.cert_time_to_seconds(cert["notAfter"])
    if expiry <= now:
        raise ValueError("CERT_EXPIRED")
    if expiry - now < MIN_SECONDS:
        raise ValueError("CERT_UNDER_48H")
    return expiry


def check_certificate(now):
    context = ssl.create_default_context()
    with socket.create_connection((IP, 443), timeout=TIMEOUT) as raw:
        with context.wrap_socket(raw, server_hostname=IP) as conn:
            cert = conn.getpeercert()
            fingerprint = hashlib.sha256(conn.getpeercert(binary_form=True)).hexdigest()
            expiry = certificate_status(cert, now)
    print(f"IP_TLS=PASS EXPIRY_UTC={time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime(expiry))} FINGERPRINT_SHA256={fingerprint}")


def health_status(status, body):
    if status != 200:
        raise ValueError(f"HEALTH_HTTP_{status}")
    try:
        payload = json.loads(body)
    except (UnicodeError, ValueError) as exc:
        raise ValueError("HEALTH_JSON_INVALID") from exc
    if not isinstance(payload, dict) or payload.get("ok") is not True or payload.get("service") != "deopt-v2-backend":
        raise ValueError("HEALTH_BODY_INVALID")


def check_health(host, path, label):
    connection = http.client.HTTPSConnection(host, timeout=TIMEOUT, context=ssl.create_default_context())
    try:
        connection.request("GET", path, headers={"Accept": "application/json"})
        response = connection.getresponse()
        body = response.read(1025)
        if len(body) > 1024:
            raise ValueError("HEALTH_BODY_OVERSIZE")
        health_status(response.status, body)
        print(f"{label}=PASS HTTP=200")
    finally:
        connection.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--simulate-alert", action="store_true", help="Fail only for notification delivery drill")
    parser.add_argument("--simulate-expiring-certificate", action="store_true", help="Exercise the 48-hour expiry policy without changing the live certificate")
    args = parser.parse_args()
    try:
        check_certificate(time.time())
        check_health(IP, "/health", "NETCUP_HEALTH")
        check_health(VERCEL, "/api/deopt/public/health", "VERCEL_GATEWAY")
        if args.simulate_expiring_certificate:
            synthetic_cert = {
                "subjectAltName": (("IP Address", IP),),
                "notAfter": time.strftime("%b %d %H:%M:%S %Y GMT", time.gmtime(time.time() + 3600)),
            }
            certificate_status(synthetic_cert, time.time())
        if args.simulate_alert:
            raise ValueError("SYNTHETIC_ALERT_TEST")
    except (OSError, ssl.SSLError, ValueError, http.client.HTTPException) as exc:
        reason = exc.args[0] if isinstance(exc, ValueError) and exc.args else type(exc).__name__
        print(f"::error::DEOPT_HTTPS_MONITOR_FAIL={reason}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
