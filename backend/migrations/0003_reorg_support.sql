ALTER TABLE orders ADD COLUMN block_hash TEXT NOT NULL DEFAULT '';
ALTER TABLE indexer_cursors ADD COLUMN last_indexed_block_hash TEXT;
