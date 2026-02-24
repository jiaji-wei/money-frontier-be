# Indexer Behavior

## Finality / Confirmations

The indexer only processes up to `latest_finalized_block`.

For each chain:

- `confirmations` is configured in `APP_CHAINS_JSON`.
- finalized block is computed as `latest_block - confirmations`.

## Reorg Handling

The cursor stores both:

- `last_indexed_block`
- `last_indexed_block_hash`

On every sync iteration:

1. Fetch canonical hash of `last_indexed_block` from RPC.
2. Compare with `last_indexed_block_hash`.
3. If mismatch, treat as reorg:
   - rollback from `max(start_block, last_indexed_block - INDEXER_REORG_ROLLBACK_BLOCKS + 1)`
   - delete affected `orders` and `tickets`
   - reset cursor to `rollback_from - 1`
   - reindex forward to finalized block

This gives deterministic recovery with bounded rollback cost.

## Relevant Config

- `INDEXER_POLL_INTERVAL_SECS`
- `INDEXER_BATCH_SIZE`
- `INDEXER_REORG_ROLLBACK_BLOCKS`
- `APP_CHAINS_JSON[*].start_block`
- `APP_CHAINS_JSON[*].confirmations`

## Signin Challenge Cleanup

The backend also runs a periodic cleanup task for signin challenges.

- Deletes rows where `expires_at < now - SIGNIN_CLEANUP_RETENTION_SECS`.
- Deletes rows where `used_at` is set and `used_at < now - SIGNIN_CLEANUP_RETENTION_SECS`.
- Keeps recent used rows for a retention window to aid security auditing.

Related config:

- `SIGNIN_CLEANUP_INTERVAL_SECS`
- `SIGNIN_CLEANUP_RETENTION_SECS`

Set `SIGNIN_CLEANUP_INTERVAL_SECS=0` to disable cleanup (not recommended for production).
