# TicketSale Mainnet Deployments

## 2026-05-08

Ethereum mainnet and BSC mainnet were deployed to the same deterministic addresses:

- Implementation: `0x699c35637cfB2Bee805f192D93616Cc4F3AdA471`
- Proxy: `0x50263881d01887e1F1DAf82c43B516A8B9e260E9`
- ProxyAdmin: `0xC81EF5f391F50cdd319808ec168F9aa094ba5795`

Deployment output files record the script output at deployment time.
After deployment, `purchase_signer` was rotated on both chains to:

`0x11ad935078A0b6e4EB4EB483C9ac76de5707a918`

Backend production config must use the private key matching that current signer.
