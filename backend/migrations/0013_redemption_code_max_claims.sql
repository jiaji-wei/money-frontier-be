ALTER TABLE redemption_codes
    ADD COLUMN max_claims INTEGER NOT NULL DEFAULT 1;
