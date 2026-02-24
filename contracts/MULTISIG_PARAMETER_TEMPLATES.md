# Multisig Parameter Templates

## 1. Deploy Proposal

Template file:

- `templates/deploy.env.example`

Execution:

```bash
set -a
source templates/deploy.env.example
set +a

forge script script/TicketSale.s.sol:TicketSaleScript \
  --rpc-url "$RPC_URL" \
  --private-key "$PRIVATE_KEY" \
  --broadcast
```

Expected output artifact:

- `DEPLOY_OUTPUT_FILE` JSON with implementation/proxy/proxy_admin/roles

## 2. Upgrade Proposal

Template files:

- `templates/upgrade.env.example`
- `templates/verify-upgrade.env.example`

Execution:

```bash
set -a
source templates/upgrade.env.example
set +a

forge script script/UpgradeTicketSale.s.sol:UpgradeTicketSaleScript \
  --rpc-url "$RPC_URL" \
  --private-key "$PRIVATE_KEY" \
  --broadcast
```

Then verify:

```bash
set -a
source templates/verify-upgrade.env.example
set +a

forge script script/VerifyTicketSaleUpgrade.s.sol:VerifyTicketSaleUpgradeScript \
  --rpc-url "$RPC_URL"
```

## 3. Governance Rotation Proposal

Template file:

- `templates/rotate-governance.env.example`

Execution:

```bash
set -a
source templates/rotate-governance.env.example
set +a

forge script script/RotateGovernance.s.sol:RotateGovernanceScript \
  --rpc-url "$RPC_URL" \
  --private-key "$PRIVATE_KEY" \
  --broadcast
```

Expected output artifact:

- `ROTATE_OUTPUT_FILE` JSON with updated role holders and proxy admin owner

## 4. Incident Pause / Resume

Template file:

- `templates/incident.env.example`

Pause:

```bash
set -a
source templates/incident.env.example
set +a

forge script script/EmergencyPause.s.sol:EmergencyPauseScript \
  --rpc-url "$RPC_URL" \
  --private-key "$PRIVATE_KEY" \
  --broadcast
```

Resume:

```bash
set -a
source templates/incident.env.example
set +a

forge script script/ResumeOperations.s.sol:ResumeOperationsScript \
  --rpc-url "$RPC_URL" \
  --private-key "$PRIVATE_KEY" \
  --broadcast
```
