CREATE TABLE IF NOT EXISTS admin_wallets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    wallet_address TEXT NOT NULL UNIQUE,
    role TEXT NOT NULL CHECK (role IN ('viewer', 'operator', 'finance')),
    status TEXT NOT NULL CHECK (status IN ('active', 'disabled')),
    notes TEXT,
    created_by TEXT NOT NULL,
    updated_by TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_admin_wallets_status
    ON admin_wallets(status);
