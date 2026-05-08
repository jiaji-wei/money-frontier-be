# Ticket Backend DB Schema

## Overview

The backend persists six domains:

1. `orders`: on-chain purchase event records (`TicketsPurchased`)
2. `tickets`: off-chain ticket ownership and QR lifecycle
3. `signin_challenges`: one-time login challenge/nonce records
4. `indexer_cursors`: per-chain block cursor for background indexing
5. `promotions`: shared code registry, referral bindings, and discount redemptions
6. `promotion snapshots`: immutable attribution rows captured at order finalization

All timestamps are Unix seconds.

## Tables

### orders

- `id` INTEGER PK AUTOINCREMENT
- `chain_id` INTEGER NOT NULL
- `tx_hash` TEXT NOT NULL
- `log_index` INTEGER NOT NULL
- `block_number` INTEGER NOT NULL
- `block_hash` TEXT NOT NULL
- `order_id` TEXT NOT NULL
- `buyer_address` TEXT NOT NULL
- `payment_token` TEXT NOT NULL
- `total_amount` TEXT NOT NULL
- `created_at` INTEGER NOT NULL
- UNIQUE(`chain_id`, `tx_hash`, `log_index`)

Notes:
- Uniqueness key guarantees idempotency across API notify path and background indexer path.

### tickets

- `id` TEXT PK (UUID)
- `chain_id` INTEGER NOT NULL
- `order_id` TEXT NOT NULL
- `source_order_row_id` INTEGER NOT NULL FK -> `orders.id`
- `owner_wallet` TEXT NULL
- `owner_email` TEXT NULL
- `ticket_level` INTEGER NOT NULL
- `unit_price` TEXT NOT NULL
- `qr_payload` TEXT NOT NULL
- `qr_version` INTEGER NOT NULL
- `status` TEXT NOT NULL (`active`, `transferred_out`)
- `created_at` INTEGER NOT NULL
- `updated_at` INTEGER NOT NULL

Indexes:
- `idx_tickets_owner_wallet(owner_wallet, status)`
- `idx_tickets_owner_email(owner_email, status)`

Notes:
- One purchase line item with quantity `N` creates `N` ticket rows.
- Transfer marks original row `transferred_out` and inserts a new `active` row with a rotated QR payload.

### signin_challenges

- `id` TEXT PK (UUID)
- `wallet` TEXT NOT NULL
- `challenge_message` TEXT NOT NULL
- `nonce` TEXT NOT NULL
- `expires_at` INTEGER NOT NULL
- `used_at` INTEGER NULL
- `created_at` INTEGER NOT NULL

Indexes:
- `idx_signin_challenges_wallet(wallet)`
- `idx_signin_challenges_expiry(expires_at)`

Notes:
- Challenge is one-time use.
- Verification flow atomically checks: matching wallet + not expired + not used, then sets `used_at`.

### indexer_cursors

- `chain_id` INTEGER PK
- `last_indexed_block` INTEGER NOT NULL
- `last_indexed_block_hash` TEXT NULL
- `updated_at` INTEGER NOT NULL

Notes:
- Maintains independent cursors per chain.
- `last_indexed_block_hash` is used to detect chain reorg against canonical block hash.
- Startup behavior:
  - if cursor exists: resume from `last_indexed_block + 1`
  - if cursor missing: starts from `start_block - 1` (if configured), otherwise from `latest_finalized - 1`
  - if cursor hash mismatches canonical chain hash: rollback recent window and reindex

### promotion_codes

- `id` INTEGER PK AUTOINCREMENT
- `code_normalized` TEXT NOT NULL UNIQUE
- `kind` TEXT NOT NULL (`referral`, `discount`)
- `status` TEXT NOT NULL (`active`, `paused`, `expired`, `exhausted`)
- `beneficiary_wallet` TEXT NULL
- `valid_from` INTEGER NULL
- `valid_until` INTEGER NULL
- `max_total_uses` INTEGER NULL
- `max_uses_per_wallet` INTEGER NULL
- `first_purchase_only` INTEGER NOT NULL DEFAULT `0`
- `stacking_policy` TEXT NULL
- `applicable_chain_ids` TEXT NULL
- `applicable_ticket_levels` TEXT NULL
- `discount_type` TEXT NULL
- `discount_value` TEXT NULL
- `max_discount_amount` TEXT NULL
- `commission_type` TEXT NULL
- `commission_value` TEXT NULL
- `created_at` INTEGER NOT NULL
- `updated_at` INTEGER NOT NULL

