CREATE TABLE IF NOT EXISTS signin_challenges (
    id TEXT PRIMARY KEY,
    wallet TEXT NOT NULL,
    challenge_message TEXT NOT NULL,
    nonce TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    used_at INTEGER,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_signin_challenges_wallet ON signin_challenges(wallet);
CREATE INDEX IF NOT EXISTS idx_signin_challenges_expiry ON signin_challenges(expires_at);

CREATE TABLE IF NOT EXISTS indexer_cursors (
    chain_id INTEGER PRIMARY KEY,
    last_indexed_block INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
