CREATE TABLE IF NOT EXISTS orders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    chain_id INTEGER NOT NULL,
    tx_hash TEXT NOT NULL,
    log_index INTEGER NOT NULL,
    block_number INTEGER NOT NULL,
    order_id TEXT NOT NULL,
    buyer_address TEXT NOT NULL,
    payment_token TEXT NOT NULL,
    total_amount TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE(chain_id, tx_hash, log_index)
);

CREATE TABLE IF NOT EXISTS tickets (
    id TEXT PRIMARY KEY,
    chain_id INTEGER NOT NULL,
    order_id TEXT NOT NULL,
    source_order_row_id INTEGER NOT NULL,
    owner_wallet TEXT,
    owner_email TEXT,
    ticket_level INTEGER NOT NULL,
    unit_price TEXT NOT NULL,
    qr_payload TEXT NOT NULL,
    qr_version INTEGER NOT NULL,
    status TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY(source_order_row_id) REFERENCES orders(id)
);

CREATE INDEX IF NOT EXISTS idx_tickets_owner_wallet ON tickets(owner_wallet, status);
CREATE INDEX IF NOT EXISTS idx_tickets_owner_email ON tickets(owner_email, status);
