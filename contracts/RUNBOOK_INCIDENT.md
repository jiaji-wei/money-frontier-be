# Incident Runbook (Pause / Recovery)

## 1. Trigger Conditions

Execute emergency pause when one of the following is detected:

1. exploitable logic bug in purchase or payment path
2. abnormal fund movement or unauthorized config change
3. severe dependency incident that impacts settlement correctness

## 2. Emergency Pause

Required env vars:

- `TICKET_SALE_PROXY`
- `PRIVATE_KEY` (must belong to an account with `PAUSER_ROLE`)
- `RPC_URL`

Run:

```bash
forge script script/EmergencyPause.s.sol:EmergencyPauseScript \
  --rpc-url "$RPC_URL" \
  --private-key "$PRIVATE_KEY" \
  --broadcast
```

Immediate checks:

1. `paused == true` from script output
2. attempt purchase call on test endpoint, expect revert
3. notify backend and operations team that sales are halted

## 3. Root Cause and Fix

1. identify root cause and impact window
2. prepare fix (config change or implementation upgrade)
3. run full regression on local + testnet
4. if upgraded, run `VerifyTicketSaleUpgradeScript`

## 4. Resume Operations

Prerequisites:

1. root cause fixed
2. regression checks completed
3. risk owner approves resume

Run:

```bash
forge script script/ResumeOperations.s.sol:ResumeOperationsScript \
  --rpc-url "$RPC_URL" \
  --private-key "$PRIVATE_KEY" \
  --broadcast
```

Post-resume checks:

1. `paused == false`
2. one controlled purchase succeeds with expected amount/event
3. indexer and backend ingest the event correctly
