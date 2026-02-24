# Lili Ticket Frontend Quickstart

本文档是给前端同学的精简接入版，目标是快速完成可用联调。

如果需要完整业务背景、边界条件和详细说明，请看：

- `backend/docs/frontend-integration-guide.md`
- `backend/docs/openapi.yaml`

## 1. 你需要接什么（最小闭环）

前端最小可用流程：

1. 钱包登录（签名换 JWT）
2. 调用合约 `purchase(...)` 发起购票交易
3. 用 `chain_id + tx_hash` 调后端 `POST /tickets` 同步票务
4. 调后端 `GET /tickets` 展示票夹
5. （可选）转赠 `PUT /tickets/:id`

## 2. 核心认知（避免走偏）

- 门票不是链上 NFT，**以后端票务记录为准**
- 链上只负责：
  - 收款
  - 价格规则
  - 发 `TicketsPurchased` 事件
- 链下后端负责：
  - 票归属（钱包 / 邮箱）
  - 二维码 payload
  - 转赠后二维码轮换

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
  "signature": "0x..."
}
```

返回：

- `token`（JWT）
- `wallet`
- `expires_at`

后续所有票务 API 请求都要带：

```http
Authorization: Bearer <jwt>
```

## 5. 购票（链上）

### 5.1 建议调用顺序

1. `quote(level_ids, quantities)`（可选但强烈建议）
2. 检查 token allowance
3. `approve`（如果不足）
4. `purchase(payment_token, level_ids, quantities)`
5. 等待交易成功 receipt
6. 调后端 `POST /tickets` 同步
7. `GET /tickets` 刷新票夹

### 5.2 推荐接入的合约方法

只读：

- `quote(uint8[] level_ids, uint256[] quantities)`
- `currentPrice(uint8 level_id)`
- `getPriceSchedule(uint8 level_id)`（如果要展示时间段价格）

写入：

- `purchase(address payment_token, uint8[] level_ids, uint256[] quantities)`

### 5.3 精度说明

- USDT / USDC 通常是 `6 decimals`
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
- 解码 `TicketsPurchased` 事件
- 写入 `orders` / `tickets`

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

### 9.1 `POST /tickets` 返回 403，但票已经能查到

这在当前实现里可能发生，原因通常是：

- 后台 indexer 已经先把这笔交易索引入库了
- 你再调用 `POST /tickets` 时没有新增结果

前端处理建议（推荐）：

- `POST /tickets` 后无论成功/失败，都执行一次 `GET /tickets`
- 如果票已经出现，就按“同步成功”处理 UI

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
});

jwtStore.set(signin.token);
```

### 12.2 交易后同步

```ts
const txHash = await writeContractAsync({
  address: saleProxyAddress,
  abi: ticketSaleAbi,
  functionName: "purchase",
  args: [paymentToken, levelIds, quantities],
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
