# Governance and Role Separation

## 1. Role Model

Use three independent multisig groups:

1. `ProxyAdmin owner`:
   - authority: upgrade implementation, transfer proxy admin ownership
   - contract: `ProxyAdmin`
2. `DEFAULT_ADMIN_ROLE`:
   - authority: business configuration (`setPriceSchedule`, `setPaymentToken`, `setTreasury`), role grant/revoke
   - contract: `TicketSale` (proxy address)
3. `PAUSER_ROLE`:
   - authority: `pause` / `unpause` only
   - contract: `TicketSale` (proxy address)

Recommended default:

- `ProxyAdmin owner` != `DEFAULT_ADMIN_ROLE` != `PAUSER_ROLE`
- all three are multisig wallets

## 2. Rotation Strategy

Rotate in two phases:

1. grant new authorities
2. revoke old authorities

Never revoke old authority before new one is confirmed active.

## 3. Rotation Execution

Required env vars:

- `TICKET_SALE_PROXY`
- `PROXY_ADMIN`
- `NEW_DEFAULT_ADMIN`
- `NEW_PAUSER`
- `NEW_PROXY_ADMIN_OWNER` (optional)
- `OLD_DEFAULT_ADMIN` (optional)
- `OLD_PAUSER` (optional)
- `PRIVATE_KEY`
- `RPC_URL`

Run:

```bash
forge script script/RotateGovernance.s.sol:RotateGovernanceScript \
  --rpc-url "$RPC_URL" \
  --private-key "$PRIVATE_KEY" \
  --broadcast
```

## 4. Post-rotation Checklist

1. `ProxyAdmin.owner()` equals expected owner.
2. `hasRole(DEFAULT_ADMIN_ROLE, NEW_DEFAULT_ADMIN)` is true.
3. `hasRole(PAUSER_ROLE, NEW_PAUSER)` is true.
4. old role holders are revoked when expected.
5. perform one dry-run config update on testnet and revert to baseline.
