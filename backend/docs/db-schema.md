# Ticket Backend DB Schema

## Overview

The backend persists three domains:

1. `orders`: on-chain purchase event records (`TicketsPurchased`)
2. `tickets`: off-chain ticket ownership and QR lifecycle
3. `signin_challenges`: one-time login challenge/nonce records
4. `indexer_cursors`: per-chain block cursor for background indexing

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

## Data Lifecycle

1. Wallet signs in:
   - create row in `signin_challenges`
   - consume row during verification (`used_at` set)
   - cleanup worker periodically deletes expired/old-used rows
2. Purchase indexed (via `/tickets` notify or indexer):
   - upsert into `orders` by unique key
   - insert `tickets` rows if this order log has not produced tickets before
3. Ticket transfer:
   - source ticket set to `transferred_out`
   - destination ticket inserted as new `active` row with new QR payload
