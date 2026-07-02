CREATE TABLE IF NOT EXISTS redemption_codes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code_normalized TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL CHECK(status IN ('active', 'paused', 'redeemed', 'expired')),
    ticket_level INTEGER NOT NULL,
    valid_from INTEGER,
    valid_until INTEGER,
    notes TEXT,
    created_by TEXT NOT NULL,
    updated_by TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_redemption_codes_status
    ON redemption_codes(status);

CREATE INDEX IF NOT EXISTS idx_redemption_codes_ticket_level
    ON redemption_codes(ticket_level);

CREATE TABLE IF NOT EXISTS redemption_claims (
    id TEXT PRIMARY KEY,
    code_id INTEGER NOT NULL UNIQUE,
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

CREATE INDEX IF NOT EXISTS idx_redemption_claims_claimant
    ON redemption_claims(claimant_type, claimant);

CREATE INDEX IF NOT EXISTS idx_redemption_claims_claimed_at
    ON redemption_claims(claimed_at);
