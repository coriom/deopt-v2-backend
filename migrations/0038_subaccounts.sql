-- SUBACCOUNTS-CORE-BACKEND-V1 — real Derive-like subaccounts.
--
-- A subaccount is an internal trading identity that belongs to a
-- connected wallet. Every wallet owns one or more subaccounts; the
-- default `Account 1` is lazily created on the first authenticated
-- interaction (see `crate::subaccounts::ensure_default_subaccount`).
--
-- This migration lands the identity table and its indexes only. Broad
-- migration of Options / RFQ / TWAP / Perps / fees / write-auth-nonce
-- tables to include `subaccount_id` is deferred to the follow-up
-- milestones documented in
-- `docs/SUBACCOUNTS_READINESS_AUDIT_AND_SCOPE_V1_RESULT.md`.
--
-- Off-chain only. Nothing here touches Solidity, mainnet, or the Perps
-- fail-closed posture.

CREATE TABLE IF NOT EXISTS subaccounts (
    owner_address    TEXT     NOT NULL,
    subaccount_id    INTEGER  NOT NULL,
    -- Human-friendly name. NULL means the frontend displays the
    -- default "Account N" derived from `subaccount_id`.
    name             TEXT,
    created_at_ms    BIGINT   NOT NULL,
    updated_at_ms    BIGINT   NOT NULL,
    -- Reserved for a future archive endpoint. NULL = active. V1 has
    -- no route that sets this column; the schema carries it so the
    -- follow-up milestone can land without another migration.
    archived_at_ms   BIGINT,
    PRIMARY KEY (LOWER(owner_address), subaccount_id),
    -- `0` is reserved for a future system-owned sweep/vault subaccount;
    -- user subaccounts start at 1 and are dense integers per wallet.
    CONSTRAINT subaccounts_subaccount_id_positive CHECK (subaccount_id >= 1),
    CONSTRAINT subaccounts_name_len CHECK (
        name IS NULL OR (char_length(name) >= 1 AND char_length(name) <= 64)
    )
);

CREATE INDEX IF NOT EXISTS subaccounts_by_owner
    ON subaccounts (LOWER(owner_address));
