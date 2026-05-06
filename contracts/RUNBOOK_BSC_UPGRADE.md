# BSC TicketSale Upgrade Runbook

Use this runbook for the BSC `TicketSale` proxy when upgrading the referral and discount flow.

## 1. Prerequisites

Prepare a local env from `templates/ticketsale-upgrade-bsc.env.example` and set the values for the live proxy, proxy admin, broadcast key, and verification assertions.

If the proxy already exists on BSC, you can derive most of the env file from chain state first:

```bash
./scripts/prepare-bsc-upgrade-env.sh \
  --rpc-url "$RPC_URL" \
  --proxy "$TICKET_SALE_PROXY" \
  --output ./.env.upgrade.bsc
```

Then edit the generated file to:

- set `PRIVATE_KEY`
- replace any `__SET_*__` placeholders
- replace `EXPECTED_IMPLEMENTATION` with the upgraded implementation address after the broadcast and before verify

Required inputs:

- `RPC_URL`
- `PRIVATE_KEY`
- `TICKET_SALE_PROXY`
- `PROXY_ADMIN`
- `UPGRADE_OUTPUT_FILE`

Optional broadcast inputs:

- `NEW_IMPLEMENTATION` or leave unset to deploy a fresh implementation
- `PURCHASE_SIGNER`

Optional preflight assertions:

- `EXPECTED_PROXY_ADMIN`
- `EXPECTED_PROXY_ADMIN_OWNER`
- `EXPECTED_IMPLEMENTATION`
- `EXPECTED_PURCHASE_SIGNER`

Required post-upgrade verification assertions:

- `EXPECTED_DEFAULT_ADMIN`
- `EXPECTED_PAUSER`
- `EXPECTED_TREASURY`
- `EXPECTED_PURCHASE_SIGNER`

Before touching BSC, also confirm the backend signer configuration matches the post-upgrade purchase signer you plan to enforce.

## 2. Build And Layout Check

Run these checks before any broadcast:

```bash
forge build
./scripts/check-storage-layout.sh
```

If either command fails, stop and fix the code or the expected storage baseline before proceeding.

## 3. Preflight

Inspect the live proxy state and compare it against any expected values you want to enforce:

```bash
forge script script/PreflightTicketSaleUpgrade.s.sol:PreflightTicketSaleUpgradeScript \
  --rpc-url "$RPC_URL"
```

Recommended use:

- set `EXPECTED_PROXY_ADMIN` to confirm the proxy slot points at the intended admin
- set `EXPECTED_PROXY_ADMIN_OWNER` to confirm the admin owner is the expected EOA
- set `EXPECTED_IMPLEMENTATION` when you are resuming or validating a specific implementation
- set `EXPECTED_PURCHASE_SIGNER` when the current implementation already exposes `purchase_signer()` and you want to guard signer rotation during the upgrade

For older implementations that predate the signed purchase flow, preflight reports `purchase_signer_supported = false` instead of reverting. In that case, keep the target signer in the generated env draft and enforce it during the upgrade and post-upgrade verify steps.

Capture the output and keep it with the upgrade record.

## 4. Broadcast Upgrade

Run the actual upgrade from the same env file:

```bash
forge script script/UpgradeTicketSale.s.sol:UpgradeTicketSaleScript \
  --rpc-url "$RPC_URL" \
  --private-key "$PRIVATE_KEY" \
  --broadcast
```

Operational guidance:

- keep `UPGRADE_OUTPUT_FILE` set so the script writes the proxy, implementation, and signer result to JSON
- save the console output with the broadcast record, for example by teeing it into an artifact log
- do not proceed to app handoff until the upgrade output shows the expected implementation and signer values

## 5. Post-upgrade Verification

Verify the live proxy state immediately after the broadcast:

```bash
forge script script/VerifyTicketSaleUpgrade.s.sol:VerifyTicketSaleUpgradeScript \
  --rpc-url "$RPC_URL"
```

This must confirm:

- proxy admin matches the expected admin
- implementation matches the upgraded implementation
- `DEFAULT_ADMIN_ROLE` is present on the expected admin account
- `PAUSER_ROLE` is present on the expected pauser account
- treasury matches the expected treasury
- purchase signer matches the expected signer

Keep the verification output alongside the broadcast log and the `UPGRADE_OUTPUT_FILE` JSON.

## 6. Backend And Frontend Handoff

After verification passes:

- update backend runtime config so `PURCHASE_SIGNER_PRIVATE_KEY` matches the signer that now owns the contract authorization flow
- restart or redeploy the backend so it reads the new signer value
- confirm frontend deployment config still points at the same BSC proxy address through `NEXT_PUBLIC_CONTRACT_JIMMY`
- confirm frontend API config still points at the backend that knows the upgraded signer through `NEXT_PUBLIC_API_BASE_URL`

If the proxy address changes in a future redeploy, update both the backend and frontend config together before re-enabling purchases.

## 7. Rollback Posture

If verification fails or the app handoff is inconsistent:

1. pause sales if the issue could affect purchases
2. keep the proxy address unchanged
3. upgrade back to the previous known-good implementation if you need to roll back the contract logic
4. rerun preflight and post-upgrade verification against the rolled-back implementation
5. only resume operations after the verification output matches the expected state

The rollback target is implementation parity, not a new proxy address.
