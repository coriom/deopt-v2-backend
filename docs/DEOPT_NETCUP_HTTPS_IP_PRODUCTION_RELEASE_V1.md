# DeOpt Netcup HTTPS IP production release V1 — gated evidence

Date: 2026-11-03 UTC. This is a staged release record, not evidence that the public API or wallet authentication is live.

## Current production boundary

- Coolify resource `dxco90jwpdxyennmquv2elyj` was last verified running the private read-only image from source `e2f139fc`. PostgreSQL 16 is private; the new wallet-auth migration `0072_wallet_read_auth.sql` has not been reported applied in production.
- The backend wallet-auth source at `aa84d42404ba381725c6bab4839ca0969bf21d5f` and frontend same-origin gateway at `a7b165a97d168726f5bbb9e2297425c6be2b90e9` are local, unpushed release candidates. The gateway feature flag defaults off.
- Root read-only preflight on the reviewed VPS: Traefik `3.7.17`, file provider watching `/traefik/dynamic/`, resolver named `letsencrypt`, only `default_redirect_503.yaml` in the dynamic directory, no default TLS configuration. Proxy Compose SHA-256 `c9e17d1716197d8124b7ae0f686335f6ddb9574fcc324280768020b504986833`. This does not establish an IP certificate or ACME eligibility.
- A separate SSH read-only `openssl s_client -noservername` check returned `CN=TRAEFIK DEFAULT CERT`; its SAN is a generated `traefik.default` DNS name, not the VPS IP. A browser or Vercel function must reject this certificate for `https://94.16.113.122`.
- The proxy still publishes 8080 on IPv4 and IPv6. The saved Compose removes its mapping, but the running proxy has not been restarted. Port 80/443 and the private Coolify tunnel must be checked after a controlled proxy change.
- Backup timer and watchdog were active at the last SSH check. First automatic backup is scheduled for 2026-11-04 02:07 UTC; its success is not yet proven. A previous manual production dump, R2 upload and isolated restore were accepted separately.

## Prepared local changes

- `deploy/vps/ip_tls_readonly_preflight.py` emits bounded proxy evidence, with no environment, ACME storage or key material. It has been staged unprivileged on the VPS and its SHA-256 was checked against the root-owned copy before the operator ran it.
- `compose.coolify-readonly.yml` now supports `DEOPT_WALLET_READ_AUTH_ENABLED`, default `false`; the signed-message origin is pinned to `https://deoptv2-frontend.vercel.app`. The private Compose validator accepts `true` only for this one read-only feature and rejects another origin or another enabled economic flag. Current deployed Compose is unchanged.
- `deploy/vps/ghcr-private-first-publish.sh --publish-private-update` supports an already private GHCR package, demands interactive PAT input, verifies the package remains private and the commit tag is unused, and uses a disposable Docker credentials directory. Its invocation requires the reviewed commit at `origin/main`; it has not published anything.
- Fresh bounded checks: 15/15 Compose validator tests; 13/13 GHCR/preflight/proxy tests; 2/2 wallet-auth Rust unit tests; 2/2 read-only Rust unit tests; `cargo build --locked --offline` PASS; frontend TypeScript, lint and Next.js production build pass (one known lint warning). The disposable PostgreSQL wallet session and migration test passed 1/1 against local PostgreSQL 16 after rerunning with permission for the local loopback socket. The first sandbox attempt failed with EPERM and is not an application regression. The disposable container was stopped and removed. A focused Chromium rerun was stopped under host load; prior browser fixture evidence remains historical.
- At this checkpoint, remote `origin/main` is `4b9faac20eebebc0f1d3054429c23fe46dded3ee` for backend and `a32c5cc1b6dee30a0676b4adc156718687a0a727` for frontend. Local release commits are not pushed.

## Ordered activation gates

