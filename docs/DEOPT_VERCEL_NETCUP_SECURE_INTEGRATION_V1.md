# DeOpt Vercel / Netcup secure API integration V1

## Implemented locally

The Rust read-only profile now has an opt-in wallet read session. It generates an EIP-4361-style message for `https://deoptv2-frontend.vercel.app`, Base Sepolia `84532`, a random one-use nonce and a five-minute expiry. Ethereum personal-sign recovery must equal the requested wallet. PostgreSQL atomically consumes the nonce and creates a 15-minute revocable session. Only token hashes are stored. The canonical fee-history route checks the session wallet against the requested wallet. Smart-contract wallets using ERC-1271 are not supported by this verifier and cannot sign in through it.

The frontend has same-origin Next.js API handlers under `/api/deopt`. The server-only `DEOPT_BACKEND_HTTPS_ORIGIN` must be a clean HTTPS origin with a valid certificate. The browser never receives the upstream origin or bearer token. The verified token is held in an HttpOnly, Secure, SameSite=Strict cookie scoped to `/api/deopt`. The public proxy handler has an explicit read-only allowlist; wallet fees require that cookie. `My Fees` asks the connected wallet to sign explicitly. Disconnected and wrong-chain states remain unavailable. The frontend feature gate `NEXT_PUBLIC_DEOPT_WALLET_READ_AUTH` defaults off.

The backend feature gate `WALLET_READ_AUTH_ENABLED` defaults off and requires read-only mode, PostgreSQL, chain 84532, and `WALLET_READ_AUTH_ORIGIN=https://deoptv2-frontend.vercel.app`. The deployed image from commit `e2f139fc` does not contain this implementation. Migration `0072_wallet_read_auth.sql` must be applied before enabling it. The live wallet route remains blocked in the current read-only release.

## Activation and rollback gates

1. Establish a stable HTTPS origin for Netcup with publicly trusted TLS and automated renewal, then verify it from Vercel. A same-origin Vercel function still needs a secure upstream origin. No production API origin has been validated. Do not use public HTTP, a self-signed certificate or TLS verification bypass.
   [Let's Encrypt now issues IP-address certificates](https://letsencrypt.org/2026/01/15/6day-and-ip-general-availability), but they are short-lived. [Coolify supports custom certificates](https://coolify.io/docs/core/networking/proxy/traefik/custom-ssl-certs); a safe ACME challenge path, renewal and Traefik routing for this IP have not been validated on this host. Coolify's generated `sslip.io` address remains HTTP-only by its [own documentation](https://coolify.io/docs/core/networking/domains). Treat IP TLS as an engineering option, not as an existing secure origin.
2. Build, test and publish a new private amd64 backend image by immutable digest. Review its source commit, migration 0072, signing key absence, and read-only flags. Use a separate production backup before migration. Preserve PostgreSQL volume and never automatically downgrade the schema.
3. On Coolify, permit only the API paths used by `/api/deopt`: health, readiness, bounded market reads, wallet auth challenge/verify/session/logout and canonical wallet fees. Keep database and admin ports private. Keep all economic execution flags false. Pin the image digest. Apply a bounded request/body rate limit at the public edge in addition to the database auth limits.
4. Configure `DEOPT_BACKEND_HTTPS_ORIGIN` (server only) and `NEXT_PUBLIC_DEOPT_WALLET_READ_AUTH=true` in Vercel after the secure upstream is verified. Test valid and invalid signatures, replay, expiry, logout, wallet isolation and real `My Fees` from the deployed frontend before accepting production integration.
5. Roll back the frontend feature flag and origin first if the upstream fails. Roll back the backend image by digest without reverting migration 0072 or destroying the database. Keep history access closed while authentication is unavailable.

## Current independent evidence and open gates

- Fresh local checks: `cargo build --locked --offline` PASS; wallet-auth unit tests 2/2; read-only guard unit tests 2/2; disposable PostgreSQL HTTP/session test 1/1 (including a concurrent nonce race); frontend `npm run typecheck`, `npm run lint` and `npm run build` PASS; Chromium wallet-auth fixtures 2/2 and existing My Fees regression fixtures 2/2; local Next.js route probes returned 401 for anonymous fees, 403 for unapproved public paths, 503 for absent secure upstream, and 403 for cross-origin auth POST. Browser fixtures do not prove a deployed HTTPS origin.
- Repository-wide `cargo fmt --check` remains FAIL across 44 files; touched new Rust files pass focused `rustfmt --check`. Strict `cargo clippy --lib -- -D warnings` remains FAIL with 197 diagnostics, none in the new wallet-auth module. The frontend lint has one pre-existing warning in `rfq-strategy-foundation-v1.spec.ts`.
- The existing private VPS backend was previously checked as read-only: health 200, ready 200, absent market 503, anonymous fee history 403 and write 405. No new backend image was deployed in this milestone.
- Coolify admin remains on a private tunnel. Proxy port 8080 is still bound on IPv4 and IPv6; its persistent removal needs a separately verified proxy change and rollback that preserves 80/443 and existing applications.
- The daily backup timer and watchdog are active, with first scheduled automatic run on 2026-11-04 at about 02:07 UTC. Its successful automatic R2 object has not yet been observed. The prior real database R2 backup and isolated restore passed; that does not prove the timer run.
- No independent public HTTPS origin, production SIWE session, production Vercel/My Fees transaction, or post-update PostgreSQL persistence test has been observed. These remain blocked rather than inferred from local tests.

## Narrowest next action

Choose and validate a trusted HTTPS origin for the VPS (a controlled DNS name or an IP certificate with an automated short-lived renewal path compatible with the existing proxy). Until that gate passes, retain the production read-only backend and the Vercel frontend unchanged.
