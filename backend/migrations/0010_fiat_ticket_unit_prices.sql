ALTER TABLE fiat_checkout_sessions
ADD COLUMN unit_prices_cents_json TEXT NOT NULL DEFAULT '[]';

UPDATE fiat_checkout_sessions
SET unit_prices_cents_json = '[' || CAST(
    original_amount_cents / CAST(json_extract(quantities_json, '$[0]') AS INTEGER)
    AS TEXT
) || ']'
WHERE unit_prices_cents_json = '[]'
  AND json_array_length(level_ids_json) = 1
  AND json_array_length(quantities_json) = 1
  AND CAST(json_extract(quantities_json, '$[0]') AS INTEGER) > 0;

UPDATE tickets
SET unit_price = (
    SELECT CAST(
        f.original_amount_cents / CAST(json_extract(f.quantities_json, '$[0]') AS INTEGER)
        AS TEXT
    )
    FROM fiat_checkout_sessions f
    WHERE f.internal_order_row_id = tickets.source_order_row_id
)
WHERE chain_id = 0
  AND unit_price = '0'
  AND EXISTS (
      SELECT 1
      FROM fiat_checkout_sessions f
      WHERE f.internal_order_row_id = tickets.source_order_row_id
        AND json_array_length(f.level_ids_json) = 1
        AND json_array_length(f.quantities_json) = 1
        AND CAST(json_extract(f.quantities_json, '$[0]') AS INTEGER) > 0
  );
