# Money Frontier Ticket Frontend Quickstart

本文档是给前端同学的精简接入版，目标是快速完成可用联调。

如果需要完整业务背景、边界条件和详细说明，请看：

- `backend/docs/frontend-integration-guide.md`
- `backend/docs/openapi.yaml`

## 1. 你需要接什么（最小闭环）

前端最小可用流程：

1. 捕获落地页 `?ref=CODE` 并暂存 referral code
2. 钱包登录（签名换 JWT，可选带 `referral_code` 做首次绑定）
3. 调后端 `POST /purchase-intents` 锁定报价，拿 `intent_id + signature`
4. 检查 token allowance，不足则先 `approve`
5. 调用合约 `purchaseWithAuthorization(...)` 发起购票交易
6. 用 `chain_id + tx_hash` 调后端 `POST /tickets` 同步票务
7. 调后端 `GET /tickets` 展示票夹
8. （可选）转赠 `PUT /tickets/:id`

## 2. 核心认知（避免走偏）

- 门票不是链上 NFT，**以后端票务记录为准**
- 链上只负责：
  - 收款
  - 验证后端签发的购买授权
  - 发购票事件（新流程为 `TicketsPurchasedWithIntent`）
- 链下后端负责：
  - 票归属（钱包 / 邮箱）
  - 二维码 payload
  - 转赠后二维码轮换
- referral 不是在 `POST /tickets` 里事后绑定，而是在首次登录或首次创建 intent 时绑定到钱包
- discount 会影响实付金额，必须在购买前通过 `POST /purchase-intents` 锁定，不能等链上交易完成后再补

## 3. 前端接入所需配置

前端至少需要这些配置（按链）：

- `chainId`
- `TicketSale proxy address`
- `TicketSale ABI`（使用 `TicketSale.sol` ABI）
- `USDT token address`
- `USDC token address`
- `Backend API base URL`

说明：

- 当前使用 **Transparent Proxy**
- 前端调用目标地址是 **proxy address**
- ABI 用实现合约 `TicketSale` 的 ABI

## 4. 登录（Wallet Sign-In -> JWT）

### Step 1: 获取 challenge

`POST /signin/challenge`

```json
{
  "address": "0x..."
}
```

返回：

- `challenge_id`
- `challenge_message`
- `expires_at`

### Step 2: 钱包签名后换 JWT

签名方式：

- `personal_sign`（EIP-191 personal message）

`POST /signin`

```json
{
  "address": "0x...",
  "challenge_id": "uuid",
  "signature": "0x...",
  "referral_code": "PARTNERX"
}
```

返回：

- `token`（JWT）
- `wallet`
- `expires_at`
- `referral_binding`（可选，状态为 `bound | already_bound | invalid`）

后续所有票务 API 请求都要带：

```http
Authorization: Bearer <jwt>
```

## 5. 购票（授权购买）

### 5.1 建议调用顺序

1. 可选捕获 URL 里的 `referral_code`
2. `POST /signin` 签发 JWT，并在 wallet 尚未绑定 referral 时尝试首次绑定
3. `POST /purchase-intents`
4. 检查 token allowance
5. `approve`（如果不足）
6. `purchaseWithAuthorization(payment_token, level_ids, quantities, intent_id, final_total_amount, expires_at, signature)`
7. 等待交易成功 receipt
8. 调后端 `POST /tickets` 同步
9. `GET /tickets` 刷新票夹

### 5.2 `POST /purchase-intents`

请求：

```json
{
  "chain_id": 56,
  "payment_token": "0x...",
  "level_ids": [1],
  "quantities": [2],
  "discount_code": "SAVE2345",
  "referral_code": "PARTNERX"
}
```

说明：

- `discount_code` 可选，用于锁定本次折扣报价
- `referral_code` 可选，只在 wallet 尚未绑定 referral 时生效
- 已绑定 wallet 再传 `referral_code` 会被忽略

响应：

```json
{
  "intent_id": "0x...",
  "expires_at": 1760000000,
  "original_total_amount": "200000000",
  "discount_amount": "50000000",
  "final_total_amount": "150000000",
  "signature": "0x...",
  "referral_binding_status": "bound"
}
```

