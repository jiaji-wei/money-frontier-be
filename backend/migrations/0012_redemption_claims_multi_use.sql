CREATE TABLE IF NOT EXISTS redemption_claims_new (
    id TEXT PRIMARY KEY,
    code_id INTEGER NOT NULL,
    claimant_type TEXT NOT NULL CHECK(claimant_type IN ('wallet', 'email')),
    claimant TEXT NOT NULL,
    ticket_id TEXT NOT NULL UNIQUE,
    order_row_id INTEGER NOT NULL UNIQUE,
    status TEXT NOT NULL CHECK(status IN ('claimed')),
    claimed_at INTEGER NOT NULL,
    FOREIGN KEY(code_id) REFERENCES redemption_codes(id),
    FOREIGN KEY(ticket_id) REFERENCES tickets(id),
    FOREIGN KEY(order_row_id) REFERENCES orders(id)
);

INSERT OR IGNORE INTO redemption_claims_new (
    id,
    code_id,
    claimant_type,
    claimant,
    ticket_id,
    order_row_id,
    status,
    claimed_at
)
SELECT
    id,
    code_id,
    claimant_type,
    claimant,
    ticket_id,
    order_row_id,
    status,
    claimed_at
FROM redemption_claims;

DROP TABLE redemption_claims;

ALTER TABLE redemption_claims_new RENAME TO redemption_claims;

CREATE INDEX IF NOT EXISTS idx_redemption_claims_code_id
    ON redemption_claims(code_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_redemption_claims_claimant_code
    ON redemption_claims(code_id, claimant_type, claimant);

CREATE INDEX IF NOT EXISTS idx_redemption_claims_claimant
    ON redemption_claims(claimant_type, claimant);

CREATE INDEX IF NOT EXISTS idx_redemption_claims_claimed_at
    ON redemption_claims(claimed_at);
