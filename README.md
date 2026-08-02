# qddxp — 虚拟商品商城

一个带库存管理与在线支付的虚拟商品（卡密类）售卖站点。顾客在线下单并完成支付后，系统自动从库存中分配商品卡密；支持下单时预留或支付时分配两种库存策略，库存不足时自动转为预售。

## 功能特性

**顾客端**
- 商品列表浏览，实时展示各商品剩余库存与已售数量
- 创建订单（须设置 6 位以上订单密码）
- 按订单号 + 订单密码查询订单详情
- 按联系方式查询历史订单列表
- 易支付（epay）网关在线支付，支付后自动分配卡密并展示

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
| 后端 | Rust · axum · diesel (async) · PostgreSQL 16 · tower-http · tracing |
| 前端 | React 18 · TypeScript · Vite · Tailwind CSS |
| 支付 | 易支付（epay）网关，MD5 签名 |
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
| `EPAY_GATEWAY` / `EPAY_PID` / `EPAY_KEY` | 否 | 三者都设置才启用易支付 |
| `RUST_LOG` | 否 | 日志级别，默认 `info` |

## License

<!-- 待定：请补充许可证（如 MIT / Apache-2.0） -->