说明：

- `intent_id` 要原样传给合约
- `signature` 是后端对本次购买授权的签名
- `final_total_amount` 是本次实际应付金额
- `referral_binding_status` 仅在本次请求顺带处理 referral 首绑时返回

### 5.3 推荐接入的合约方法

只读：

- `quote(uint8[] level_ids, uint256[] quantities)`
- `currentPrice(uint8 level_id)`
- `getPriceSchedule(uint8 level_id)`（如果要展示时间段价格）

前端展示当前票价时优先调用后端 `POST /purchase-prices`，后端会通过合约 `quote(level_ids, [1...])` 返回链上实时单价。

写入：

- `purchaseWithAuthorization(address payment_token, uint8[] level_ids, uint256[] quantities, bytes32 intent_id, uint256 final_total_amount, uint64 expires_at, bytes signature)`

前端不要再直接调用旧的 `purchase(...)` 路径做正式购买；新流程需要 `intent_id + signature` 才能带 discount 并与后端安全对账。

### 5.4 精度说明

- USDT / USDC 通常是 `6 decimals`
- 当前主仓 `contracts/script/LocalSetup.s.sol` 部署的 mock USDT / USDC 是 `18 decimals`，默认票价按 `e18` 配置
- 合约与后端返回金额一般是整数（最小单位）
- 前端展示时自己做格式化

## 6. 交易后通知后端同步（非常关键）

交易成功后，前端需要调用：

`POST /tickets`

```json
{
  "chain_id": 11155111,
  "tx_hash": "0x..."
}
```

用途：

- 后端根据 `tx_hash` 读取链上 receipt
- 解码 `TicketsPurchasedWithIntent` / `TicketsPurchased` 事件
- 写入 `orders` / `tickets`
- 确认 discount redemption
- 写 referral / discount 快照

成功响应示例：

```json
{
  "indexed_orders": 1,
  "created_tickets": 2
}
```

## 7. 票夹与详情

### 7.1 查询票夹

`GET /tickets`

返回当前钱包的有效票（`active`）。

关键字段：

- `id`：后端票 ID（UUID，不是 tokenId）
- `order_id`：链上订单号（事件里的 `order_id`）
- `ticket_level`
- `unit_price`
- `owner_wallet` / `owner_email`
- `qr_payload`
- `qr_version`

### 7.2 查询单票

`GET /tickets/:id`

## 8. 转赠（链下）

`PUT /tickets/:id`

请求体约束：

- `to_wallet` 和 `to_email` 二选一

示例（转钱包）：

```json
{
  "to_wallet": "0x..."
}
```

示例（转邮箱）：

```json
{
  "to_email": "receiver@example.com"
}
```

成功后返回“新票对象”（新 `id`、新 `qr_payload`）。

## 9. 前端最容易踩的坑（请先看）

### 9.1 `POST /tickets` 重复调用不会重复记账

当前实现对同一笔交易的重复 notify 是幂等的：

- 首次成功时通常返回：
  - `indexed_orders = 1`
  - `created_tickets >= 1`
- 重复调用同一笔交易时通常返回：
  - `indexed_orders = 0`
  - `created_tickets = 0`

前端仍然建议在 `POST /tickets` 后执行一次 `GET /tickets`，因为 UI 最终应以后端票夹结果为准。

### 9.2 登录钱包和购票钱包不一致

会导致 `POST /tickets` 返回 `403`：

- JWT 的钱包地址 ≠ 事件里的 `buyer`

前端在下单前可以显示当前登录地址，避免用户切钱包后误操作。

### 9.3 测试网有确认数延迟

若后端配置了 `confirmations > 0`：

- 票不会立即出现
- 需要轮询 `GET /tickets`

## 10. 建议的前端状态机（最小版）

购票按钮建议状态：

- `idle`
- `signing_in`
- `creating_intent`
- `approving`
- `purchasing`
- `waiting_receipt`
- `notifying_backend`
- `refreshing_tickets`
- `done`
- `failed`

这样可以清晰表达“链上成功但后端同步中”的状态。

## 11. 联调命令（后端同学已提供）