1. Inspect the non-secret ACME configuration and Coolify saved proxy configuration. Traefik can be tested with an isolated Let's Encrypt staging resolver using `shortlived`, `disableCommonName` and a six-day renewal interval. The existing resolver and other app routes must remain intact. An IP certificate must be presented as the default for clients without DNS SNI; test staging first and production only after a passing challenge, SAN and proxy postflight. Do not serve a staging certificate as production. Sources: [Let's Encrypt IP/shortlived](https://letsencrypt.org/2026/03/11/shorter-certs-certbot), [Traefik ACME](https://doc.traefik.io/traefik/v3.5/reference/install-configuration/tls/certificate-resolvers/acme/), [Traefik default certificate](https://doc.traefik.io/traefik/v3.4/https/tls/).
2. Verify trusted `https://94.16.113.122` externally with IP SAN and no SNI, then add an explicit Traefik allowlist for read-only/auth routes and edge rate limits. Recheck Coolify admin privacy, PostgreSQL isolation and other application routes. The private Compose validator deliberately rejects a proxy route; public exposure requires a separately reviewed configuration and postflight.
3. Obtain acceptance to push the exact backend and frontend commits. Publish a new private GHCR digest from the accepted backend commit with an authenticated operator PAT. Keep the old digest available for rollback. No GitHub Actions package grant from the public repository.
4. Confirm a fresh encrypted pre-migration production backup. Validate migration 0072 against a disposable PostgreSQL 16 database, then deploy the new digest through the existing Coolify resource, preserving the production volume. Enable `WALLET_READ_AUTH_ENABLED` only after the new image and migration are both verified. Do not downgrade the schema on image rollback.
5. Configure Vercel's server-only `DEOPT_BACKEND_HTTPS_ORIGIN=https://94.16.113.122` and then the client wallet-read feature gate. Verify deployed same-origin `/api/deopt` responses, valid signature, nonce replay, expiry, logout, wallet isolation, cookie flags, no-store and My Fees. Rollback order is frontend feature flag/origin, then backend digest; leave migration 0072 in place.

## Remaining prerequisites

The root-only filtered preflight confirmed the existing `letsencrypt` resolver uses HTTP-01 on `http`, the default production CA and persisted ACME storage. Its `profile`, `certificatesDuration` and `disableCommonName` settings are unset. Preserve that resolver for existing applications. No staging or production IP certificate has been issued. No external IP TLS, migration 0072, new GHCR digest, hosted wallet session or Vercel integration has passed. The automatic R2 timer run is not due yet. Public trading, indexer writes and broadcaster remain disabled.

The filtered ACME preflight was extended and staged at `/home/deoptops/deopt-release.stage/ip_tls_readonly_preflight.py`, SHA-256 `c3e86d621601085e8e735148f0113d9716ca24317eb33616af73e0f9d1dcfc99`. It prints only ACME challenge/profile/duration/CA-kind/storage flags. Root execution of this updated copy awaits the operator; no proxy change is authorized by the script.

## Candidate Traefik IP ACME sequence (not applied)

The running proxy is Traefik 3.7.17. Preserve the current `letsencrypt` resolver and add two independent resolvers in the Coolify-managed proxy configuration: `deoptipstaging` with Let's Encrypt staging CA, and `deoptipproduction` with the default production CA. Each needs its own persisted ACME storage file, HTTP-01 on the existing `http` entrypoint, `profile=shortlived`, `disableCommonName=true` and `certificatesDuration=160`. Adding both at once requires one controlled proxy restart and complete 80/443/other-app/tunnel postflight. Restore the exact prior proxy configuration if the postflight fails.

The exact additional Traefik command flags to review in **Coolify > Servers > localhost > Proxy > Configuration** are:

The same candidate is saved in `deploy/vps/traefik-ip-acme-flags-v1.txt`; the staging TLS store is in `deploy/vps/traefik-ip-staging-dynamic-v1.yaml`. Both are review-only and have 2 focused guard tests plus a YAML parse check.

```text
--certificatesresolvers.deoptipstaging.acme.httpchallenge.entrypoint=http
--certificatesresolvers.deoptipstaging.acme.caserver=https://acme-staging-v02.api.letsencrypt.org/directory
--certificatesresolvers.deoptipstaging.acme.storage=/traefik/acme-ip-staging.json
--certificatesresolvers.deoptipstaging.acme.profile=shortlived
--certificatesresolvers.deoptipstaging.acme.disablecommonname=true
--certificatesresolvers.deoptipstaging.acme.certificatesduration=160
--certificatesresolvers.deoptipproduction.acme.httpchallenge.entrypoint=http
--certificatesresolvers.deoptipproduction.acme.storage=/traefik/acme-ip-production.json
--certificatesresolvers.deoptipproduction.acme.profile=shortlived
--certificatesresolvers.deoptipproduction.acme.disablecommonname=true
--certificatesresolvers.deoptipproduction.acme.certificatesduration=160
```

These are candidates, not applied or proven by staging. The two storage files need owner access for Traefik and mode 0600; never print their contents. Retain the existing resolver, all 80/443 bindings and existing routes. Coolify's [dynamic configuration guidance](https://coolify.io/docs/core/networking/proxy/traefik/dynamic-config) says static proxy changes must be saved in its UI and require a proxy restart; disk-only edits can be overwritten. The currently saved Compose SHA-256 `c9e17d1716197d8124b7ae0f686335f6ddb9574fcc324280768020b504986833` already omits the 8080 binding, so a reviewed restart can apply that removal at the same time. Check that the saved UI still matches this exact Compose before changing it.

Only then add a watched dynamic TLS store file with `defaultGeneratedCert` for `94.16.113.122`, pointing first to the staging resolver. Confirm HTTP-01 succeeded and the no-SNI certificate has an IP SAN. Change the dynamic resolver to production only after staging passes; confirm a publicly trusted no-SNI certificate and automated-renewal settings. The default store is global, so verify existing DNS-SNI applications before and after both dynamic changes. Configure a separate expiry alert before depending on a 160-hour certificate. An actual renewal remains unverified until observed; a staging issue proves only initial issuance.

Staging dynamic file candidate:

```yaml
tls:
  stores:
    default:
      defaultGeneratedCert:
        resolver: deoptipstaging
        domain:
          main: "94.16.113.122"
```

After staging passes, change only `resolver` to `deoptipproduction`. Do not put these files on the host until the static resolvers are saved and verified; preserve the prior dynamic configuration for rollback.

Never route DeOpt's backend through this change alone. Add its restricted API router only after the trusted certificate, application release and database migration are independently ready.

## Coolify authenticated UI rehearsal, 2026-11-03

The operator completed personal login and 2FA in a separate Chromium profile over the existing private SSH tunnel. No session cookie, password or 2FA code was read or retained. The proxy configuration shown by Coolify matched the host SHA-256 `c9e17d1716197d8124b7ae0f686335f6ddb9574fcc324280768020b504986833` exactly. A local mode-0600 rollback copy was made. The candidate SHA-256 was `5b3aa11b9ced228e569a7927789b0b88bca16e09d053e495ca1a8dcfc60c5f7d`. `docker compose config --format json` showed the only rendered difference was the 11 candidate ACME command flags. Ports 80/443 and all other proxy settings were unchanged; 8080 was absent from the saved Compose but present in the running proxy.

The candidate was saved and read back in Coolify, but the proxy restart was **not** confirmed. Automatic approval review rejected the restart because it would briefly interrupt all services behind the shared proxy and an explicit separate approval was not present. The confirmation dialog was cancelled. To avoid a latent change on a future restart, the saved Coolify configuration was restored and read back at the original SHA-256 `c9e17d1716197d8124b7ae0f686335f6ddb9574fcc324280768020b504986833`. A final SSH check still showed 80/443/8080 listening on IPv4 and IPv6 and the private Coolify loopback returned HTTP 200. No staging certificate was requested, no dynamic TLS file was installed, and no service was restarted.

An explicit, separate operator approval for **one shared Coolify proxy restart** is required before applying the reviewed static candidate again. The approval should acknowledge a short interruption to every proxied application and the rollback path: restore the saved original configuration in Coolify, save, restart, then verify 80/443 and the private 8000 tunnel. Only after the static proxy postflight may the staging dynamic file be added and Let's Encrypt staging issuance tested. The local candidate artifacts remain review-only until then.
