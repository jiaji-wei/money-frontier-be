CREATE TABLE IF NOT EXISTS email_access_challenges (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at INTEGER NOT NULL,
    used_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_email_access_challenges_email
    ON email_access_challenges(email);

CREATE INDEX IF NOT EXISTS idx_email_access_challenges_expiry
    ON email_access_challenges(expires_at);