For `kind = referral`, `discount_type` and `discount_value` represent the optional automatic buyer discount applied from the referral when no manual discount code is provided. For `kind = discount`, those fields represent the standalone discount code rule.

Indexes:
- `idx_promotion_codes_kind_status(kind, status)`

Notes:
- Stores referral and discount codes in one registry.
- Applicability lists are stored as JSON text for backend-side evaluation.

### wallet_referral_bindings

- `wallet_address` TEXT PK
- `referral_code_id` INTEGER NOT NULL FK -> `promotion_codes.id`
- `bound_at` INTEGER NOT NULL
- `first_bound_source` TEXT NOT NULL (`signin`, `purchase_intent`)

Indexes:
- `idx_wallet_referral_bindings_referral_code_id(referral_code_id)`

Notes:
- First successful bind wins because `wallet_address` is unique.
- Later referral submissions for the same wallet are ignored at the service layer.

### purchase_intents

- `id` TEXT PK
- `wallet_address` TEXT NOT NULL
- `chain_id` INTEGER NOT NULL
- `payment_token` TEXT NOT NULL
- `level_ids_json` TEXT NOT NULL
- `quantities_json` TEXT NOT NULL
- `referral_code_id` INTEGER NULL FK -> `promotion_codes.id`
- `discount_code_id` INTEGER NULL FK -> `promotion_codes.id`
- `original_total_amount` TEXT NOT NULL
- `discount_amount` TEXT NOT NULL
- `final_total_amount` TEXT NOT NULL
- `expires_at` INTEGER NOT NULL
- `status` TEXT NOT NULL (`pending`, `submitted`, `confirmed`, `expired`, `cancelled`)
- `tx_hash` TEXT NULL
- `order_id` TEXT NULL
- `created_at` INTEGER NOT NULL
- `updated_at` INTEGER NOT NULL

Indexes:
- `idx_purchase_intents_wallet_status(wallet_address, status)`
- `idx_purchase_intents_discount_code_id(discount_code_id)`
- `idx_purchase_intents_expires_at(expires_at)`

Notes:
- Captures the signed purchase quote the contract flow will consume.
- `level_ids_json` and `quantities_json` preserve the exact requested basket.

### discount_redemptions

- `purchase_intent_id` TEXT PK FK -> `purchase_intents.id`
- `discount_code_id` INTEGER NOT NULL FK -> `promotion_codes.id`
- `wallet_address` TEXT NOT NULL
- `status` TEXT NOT NULL (`reserved`, `confirmed`, `released`)
- `tx_hash` TEXT NULL
- `order_id` TEXT NULL
- `reserved_at` INTEGER NOT NULL
- `confirmed_at` INTEGER NULL
- `released_at` INTEGER NULL

Indexes:
- `idx_discount_redemptions_discount_code_wallet(discount_code_id, wallet_address)`
- `idx_discount_redemptions_status(status)`

Notes:
- One redemption row exists per purchase intent.
- Reservation happens before chain purchase and is either confirmed or released later.

### order_promotions_snapshot

- `order_row_id` INTEGER PK FK -> `orders.id`
- `wallet_address` TEXT NOT NULL
- `referral_code_id` INTEGER NULL FK -> `promotion_codes.id`
- `discount_code_id` INTEGER NULL FK -> `promotion_codes.id`
- `original_total_amount` TEXT NOT NULL
- `discount_amount` TEXT NOT NULL
- `paid_amount` TEXT NOT NULL
- `commission_base_amount` TEXT NOT NULL
- `commission_amount` TEXT NOT NULL
- `rule_version` TEXT NOT NULL
- `created_at` INTEGER NOT NULL

Indexes:
- `idx_order_promotions_snapshot_wallet_address(wallet_address)`

Notes:
- Immutable order-level snapshot for audit and settlement.
- One row per materialized order because `order_row_id` is unique.

## Data Lifecycle

1. Wallet signs in:
   - create row in `signin_challenges`
   - consume row during verification (`used_at` set)
   - cleanup worker periodically deletes expired/old-used rows
2. Purchase indexed (via `/tickets` notify or indexer):
   - upsert into `orders` by unique key
   - insert `tickets` rows if this order log has not produced tickets before
   - finalize promotion state using stored referral binding and purchase intent records
3. Ticket transfer:
   - source ticket set to `transferred_out`
   - destination ticket inserted as new `active` row with new QR payload