### 11.1 本地联调环境（推荐：Docker Compose，不需要 Rust / Foundry）

前提（前端机器）：

- 已安装 Docker Desktop（或 Docker Engine + Compose）

启动：

```bash
docker compose up --build
```

停止并清理（重置本地链与数据库）：

```bash
docker compose down -v
```

说明：

- 会自动启动 Anvil、本地合约部署初始化、backend
- 合约部署结果会写到宿主机：`.dev/docker/deploy-output.json`
- 前端直接使用固定参数：
  - RPC URL: `http://127.0.0.1:8545`
  - Chain ID: `31337`
  - Backend API: `http://127.0.0.1:8080`

前端读取地址（推荐）：

```bash
jq -r '.proxy' .dev/docker/deploy-output.json
jq -r '.usdt' .dev/docker/deploy-output.json
jq -r '.usdc' .dev/docker/deploy-output.json
```

### 11.2 本地联调环境（源码模式，需要 Rust）

```bash
./scripts/dev-up.sh
./scripts/dev-down.sh
```

### 11.3 本地联调环境（预编译 backend，不需要 Rust）

前提（前端机器）：

- 已安装 Foundry（`anvil` / `forge` / `cast`）
- 已安装 `curl`
- 已拿到后端同学交付的预编译 binary（按你的系统架构）

启动命令（推荐）：

```bash
./scripts/dev-up-prebuilt.sh
./scripts/dev-down.sh
```

如果 binary 不在默认位置，也可以显式指定：

```bash
BACKEND_BIN=./dist/prebuilt/<platform>/ticket-backend ./scripts/dev-up.sh
```

### 11.4 本地固定联调参数（前端可直接使用）

- RPC URL: `http://127.0.0.1:8545`
- Chain ID: `31337`
- Backend API: `http://127.0.0.1:8080`

本地测试钱包（仅限本地 Anvil，禁止用于测试网/主网）：

- Deployer
  - Address: `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266`
  - Private Key: `0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80`
- Buyer（脚本默认购票钱包）
  - Address: `0x70997970C51812dc3A010C7d01b50e0d17dc79C8`
  - Private Key: `0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d`

用户流程测试脚本（可模拟真实用户操作）：

```bash
./scripts/user-flow.sh help
./scripts/user-flow.sh flow
./scripts/user-flow.sh signin
./scripts/user-flow.sh buy --token usdt --levels 1,2 --quantities 1,1
./scripts/user-flow.sh notify
./scripts/user-flow.sh list
```

## 12. 前端伪代码（最小版）

### 12.1 登录

```ts
const challenge = await api.post("/signin/challenge", { address });
const signature = await walletClient.signMessage({
  account: address,
  message: challenge.challenge_message,
});

const signin = await api.post("/signin", {
  address,
  challenge_id: challenge.challenge_id,
  signature,
  referral_code: capturedReferralCode,
});

jwtStore.set(signin.token);
```

### 12.2 交易后同步

```ts
const intent = await api.post(
  "/purchase-intents",
  {
    chain_id: chainId,
    payment_token: paymentToken,
    level_ids: levelIds,
    quantities,
    discount_code: discountCode,
    referral_code: capturedReferralCode,
  },
  { headers: { Authorization: `Bearer ${jwtStore.token}` } }
);

if (allowance < BigInt(intent.final_total_amount)) {
  await writeContractAsync({
    address: paymentToken,
    abi: erc20Abi,
    functionName: "approve",
    args: [saleProxyAddress, BigInt(intent.final_total_amount)],
  });
}

const txHash = await writeContractAsync({
  address: saleProxyAddress,
  abi: ticketSaleAbi,
  functionName: "purchaseWithAuthorization",
  args: [
    paymentToken,
    levelIds,
    quantities,
    intent.intent_id,
    BigInt(intent.final_total_amount),
    BigInt(intent.expires_at),
    intent.signature,
  ],
});

await waitForTransactionReceipt({ hash: txHash });

try {
  await api.post(
    "/tickets",
    { chain_id: chainId, tx_hash: txHash },
    { headers: { Authorization: `Bearer ${jwtStore.token}` } }
  );
} finally {
  await refetchTickets();
}
```
