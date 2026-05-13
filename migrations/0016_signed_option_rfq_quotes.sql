ALTER TABLE option_rfq_quotes
    ADD COLUMN IF NOT EXISTS signature TEXT NULL,
    ADD COLUMN IF NOT EXISTS quote_digest TEXT NULL,
    ADD COLUMN IF NOT EXISTS quote_nonce TEXT NULL,
    ADD COLUMN IF NOT EXISTS signature_status TEXT NULL,
    ADD COLUMN IF NOT EXISTS recovered_signer TEXT NULL;

CREATE INDEX IF NOT EXISTS idx_option_rfq_quotes_digest
    ON option_rfq_quotes(quote_digest);

CREATE INDEX IF NOT EXISTS idx_option_rfq_quotes_signature_status
    ON option_rfq_quotes(signature_status);
