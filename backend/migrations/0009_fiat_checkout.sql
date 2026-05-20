CREATE TABLE IF NOT EXISTS fiat_checkout_sessions (
    id TEXT PRIMARY KEY,
    stripe_session_id TEXT UNIQUE,
    email TEXT NOT NULL,
    currency TEXT NOT NULL,
    level_ids_json TEXT NOT NULL,
    quantities_json TEXT NOT NULL,
    referral_code_id INTEGER,
    discount_code_id INTEGER,
    original_amount_cents INTEGER NOT NULL,
    discount_amount_cents INTEGER NOT NULL,
    final_amount_cents INTEGER NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('pending', 'paid', 'expired', 'cancelled', 'failed')),
    stripe_url TEXT,
    payment_intent_id TEXT,
    internal_order_row_id INTEGER,
    created_tickets INTEGER NOT NULL DEFAULT 0,
    expires_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY(referral_code_id) REFERENCES promotion_codes(id),
    FOREIGN KEY(discount_code_id) REFERENCES promotion_codes(id),
    FOREIGN KEY(internal_order_row_id) REFERENCES orders(id)
);

CREATE INDEX IF NOT EXISTS idx_fiat_checkout_sessions_email_status
    ON fiat_checkout_sessions(email, status);

CREATE INDEX IF NOT EXISTS idx_fiat_checkout_sessions_stripe_session
    ON fiat_checkout_sessions(stripe_session_id);
