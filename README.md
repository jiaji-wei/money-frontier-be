这是一个后端项目包含两个部分
# 1. 智能合约
提供售票功能，由 foundry 作为开发工具，Solidity 进行编码。
售票功能设计如下：
* 门票分为三个级别，每个级别是不同的售价
* 分时间段有不同的折扣价格
* 用户可以选择 usdt 或者 usdc 进行付款
* 单次可以购买多张门票

# 2. 后端服务
与智能合约相配合的后端服务
1. 对外提供 RESTful API 用于票务数据查询，另外门票提供转赠功能可以通过区块链地址或者邮箱进行转赠
2. 同步区块信息，将链上购票信息索引到 db 中，便于后续查询。
3. 门票与一个密钥进行绑定，作为会议入场时的唯一凭证，当进行转赠时当前密钥失效，获票方会收到新生成的密钥
API 列表：
* POST /signin
* GET /tickets
* POST /tickets
* GET /tickets/:id
* PUT /tickets/:id

登陆接口通过钱包签名进行鉴权，验证签名通过后给予 jwt token，后续API根据这个 token 鉴权，可对门票进行管理。

## 本地测试环境一键启动

推荐给前端同学的方式（只需要 Docker Desktop）：

```bash
docker compose up --build
```

停止并清理（包含本地链/数据库卷）：

```bash
docker compose down -v
```

`docker compose` 会完成：
* 启动本地 Anvil（`http://127.0.0.1:8545`）
* 自动部署本地 Mock USDT/USDC 与 TicketSale 透明代理
* 自动初始化价格配置并给默认买家授权
* 启动 backend（`http://127.0.0.1:8080`）

如果需要查看合约部署结果（容器共享运行时）：
* 使用 `docker volume` 查看 `ticket_runtime` 卷内容

已提供本地脚本（Anvil + 合约部署 + 后端启动）：

```bash
./scripts/dev-up.sh
```

脚本会完成：
* 启动本地 Anvil（`http://127.0.0.1:8545`）
* 部署本地 Mock USDT/USDC 与 TicketSale 透明代理
* 初始化 3 档票价配置（含分时价格）
* 给本地买家账户铸币并授权购票
* 启动 backend（`http://127.0.0.1:8080`）

停止环境：

```bash
./scripts/dev-down.sh
```

如果前端同学不安装 Rust，可使用预编译 backend（二进制）：

```bash
./scripts/dev-up-prebuilt.sh
./scripts/dev-down.sh
```

后端同学可在本机打包预编译 backend（按平台）：

```bash
./scripts/package-frontend-kit.sh
```

默认本地联调固定参数（前端可直接使用）：
* RPC: `http://127.0.0.1:8545`
* Chain ID: `31337`
* Backend: `http://127.0.0.1:8080`

本地测试钱包（仅限本地 Anvil）：
* Deployer: `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266`
* Buyer: `0x70997970C51812dc3A010C7d01b50e0d17dc79C8`

生成文件位于：
* `.dev/local/deploy-output.json`
* `.dev/local/backend.env`
* `.dev/local/anvil.log`
* `.dev/local/backend.log`

## 用户侧测试脚本

为了减少手工测试步骤，提供了用户操作脚本：

```bash
./scripts/user-flow.sh help
```

常用流程：

```bash
# 1) 登录并保存 JWT 到 .dev/local/user-session.json
./scripts/user-flow.sh signin

# 2) 购票（默认 usdt, level=1, quantity=1）
./scripts/user-flow.sh buy --token usdt --levels 1,2 --quantities 1,1

# 3) 通知后端同步该交易（默认使用上一步 tx hash）
./scripts/user-flow.sh notify

# 4) 查询当前钱包门票
./scripts/user-flow.sh list
```

一键串联流程（登录 + 购票 + 同步 + 查票）：

```bash
./scripts/user-flow.sh flow
```
