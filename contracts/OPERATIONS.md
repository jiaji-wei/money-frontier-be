# TicketSale Contract Operations

All script parameters should be provided via `contracts/.env`.

Suggested bootstrap:

```bash
cp .env.example .env
```

## 1. Deploy (Transparent Proxy)

Required env vars:

- `OWNER`
- `PAUSER` (optional, defaults to `OWNER`)
- `PROXY_ADMIN_OWNER` (optional, defaults to `OWNER`)
- `TREASURY`
- `PURCHASE_SIGNER` (optional but required for signed purchase-intent flow)
- `USDT_TOKEN`
- `USDC_TOKEN`
- `PRIVATE_KEY`
- `RPC_URL`

Run:

```bash
forge script script/TicketSale.s.sol:TicketSaleScript \
  --rpc-url "$RPC_URL" \
  --private-key "$PRIVATE_KEY" \
  --broadcast
```

The script logs:

- implementation address
- proxy address
- proxy admin address
- proxy admin owner
- default admin
- pauser
- treasury
- purchase signer
- token addresses

## 2. Upgrade

For BSC upgrades, start from `templates/ticketsale-upgrade-bsc.env.example`.
The exact preflight, broadcast, verification, handoff, and rollback sequence lives in `RUNBOOK_BSC_UPGRADE.md`.

If you already have a live BSC proxy and want a draft env file populated from chain state, generate it first:

```bash
./scripts/prepare-bsc-upgrade-env.sh \
  --rpc-url "$RPC_URL" \
  --proxy "$TICKET_SALE_PROXY" \
  --output ./.env.upgrade.bsc
```

The helper resolves:

- `PROXY_ADMIN`
- `EXPECTED_PROXY_ADMIN_OWNER`
- current implementation
- current treasury when readable
- current purchase signer when readable

You still need to fill any governance-role placeholders that cannot be enumerated from chain state, and you must replace `EXPECTED_IMPLEMENTATION` with the upgraded implementation address before running the post-upgrade verify script.

Broadcast inputs:

- `TICKET_SALE_PROXY`
- `PRIVATE_KEY`
- `RPC_URL`
- `PROXY_ADMIN` (optional safety check)
- `NEW_IMPLEMENTATION` (optional; if omitted, script deploys a new `TicketSale`)
- `PURCHASE_SIGNER` (optional; set to rotate signer during upgrade)

Post-upgrade verification inputs:

- `EXPECTED_PROXY_ADMIN_OWNER`
- `EXPECTED_IMPLEMENTATION`
- `EXPECTED_DEFAULT_ADMIN`
- `EXPECTED_PAUSER`
- `EXPECTED_TREASURY`
- `EXPECTED_PURCHASE_SIGNER`

Run:

```bash
forge script script/UpgradeTicketSale.s.sol:UpgradeTicketSaleScript \
  --rpc-url "$RPC_URL" \
  --private-key "$PRIVATE_KEY" \
  --broadcast
```

## 3. Preflight Upgrade Check

Run this before broadcasting an upgrade to inspect the live proxy state and compare it against any expected values you set.

Tip: if you have not prepared your env file yet, run `./scripts/prepare-bsc-upgrade-env.sh` first and source or copy the generated values.

Required env vars:

- `TICKET_SALE_PROXY`
- `RPC_URL`

Optional assertions:

- `EXPECTED_PROXY_ADMIN` (optional)
- `EXPECTED_PROXY_ADMIN_OWNER` (optional)
- `EXPECTED_IMPLEMENTATION` (optional)
- `EXPECTED_PURCHASE_SIGNER` (optional)

Run:

```bash
forge script script/PreflightTicketSaleUpgrade.s.sol:PreflightTicketSaleUpgradeScript \
  --rpc-url "$RPC_URL"
```

The script logs:

- proxy address
- proxy admin address
- proxy admin owner
- implementation address
- whether `purchase_signer()` is available on the current implementation
- purchase signer when available

## 4. Post-upgrade Verification

Required env vars:

- `TICKET_SALE_PROXY`
- `PROXY_ADMIN`
- `EXPECTED_IMPLEMENTATION`
- `EXPECTED_DEFAULT_ADMIN`
- `EXPECTED_PAUSER`
- `EXPECTED_TREASURY`
- `EXPECTED_PURCHASE_SIGNER`
- `RPC_URL`

Run:

```bash
forge script script/VerifyTicketSaleUpgrade.s.sol:VerifyTicketSaleUpgradeScript \
  --rpc-url "$RPC_URL"
```

## 5. Storage Layout Guard

Update baseline (when intentionally accepting a storage change):

```bash
./scripts/update-storage-layout.sh
```

Check against baseline (for PR/CI gate):

```bash
./scripts/check-storage-layout.sh
```

## 6. Governance and Incident Runbooks

- Governance separation and role rotation:
  - `RUNBOOK_GOVERNANCE.md`
- Emergency pause and recovery:
  - `RUNBOOK_INCIDENT.md`
- BSC upgrade sequence for referral/discount releases:
  - `RUNBOOK_BSC_UPGRADE.md`
- Multisig parameter templates:
  - `MULTISIG_PARAMETER_TEMPLATES.md`
  - `templates/*.env.example`

## 7. CI Operational Jobs

Workflow:

- `.github/workflows/operations-checks.yml`

Jobs:

1. `deploy-verify`
2. `upgrade-verify`
3. `governance-rotation-checklist`

## 8. Deploy HakutoraSusdfVault (Transparent Proxy)

Required env vars:

- `VAULT_CONTRACT_NAME` (optional, defaults to `HakutoraSusdfVault.sol:HakutoraSusdfVault`)
- `PROXY_ADMIN_OWNER`
- `SUSDF_VAULT`
- `USDF_TOKEN`
- `SUSDF_TOKEN`
- `RPC_URL`
- `PRIVATE_KEY` or `ETH_FROM`/`MNEMONIC` via `BaseScript`

Run:

```bash
forge script script/Deploy.s.sol:DeployHakutoraSusdfVault \
  --rpc-url "$RPC_URL" \
  --private-key "$PRIVATE_KEY" \
  --broadcast
```
