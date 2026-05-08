# TicketSale Mainnet Redeploy Runbook

This runbook is for a clean production redeploy separated from test contracts.
Do not reuse test token addresses, test RPC endpoints, or test backend signer keys.

## 1. Prepare Env

Choose the target chain template and copy it to a local ignored env file:

```bash
cp templates/ticketsale-deploy-eth-mainnet.env.example .env.deploy.eth-mainnet
# or
cp templates/ticketsale-deploy-bsc-mainnet.env.example .env.deploy.bsc-mainnet
```

Fill:

- `RPC_URL`
- `PRIVATE_KEY`
- `OWNER`
- `FINAL_OWNER`
- `PAUSER`
- `PROXY_ADMIN_OWNER`
- `TREASURY`
- `PURCHASE_SIGNER`
- `LEVEL_1_START_TIMESTAMPS`
- `LEVEL_1_PRICES`
- `LEVEL_2_START_TIMESTAMPS`
- `LEVEL_2_PRICES`
- `LEVEL_3_START_TIMESTAMPS`
- `LEVEL_3_PRICES`

The included templates already contain the 2026 ticket price schedule:

| Window | Asia/Shanghai start | Unix seconds | Standard | Pro | Whale |
| --- | --- | ---: | ---: | ---: | ---: |
| Early Bird | 2026-05-01 00:00 | 1777564800 | 88 | 398 | 1889 |
| Phase 2 | 2026-06-01 00:00 | 1780243200 | 128 | 498 | 2889 |
| Phase 3 | 2026-07-01 00:00 | 1782835200 | 168 | 698 | 3889 |
| Last Call | 2026-07-21 00:00 | 1784563200 | 299 | 1899 | 9999 |

For a one-pass deploy, set `OWNER` to the deployer address derived from
`PRIVATE_KEY`, and set `FINAL_OWNER` to the long-term default admin wallet or
multisig. The deploy script configures signer and price schedules while the
deployer is temporary admin, then grants `DEFAULT_ADMIN_ROLE` to `FINAL_OWNER`
and revokes it from the deployer before the script finishes.

Price arrays must be comma-separated when there are multiple schedule points.
Example for 6-decimal tokens:

```bash
LEVEL_1_START_TIMESTAMPS=0,1798761600
LEVEL_1_PRICES=100000000,80000000
```

Example for 18-decimal tokens:

```bash
LEVEL_1_START_TIMESTAMPS=0,1798761600
LEVEL_1_PRICES=100000000000000000000,80000000000000000000
```

## 2. Preflight

```bash
set -a
source .env.deploy.eth-mainnet
set +a

forge test
./scripts/check-storage-layout.sh

DEPLOYER="$(cast wallet address --private-key "$PRIVATE_KEY")"
echo "deployer=$DEPLOYER"
cast nonce "$DEPLOYER" --rpc-url "$RPC_URL"
cast balance "$DEPLOYER" --rpc-url "$RPC_URL"
```

## 3. Dry Run Twice

Run two simulations and compare output addresses:

```bash
tmpdir="$(mktemp -d .tmp-deploy-dryrun.XXXXXX)"

DEPLOY_OUTPUT_FILE="$tmpdir/deploy-1.json" \
forge script script/TicketSale.s.sol:TicketSaleScript \
  --rpc-url "$RPC_URL" \
  --private-key "$PRIVATE_KEY" \
  > "$tmpdir/run-1.log" 2>&1

DEPLOY_OUTPUT_FILE="$tmpdir/deploy-2.json" \
forge script script/TicketSale.s.sol:TicketSaleScript \
  --rpc-url "$RPC_URL" \
  --private-key "$PRIVATE_KEY" \
  > "$tmpdir/run-2.log" 2>&1

diff -u "$tmpdir/deploy-1.json" "$tmpdir/deploy-2.json"
cat "$tmpdir/deploy-1.json"
```

Confirm the predicted contract addresses have no code:

```bash
IMPLEMENTATION="$(jq -r '.implementation' "$tmpdir/deploy-1.json")"
PROXY="$(jq -r '.proxy' "$tmpdir/deploy-1.json")"
PROXY_ADMIN="$(jq -r '.proxy_admin' "$tmpdir/deploy-1.json")"

cast code "$IMPLEMENTATION" --rpc-url "$RPC_URL"
cast code "$PROXY" --rpc-url "$RPC_URL"
cast code "$PROXY_ADMIN" --rpc-url "$RPC_URL"
```

All three should return `0x`.

## 4. Broadcast Deploy

Re-check the deployer nonce immediately before broadcasting:

