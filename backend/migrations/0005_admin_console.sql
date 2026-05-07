ALTER TABLE promotion_codes ADD COLUMN notes TEXT;

CREATE TABLE IF NOT EXISTS admin_signin_challenges (
    id TEXT PRIMARY KEY,
    wallet TEXT NOT NULL,
    challenge_message TEXT NOT NULL,
    nonce TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    used_at INTEGER,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_admin_signin_challenges_wallet
    ON admin_signin_challenges(wallet);

CREATE INDEX IF NOT EXISTS idx_admin_signin_challenges_expiry
    ON admin_signin_challenges(expires_at);

CREATE TABLE IF NOT EXISTS admin_audit_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    actor_wallet TEXT NOT NULL,
    actor_role TEXT NOT NULL,
    action TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_id TEXT,
    before_json TEXT,
    after_json TEXT,
    ip_address TEXT,
    user_agent TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_admin_audit_logs_actor
    ON admin_audit_logs(actor_wallet);

CREATE INDEX IF NOT EXISTS idx_admin_audit_logs_target
    ON admin_audit_logs(target_type, target_id);

CREATE INDEX IF NOT EXISTS idx_admin_audit_logs_created_at
    ON admin_audit_logs(created_at);
