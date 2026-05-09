ALTER TABLE rfq_quotes
    ADD COLUMN IF NOT EXISTS signature TEXT NULL,
    ADD COLUMN IF NOT EXISTS quote_digest TEXT NULL,
    ADD COLUMN IF NOT EXISTS quote_nonce TEXT NULL,
    ADD COLUMN IF NOT EXISTS signature_status TEXT NULL,
    ADD COLUMN IF NOT EXISTS recovered_signer TEXT NULL;

CREATE INDEX IF NOT EXISTS idx_rfq_quotes_quote_digest
    ON rfq_quotes(quote_digest);

CREATE INDEX IF NOT EXISTS idx_rfq_quotes_signature_status
    ON rfq_quotes(signature_status);