```bash
cast nonce "$DEPLOYER" --rpc-url "$RPC_URL"
```

Broadcast:

```bash
forge script script/TicketSale.s.sol:TicketSaleScript \
  --rpc-url "$RPC_URL" \
  --private-key "$PRIVATE_KEY" \
  --broadcast
```

Save `DEPLOY_OUTPUT_FILE` and broadcast artifacts.

## 5. Configure Price Schedules

The deploy script configures price schedules automatically when
`LEVEL_N_START_TIMESTAMPS` and `LEVEL_N_PRICES` are set.

Use the manual calls below only for post-deploy correction.
Only the current `DEFAULT_ADMIN_ROLE` holder can call `setPriceSchedule`.

```bash
SALE_PROXY="$(jq -r '.proxy' "$DEPLOY_OUTPUT_FILE")"
OWNER_PRIVATE_KEY=0xyour_owner_private_key_or_use_multisig

cast send "$SALE_PROXY" \
  "setPriceSchedule(uint8,uint64[],uint256[])" \
  1 "[$LEVEL_1_START_TIMESTAMPS]" "[$LEVEL_1_PRICES]" \
  --rpc-url "$RPC_URL" \
  --private-key "$OWNER_PRIVATE_KEY"

cast send "$SALE_PROXY" \
  "setPriceSchedule(uint8,uint64[],uint256[])" \
  2 "[$LEVEL_2_START_TIMESTAMPS]" "[$LEVEL_2_PRICES]" \
  --rpc-url "$RPC_URL" \
  --private-key "$OWNER_PRIVATE_KEY"

cast send "$SALE_PROXY" \
  "setPriceSchedule(uint8,uint64[],uint256[])" \
  3 "[$LEVEL_3_START_TIMESTAMPS]" "[$LEVEL_3_PRICES]" \
  --rpc-url "$RPC_URL" \
  --private-key "$OWNER_PRIVATE_KEY"
```

## 6. Verify On-chain State

```bash
DEFAULT_ADMIN_ROLE=0x0000000000000000000000000000000000000000000000000000000000000000
PAUSER_ROLE="$(cast keccak 'PAUSER_ROLE')"

cast call "$SALE_PROXY" "treasury()(address)" --rpc-url "$RPC_URL"
cast call "$SALE_PROXY" "purchase_signer()(address)" --rpc-url "$RPC_URL"
cast call "$SALE_PROXY" "payment_tokens(address)(bool)" "$USDT_TOKEN" --rpc-url "$RPC_URL"
cast call "$SALE_PROXY" "payment_tokens(address)(bool)" "$USDC_TOKEN" --rpc-url "$RPC_URL"
cast call "$SALE_PROXY" "hasRole(bytes32,address)(bool)" "$DEFAULT_ADMIN_ROLE" "$OWNER" --rpc-url "$RPC_URL"
cast call "$SALE_PROXY" "hasRole(bytes32,address)(bool)" "$PAUSER_ROLE" "$PAUSER" --rpc-url "$RPC_URL"
cast call "$SALE_PROXY" "getPriceSchedule(uint8)((uint64[],uint256[]))" 1 --rpc-url "$RPC_URL"
cast call "$SALE_PROXY" "getPriceSchedule(uint8)((uint64[],uint256[]))" 2 --rpc-url "$RPC_URL"
cast call "$SALE_PROXY" "getPriceSchedule(uint8)((uint64[],uint256[]))" 3 --rpc-url "$RPC_URL"
```

## 7. Backend Handoff

Update backend production config:

- `APP_CHAINS_JSON`
- `PURCHASE_SIGNER_PRIVATE_KEY`
- token decimals config if needed
- production DB/runtime environment

Example `APP_CHAINS_JSON` item:

```json
[
  {
    "chain_id": 1,
    "rpc_url": "https://your-ethereum-mainnet-rpc",
    "sale_contract": "0xDEPLOYED_PROXY",
    "start_block": 12345678,
    "confirmations": 12
  }
]
```

Use the deployment block as `start_block`.

## 8. Frontend Handoff

Update frontend production config:

- sale contract address for the deployed chain
- production payment token addresses
- `NEXT_PUBLIC_API_BASE_URL`

Rebuild and redeploy frontend after backend is updated.

## 9. Smoke Test

Before public launch:

- read ticket prices from `/api/purchase-prices`
- create a purchase quote with USDT
- create a purchase quote with USDC
- verify discount/referral quote behavior
- buy one low-risk test ticket with a production wallet if acceptable
- confirm backend indexes the order and admin order detail displays decimal-normalized amounts
