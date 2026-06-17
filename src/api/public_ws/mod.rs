//! Public WebSocket API for DeOpt V2.
//!
//! Endpoint: `GET /ws` (configurable via `PUBLIC_WS_PATH`).
//!
//! Surface (read-only, browser-friendly):
//!   * JSON-RPC 2.0-shaped client → server requests
//!     (`ping`, `subscribe`, `unsubscribe`, `subscriptions`,
//!     `auth.challenge`, `auth.verify`, `session.get`).
//!   * Server → client `subscription` push frames with a monotonic
//!     `seq` per channel and an `event_id`.
//!   * Public channels: `trading.health`, `options.products`,
//!     `leaderboard`. Account-scoped channels (`account.*`) require
//!     wallet authentication via the EIP-191 challenge/verify flow
//!     (BACKEND-PUBLIC-WS-AUTH-V1): `account.positions`,
//!     `account.portfolio`, `account.balances`, `account.history`
//!     are wired to the existing HTTP handlers; the remaining
//!     `account.{orders,fills,intent_status,settlements,liquidations}`
//!     return honest empty arrays until the underlying source data
//!     lands.
//!
//! Posture: public reads only. NO order submission, NO broadcast, NO
//! mainnet, NO server-side signing. The MM Gateway over WebTransport
//! remains a separate operator-whitelisted surface and is NOT
//! exposed here.

pub mod config;
pub mod dispatcher;
pub mod handler;
pub mod protocol;
pub mod session;
pub mod snapshots;

pub use config::PublicWsConfig;
pub use handler::public_ws_route;
