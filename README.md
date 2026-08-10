# qddxp — 虚拟商品商城

一个带库存管理与在线支付的虚拟商品（卡密类）售卖站点。顾客在线下单并完成支付后，系统自动从库存中分配商品卡密；支持下单时预留或支付时分配两种库存策略，库存不足时自动转为预售。

## 功能特性

**顾客端**
- 商品列表浏览，实时展示各商品剩余库存与已售数量
- 创建订单（须设置 6 位以上订单密码）
- 按订单号 + 订单密码查询订单详情
- 按联系方式查询历史订单列表
- 易支付（epay）网关与微信支付官方 API v3 Native 在线支付，支付后自动分配卡密并展示

**管理后台**（`/a-dmin`，凭 `ADMIN_KEY` 访问）
- 商品信息（SPU）管理：价格、开关状态
- 库存（卡密）管理：批量导入、状态调整、筛选分页
- 订单管理：列表、状态跟踪
- 库存分配模式切换：下单预留 / 支付时分配
- 支付回调 API 调用日志查询，便于排查支付异常

**订单与库存**
- 订单状态机：`pending` → `paid`（无库存时进入 `preorder` 预售）
- 库存状态机：`available` → `reserved` → `delivered`（另有 `disabled` 下架）
- 支付回调全链路校验：MD5 签名、商户 PID、金额与订单比对、重复通知幂等处理

## 技术栈

| 层 | 技术 |
| --- | --- |
| 后端 | Rust · axum · diesel (async) · PostgreSQL 18 · tower-http · tracing |
| 前端 | React 18 · TypeScript · Vite · Tailwind CSS |
| 支付 | 易支付 MD5 协议；微信支付官方 API v3 Native（RSA-SHA256 / AES-256-GCM） |
| 部署 | Docker 多阶段构建 · docker compose |

后端直接托管前端构建产物（`web/dist`），前后端同源部署。

## 目录结构

```
├── srv/                  # Rust 后端
│   ├── migrations/       # diesel 迁移
│   └── src/
│       ├── http/routes/  # API 路由（public / admin / epay）
│       ├── domain/       # 领域模型与状态机
│       └── db/           # 数据访问层
├── web/                  # React 前端
│   └── src/              # 顾客端 + 管理后台
├── deploy/
│   ├── compose.yml       # docker compose 部署
│   └── .env.example      # 环境变量示例
├── Dockerfile            # 多阶段构建（Rust + Node → 精简运行镜像）
└── Makefile              # 本地开发命令
```

## 本地开发

前置要求：Rust 工具链、Node.js、podman（或 docker）。

```bash
# 一键启动：启动 Postgres、构建前端、运行后端
make dev

# 或分开运行
make db-up && make srv      # 后端（http://localhost:3000）
cd web && npm install && npm run dev   # 前端开发服务器（http://localhost:5173，/api 代理到后端）
```

配置通过环境变量注入，开发默认值见 `Makefile`，完整变量见下表。

## Docker 部署

```bash
cp deploy/.env.example deploy/.env   # 按需修改（务必修改 ADMIN_KEY / ORDER_PASSWORD_PEPPER）
docker compose -f deploy/compose.yml up -d --build
```

服务启动后访问 `http://<主机>:8080`，健康检查 `GET /health`。

## 环境变量

| 变量 | 必填 | 说明 |
| --- | --- | --- |
| `DATABASE_URL` | 是 | PostgreSQL 连接串 |
| `WEB_DIST_DIR` | 是 | 前端静态文件目录 |
| `ADMIN_KEY` | 是 | 管理后台密钥（请求头 `x-admin-key`） |
| `LISTEN_ADDR` | 否 | 监听地址，默认 `0.0.0.0:3000` |
| `PUBLIC_BASE_URL` | 否 | 对外基础 URL，用于支付回调地址拼接 |
| `WEB_RETURN_URL` | 否 | 支付成功后的回跳页面 |
| `ORDER_PASSWORD_PEPPER` | 否 | 订单密码哈希 pepper，生产环境务必修改 |
| `PAYMENT_EXPIRE_MINUTES` | 否 | 支付订单有效分钟数，默认 15，范围 1–120 |
| `EPAY_GATEWAY` / `EPAY_PID` / `EPAY_KEY` | 否 | 三者都设置才启用易支付 |
| `WXPAY_APP_ID` / `WXPAY_MCH_ID` | 否 | 微信支付官方直连的应用 ID 与直连商户号 |
| `WXPAY_MERCHANT_SERIAL_NO` | 否 | 商户 API 证书序列号 |
| `WXPAY_MERCHANT_PRIVATE_KEY_PATH` | 否 | 商户 API 私钥 PEM 文件路径 |
| `WXPAY_API_V3_KEY` | 否 | 32 字节 APIv3 密钥，仅用于回调资源解密 |
| `WXPAY_PUBLIC_KEY_ID` / `WXPAY_PUBLIC_KEY_PATH` | 否 | 微信支付公钥 ID 与 PEM 文件路径，用于应答和回调验签 |
| `RUST_LOG` | 否 | 日志级别，默认 `info` |

微信支付的七项 `WXPAY_*` 配置必须同时设置，否则应用会拒绝启动。生产环境的
`PUBLIC_BASE_URL` 必须使用公网 HTTPS；回调地址固定生成为
`/api/payments/wechatpay/notify`。Docker 部署时将两个 PEM 文件放入
`deploy/secrets/`，该目录只读挂载且默认被 Git 忽略。

### 微信支付官方直连配置

本项目实现的是微信支付官方 API v3 的 Native 支付，与易支付的 `epay/wxpay`
参数是两套独立协议。商户平台需准备已绑定商户号的 AppID、商户 API 证书序列号、
证书对应私钥、APIv3 密钥，以及微信支付公钥 ID 和公钥文件。当前实现使用微信支付
公钥模式验签，不接受未验签的下单应答、查单应答或支付通知。

1. 将 `apiclient_key.pem` 和微信支付公钥放入 `deploy/secrets/`，文件不要提交到 Git。
2. 在 `deploy/.env` 中完整填写七项 `WXPAY_*` 配置，并把 `PUBLIC_BASE_URL` 改为
   可由微信服务器访问的 HTTPS 域名。
3. 确认反向代理允许 `POST /api/payments/wechatpay/notify` 直达应用，且不会改写请求体。
4. 重新构建并启动服务；前台支付方式列表出现“微信支付（官方）”即表示客户端初始化成功。

本地联调需要自行准备 HTTPS 隧道或测试域名，并将 `PUBLIC_BASE_URL` 指向该地址；
应用不会为开发机自动绕过 HTTPS。微信支付公钥轮换后，应同步替换公钥文件和
`WXPAY_PUBLIC_KEY_ID`，然后滚动重启服务。

- [微信支付 Native 下单](https://pay.weixin.qq.com/doc/v3/merchant/4012791877)
- [微信支付 Native 支付通知](https://pay.weixin.qq.com/doc/v3/merchant/4012791882)
- [微信支付 API v3 证书和密钥说明](https://pay.weixin.qq.com/doc/v3/merchant/4024350132)
- [微信支付 API v3 官方 SDK](https://github.com/wechatpay-apiv3)

## License

<!-- 待定：请补充许可证（如 MIT / Apache-2.0） -->
