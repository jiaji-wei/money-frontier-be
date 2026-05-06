CREATE TABLE IF NOT EXISTS promotion_codes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code_normalized TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK(kind IN ('referral', 'discount')),
    status TEXT NOT NULL CHECK(status IN ('active', 'paused', 'expired', 'exhausted')),
    beneficiary_wallet TEXT,
    valid_from INTEGER,
    valid_until INTEGER,
    max_total_uses INTEGER,
    max_uses_per_wallet INTEGER,
    first_purchase_only INTEGER NOT NULL DEFAULT 0,
    stacking_policy TEXT,
    applicable_chain_ids TEXT,
    applicable_ticket_levels TEXT,
    discount_type TEXT,
    discount_value TEXT,
    max_discount_amount TEXT,
    commission_type TEXT,
    commission_value TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_promotion_codes_kind_status
    ON promotion_codes(kind, status);

CREATE TABLE IF NOT EXISTS wallet_referral_bindings (
    wallet_address TEXT PRIMARY KEY,
    referral_code_id INTEGER NOT NULL,
    bound_at INTEGER NOT NULL,
    first_bound_source TEXT NOT NULL CHECK(first_bound_source IN ('signin', 'purchase_intent')),
    FOREIGN KEY(referral_code_id) REFERENCES promotion_codes(id)
);

CREATE INDEX IF NOT EXISTS idx_wallet_referral_bindings_referral_code_id
    ON wallet_referral_bindings(referral_code_id);

CREATE TABLE IF NOT EXISTS purchase_intents (
    id TEXT PRIMARY KEY,
    wallet_address TEXT NOT NULL,
    chain_id INTEGER NOT NULL,
    payment_token TEXT NOT NULL,
    level_ids_json TEXT NOT NULL,
    quantities_json TEXT NOT NULL,
    referral_code_id INTEGER,
    discount_code_id INTEGER,
    original_total_amount TEXT NOT NULL,
    discount_amount TEXT NOT NULL,
    final_total_amount TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('pending', 'submitted', 'confirmed', 'expired', 'cancelled')),
    tx_hash TEXT,
    order_id TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY(referral_code_id) REFERENCES promotion_codes(id),
    FOREIGN KEY(discount_code_id) REFERENCES promotion_codes(id)
);

CREATE INDEX IF NOT EXISTS idx_purchase_intents_wallet_status
    ON purchase_intents(wallet_address, status);
CREATE INDEX IF NOT EXISTS idx_purchase_intents_discount_code_id
    ON purchase_intents(discount_code_id);
CREATE INDEX IF NOT EXISTS idx_purchase_intents_expires_at
    ON purchase_intents(expires_at);

CREATE TABLE IF NOT EXISTS discount_redemptions (
    purchase_intent_id TEXT PRIMARY KEY,
    discount_code_id INTEGER NOT NULL,
    wallet_address TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('reserved', 'confirmed', 'released')),
    tx_hash TEXT,
    order_id TEXT,
    reserved_at INTEGER NOT NULL,
    confirmed_at INTEGER,
    released_at INTEGER,
    FOREIGN KEY(purchase_intent_id) REFERENCES purchase_intents(id),
    FOREIGN KEY(discount_code_id) REFERENCES promotion_codes(id)
);

CREATE INDEX IF NOT EXISTS idx_discount_redemptions_discount_code_wallet
    ON discount_redemptions(discount_code_id, wallet_address);
CREATE INDEX IF NOT EXISTS idx_discount_redemptions_status
    ON discount_redemptions(status);

CREATE TABLE IF NOT EXISTS order_promotions_snapshot (
    order_row_id INTEGER PRIMARY KEY,
    wallet_address TEXT NOT NULL,
    referral_code_id INTEGER,
    discount_code_id INTEGER,
    original_total_amount TEXT NOT NULL,
    discount_amount TEXT NOT NULL,
    paid_amount TEXT NOT NULL,
    commission_base_amount TEXT NOT NULL,
    commission_amount TEXT NOT NULL,
    rule_version TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    FOREIGN KEY(order_row_id) REFERENCES orders(id),
    FOREIGN KEY(referral_code_id) REFERENCES promotion_codes(id),
    FOREIGN KEY(discount_code_id) REFERENCES promotion_codes(id)
);

CREATE INDEX IF NOT EXISTS idx_order_promotions_snapshot_wallet_address
    ON order_promotions_snapshot(wallet_address);
