# TicketSale Contract Operations

## 1. Deploy (Transparent Proxy)

Required env vars:

- `OWNER`
- `PAUSER` (optional, defaults to `OWNER`)
- `PROXY_ADMIN_OWNER` (optional, defaults to `OWNER`)
- `TREASURY`
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
- token addresses

## 2. Upgrade

Required env vars:

- `TICKET_SALE_PROXY`
- `PRIVATE_KEY`
- `RPC_URL`
- `PROXY_ADMIN` (optional safety check)
- `NEW_IMPLEMENTATION` (optional; if omitted, script deploys a new `TicketSale`)

Run:

```bash
forge script script/UpgradeTicketSale.s.sol:UpgradeTicketSaleScript \
  --rpc-url "$RPC_URL" \
  --private-key "$PRIVATE_KEY" \
  --broadcast
```

## 3. Post-upgrade Verification

Required env vars:

- `TICKET_SALE_PROXY`
- `PROXY_ADMIN`
- `EXPECTED_IMPLEMENTATION`
- `EXPECTED_DEFAULT_ADMIN`
- `EXPECTED_PAUSER`
- `EXPECTED_TREASURY`
- `RPC_URL`

Run:

```bash
forge script script/VerifyTicketSaleUpgrade.s.sol:VerifyTicketSaleUpgradeScript \
  --rpc-url "$RPC_URL"
```

## 4. Storage Layout Guard

Update baseline (when intentionally accepting a storage change):

```bash
./scripts/update-storage-layout.sh
```

Check against baseline (for PR/CI gate):

```bash
./scripts/check-storage-layout.sh
```

## 5. Governance and Incident Runbooks

- Governance separation and role rotation:
  - `RUNBOOK_GOVERNANCE.md`
- Emergency pause and recovery:
  - `RUNBOOK_INCIDENT.md`
- Multisig parameter templates:
  - `MULTISIG_PARAMETER_TEMPLATES.md`
  - `templates/*.env.example`

## 6. CI Operational Jobs

Workflow:

- `.github/workflows/operations-checks.yml`

Jobs:

1. `deploy-verify`
2. `upgrade-verify`
3. `governance-rotation-checklist`
