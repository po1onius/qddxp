# qddxp — 虚拟商品商城

一个带库存管理与在线支付的虚拟商品（卡密类）售卖站点。顾客下单时系统立即预留真实库存，完成支付后自动交付卡密；库存不足时拒绝创建订单。

## 功能特性

**顾客端**
- 商品列表浏览，实时展示各商品剩余库存与已售数量
- 创建订单（须设置 6 位以上订单密码）
- 按订单号 + 订单密码查询订单详情
- 按联系方式查询历史订单列表
- 易支付（epay）网关与微信支付官方 API v3 Native 在线支付，支付后自动分配卡密并展示

**管理后台**（`/admin`，使用 `ADMIN_KEY` 登录）
- 商品信息（SPU）管理：图片、唯一名称、详情、价格、开关状态
- 库存（卡密）管理：批量导入、状态调整、筛选分页；发货内容仅显示末尾 4 个字符
- 订单管理：列表、状态跟踪；已分配的发货内容仅显示末尾 4 个字符
- 支付回调 API 调用日志查询，便于排查支付异常

**订单与库存**
- 下单时必须预留真实库存；缺货时拒绝创建订单，不接受预购
- 订单状态机只表达库存与交付结果：`pending` → `delivered`；超时未支付进入 `expired`
- 支付事实由支付尝试单独记录：`state=succeeded` 与 `paid_at` 表示已经收款；`expired + succeeded` 表示超时到账但未交付
- ePay 固定预留库存 3 分钟；超时后到账只记录支付事实并提示联系管理员，不自动发货
- ePay 支付页面回跳后会归一化进入 `/orders?order_id=<订单号>`，与微信 Native 支付完成后的订单查询地址一致
- 库存状态机：`available` → `reserved` → `delivered`（另有 `disabled` 下架）
- 支付回调全链路校验：MD5 签名、商户 PID、金额与订单比对、重复通知幂等处理

## 技术栈

| 层 | 技术 |
| --- | --- |
| 后端 | Rust · axum · diesel (async) · Percona PostgreSQL 18.4 / pgBackRest · tower-http · tracing |
| 前端 | React 18 · React Router 7 · TypeScript · Vite · Tailwind CSS |
| 支付 | 易支付 MD5 协议；微信支付官方 API v3 Native（RSA-SHA256 / AES-256-GCM） |
| 部署 | Docker 多阶段构建 · docker compose |

后端直接托管前端构建产物（`web/dist`），前后端同源部署。

顾客端页面使用独立前端路由：商城 `/`、创建订单 `/orders/new/:productId`、订单查询 `/orders`；管理后台使用 `/admin`。管理员密钥只在登录时提交，登录成功后通过服务端会话和 `HttpOnly` Cookie 认证管理 API。

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
│   ├── compose.yml       # docker compose 部署（本地开发数据库也由它启动）
│   ├── assets/           # 示例店铺 Logo
│   ├── infra/pgbackrest/ # PostgreSQL 备份、WAL 归档与恢复脚本
│   ├── secrets/          # 微信支付 PEM 文件（默认被 Git 忽略）
│   └── .env.example      # 环境变量示例
├── Dockerfile            # 多阶段构建（Rust + Node → 精简运行镜像）
└── Makefile              # 本地开发命令
```

## 本地开发

前置要求：Rust 工具链、Node.js、podman（或 docker）。

```bash
# 一键启动：podman compose 启动本地 Postgres、构建前端、运行后端
make SHOP_NAME='我的店铺' SHOP_LOGO_FILE="$PWD/deploy/assets/shop-logo.svg"

# 前端开发服务器（http://localhost:5173，/api 代理到后端）
cd web && npm install && npm run dev
```

`make` 只启动 `deploy/compose.yml` 中的 `db` 服务（不启动备份），项目名为 `qddxp-dev`，
数据落在 `qddxp-dev_*` 命名卷中，与生产部署的 `deploy_*` 卷相互隔离。停止本地数据库：
`podman stop qddxp-dev-postgres`（数据保留，下次 `make` 会自动重新启动）。
```

配置通过环境变量注入，开发默认值见 `Makefile`，完整变量见下表。

## Docker 部署

```bash
cp deploy/.env.example deploy/.env   # 按需修改（务必修改 ADMIN_KEY / ORDER_PASSWORD_PEPPER）
docker compose -f deploy/compose.yml up -d --build
```

服务启动后访问 `http://<主机>:8080`，健康检查 `GET /health`。

数据库同时将 5432 映射到宿主机回环地址（`POSTGRES_PORT` 可覆盖），便于宿主机上的
排查与备份工具直连，也供本地开发（`make`）使用。

### 应用日志

应用日志会同时输出到容器控制台和日志文件。文件按 UTC 日期每日滚动，命名格式为
`qddxp.YYYY-MM-DD.log`，并保存在 `qddxp_logs` Compose 命名卷的
`/var/log/qddxp` 目录中；正常重建容器不会删除该卷。查看当前文件或持续跟踪当天日志：

```bash
docker compose -f deploy/compose.yml exec qddxp ls -lh /var/log/qddxp
docker compose -f deploy/compose.yml exec qddxp \
  tail -f /var/log/qddxp/qddxp.$(date -u +%F).log
```

当前不会自动删除历史应用日志，生产环境应根据磁盘容量制定保留和归档策略。执行
`docker compose down -v` 会连同数据库、备份仓库和日志命名卷一起删除，不应在生产
环境使用。

### PostgreSQL 备份

Compose 使用同时包含 PostgreSQL 18.4 和 pgBackRest 的 Percona 发行镜像。正常执行
`docker compose up` 时会一起启动 `pgbackrest-backup`：全新仓库立即建立第一份 full
备份，之后按 UTC 时间执行以下调度：

- 每周日 03:00 后执行一次 full；
- 其余每天 03:00 后执行一次 diff；
- PostgreSQL 持续通过 `archive_command` 归档 WAL；低写入场景也至少每 60 秒尝试
  切换一次 WAL。

调度器每五分钟检查一次，容器在计划时刻停机时会在恢复运行后补做当前周期缺失的
备份。`.env` 中的 `PGBR_SCHEDULE_HOUR_UTC` 和
`PGBR_CHECK_INTERVAL_SECONDS` 可以调整计划小时和检查间隔。仓库默认保留最近四个
full 及其关联的 diff 与恢复所需 WAL，当前不加密。

查看备份日志、检查仓库和列出备份：

```bash
docker compose -f deploy/compose.yml logs -f pgbackrest-backup
docker compose -f deploy/compose.yml exec pgbackrest-backup \
  pgbackrest --stanza=qddxp check
docker compose -f deploy/compose.yml exec pgbackrest-backup \
  pgbackrest --stanza=qddxp info
```

备份容器只读挂载在线数据卷，并通过共享 Unix Socket 执行 PostgreSQL 控制查询；数据库
容器负责 WAL 归档，备份容器负责基础备份。两者使用同一个 Percona 镜像，避免
PostgreSQL 与 pgBackRest 版本漂移。

本次从官方 `postgres:18.4-alpine` 切换到 Percona 镜像后，PostgreSQL 主版本与
版本化数据目录结构保持不变，但容器内 PostgreSQL 用户的 UID/GID 可能不同。已有
`qddxp_db_data` 如果在首次启动时报 `Permission denied`，应停止服务并手工调整准确
卷内集群目录的属主；不要修改代码或让数据库以 root 身份运行来绕过权限：

```bash
docker compose -f deploy/compose.yml down
docker volume ls

# 将占位符替换为上一步核对出的 qddxp_db_data 实际卷名后再执行。
docker run --rm --user root \
  -v <核对无误的_qddxp_db_data_实际卷名>:/data/db \
  percona/percona-distribution-postgresql:18.4-5 \
  chown -R 26:26 /data/db/18/docker

docker compose -f deploy/compose.yml up -d --build
```

调整属主不会转换 PostgreSQL 主版本，也不能替代备份；操作已有数据卷前应先确认数据
已经另行保全。

> `qddxp_pgbackrest_repository` 默认仍是同一容器主机上的 Compose 命名卷，只能应对
> 数据库逻辑损坏或数据卷损坏，不能抵御整机磁盘丢失。正式部署应将仓库迁移到独立
> 持久磁盘，或改用 pgBackRest 支持的对象存储仓库。

### PostgreSQL 备份恢复

`pgbackrest-restore` 位于 `restore` profile，日常启动不会运行。它是停机灾难恢复的
一次性任务：只读访问备份仓库，并将物理文件恢复到正式 `qddxp_db_data`。脚本不会
替运维人员停止或启动服务。

恢复前先停止应用、备份调度器和数据库。恢复脚本还会通过 Compose 内部网络检查
`db:5432`；只要 PostgreSQL 正在接受连接或处于启动、停止、恢复过程，就会拒绝操作：

```bash
docker compose -f deploy/compose.yml stop qddxp pgbackrest-backup db
```

恢复脚本要求正式 PGDATA 是空目录，不会自动删除、清空或覆盖故障现场。先核对准确
卷名，再把原数据复制到调查目录或外部存储。只有确认不再需要原 PGDATA 后才能删除
对应卷。以下是开发阶段重建数据卷的示例，执行会永久删除原数据库，但不得删除
`qddxp_pgbackrest_repository` 仓库卷：

```bash
# 停止的容器仍会占用数据卷，因此删除卷前先移除数据库与备份容器。
docker compose -f deploy/compose.yml rm -f pgbackrest-backup
docker compose -f deploy/compose.yml rm -f db
docker volume ls

# 实际名称受 Compose 项目名影响，常见为 deploy_qddxp_db_data；必须核对后手动执行。
docker volume rm <核对无误的_qddxp_db_data_实际卷名>
```

恢复最新备份的物理文件：

```bash
docker compose -f deploy/compose.yml --profile restore run --rm pgbackrest-restore
```

按时间做 PITR 时，目标时间必须包含时区：

```bash
PGBR_RESTORE_TYPE=time \
PGBR_RESTORE_TARGET='2026-08-12 03:15:00+00' \
docker compose -f deploy/compose.yml --profile restore run --rm pgbackrest-restore
```

也可以通过 `PGBR_RESTORE_SET` 指定 `pgbackrest info` 中显示的真实备份标签，或将
`PGBR_RESTORE_TYPE` 设置为 `lsn` 并提供目标 LSN。默认的 `latest` 表示不传
`--set`，由 pgBackRest 选择最新可用备份。

`pgbackrest restore` 完成后只恢复了基础文件和恢复配置，WAL 尚未由 PostgreSQL 完整
回放。随后单独启动数据库并观察日志；确认 archive recovery 完成且数据库可以正常
读写后，再启动备份调度器和应用：

```bash
docker compose -f deploy/compose.yml up -d db
docker compose -f deploy/compose.yml logs -f db

# 另开终端确认恢复结束；结果应为 f。
docker compose -f deploy/compose.yml exec db sh -c \
  'psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -c "select pg_is_in_recovery();"'

docker compose -f deploy/compose.yml up -d pgbackrest-backup qddxp
```

如果 PostgreSQL 启动或 WAL 回放失败，不要启动应用，也不要再次覆盖当前 PGDATA；应
保留日志和故障现场，排查备份仓库、恢复目标与 WAL 连续性。

## 环境变量

| 变量 | 必填 | 说明 |
| --- | --- | --- |
| `DATABASE_URL` | 是 | PostgreSQL 连接串 |
| `WEB_DIST_DIR` | 是 | 前端静态文件目录 |
| `ADMIN_KEY` | 是 | 管理后台登录密钥；仅在登录时提交，不保存在浏览器存储中 |
| `LISTEN_ADDR` | 否 | 监听地址，默认 `0.0.0.0:3000` |
| `PUBLIC_BASE_URL` | 否 | 对外基础 URL，用于支付回调地址拼接 |
| `WEB_RETURN_URL` | 否 | 支付成功后的回跳页面，应指向前端订单查询路由 `/orders` |
| `SHOP_NAME` | 是 | 店铺名称，1–100 个字符 |
| `SHOP_LOGO_FILE` | 是 | SVG Logo 文件，启动时校验真实内容；Docker 部署时填写宿主机文件路径 |
| `ORDER_PASSWORD_PEPPER` | 否 | 订单密码哈希 pepper，生产环境务必修改 |
| `RATE_LIMIT_TRUSTED_PROXY_CIDRS` | 否 | 允许提供 `X-Forwarded-For` 的反向代理 CIDR，多个值用逗号分隔；直连部署留空 |
| `TELEGRAM_BOT_TOKEN` / `TELEGRAM_NOTIFY_CHAT_ID` | 否 | 两项同时设置后启用下单与付款单次通知；发送失败只记日志，不重试 |
| `PGBR_SCHEDULE_HOUR_UTC` | 否 | pgBackRest 每日计划小时（UTC），默认 `3` |
| `PGBR_CHECK_INTERVAL_SECONDS` | 否 | pgBackRest 调度检查间隔秒数，默认 `300` |
| `WXPAY_EXPIRE_MINUTES` | 否 | 微信官方 Native 支付结束分钟数，默认 15，范围 1–120；ePay 固定为 3 分钟 |
| `EPAY_GATEWAY` / `EPAY_PID` / `EPAY_KEY` | 否 | 三者都设置才启用易支付 |
| `WXPAY_APP_ID` / `WXPAY_MCH_ID` | 否 | 微信支付官方直连的应用 ID 与直连商户号 |
| `WXPAY_MERCHANT_SERIAL_NO` | 否 | 商户 API 证书序列号 |
| `WXPAY_MERCHANT_PRIVATE_KEY_FILE` | 否 | 商户 API 私钥 PEM 文件；Docker 部署时填写宿主机文件路径 |
| `WXPAY_API_V3_KEY` | 否 | 32 字节 APIv3 密钥，仅用于回调资源解密 |
| `WXPAY_PUBLIC_KEY_ID` / `WXPAY_PUBLIC_KEY_FILE` | 否 | 微信支付公钥 ID 与 PEM 文件，用于应答和回调验签；Docker 部署时文件参数填写宿主机路径 |
| `RUST_LOG` | 否 | 日志级别，默认 `info` |
| `LOG_DIR` | 否 | 应用日志目录；本地默认 `./logs`，容器内固定为 `/var/log/qddxp` |

启用微信支付时，五项业务凭据与两个 PEM 文件必须同时提供，否则应用会拒绝启动。
生产环境的 `PUBLIC_BASE_URL` 必须使用公网 HTTPS；回调地址固定生成为
`/api/payments/wechatpay/notify`。微信支付未启用时，两个文件参数保留示例中的占位文件即可。

店铺名称与 Logo 是运行时配置，修改后重启应用即可，不需要重新构建前端。Docker
部署只使用三个具体的宿主机文件参数：`SHOP_LOGO_FILE`、
`WXPAY_MERCHANT_PRIVATE_KEY_FILE`、`WXPAY_PUBLIC_KEY_FILE`。Compose 分别把它们
只读映射到 Dockerfile 预设的容器路径，不再挂载整个目录，也无需配置容器内路径。
应用启动时会解析 Logo 内容并确认其为 SVG，而不是信任文件扩展名。

创建订单接口 `POST /api/orders` 使用固定窗口，按客户端 IP 限制为每 3 分钟 5 次；
第 6 次请求返回 `429`，其他接口不参与应用层限流。直连部署根据 TCP 对端 IP 计数；
部署在反向代理后时，需要将代理的精确地址（优先 `/32` 或 `/128`）配置到
`RATE_LIMIT_TRUSTED_PROXY_CIDRS`，并让代理正确
追加或覆盖 `X-Forwarded-For`。不要填写不受控制的客户端网段。限流状态保存在进程
内，多实例部署时每个实例独立计数。

### 管理后台会话

访问 `/admin` 后使用 `ADMIN_KEY` 登录。登录成功后，浏览器只保存服务端签发的
`HttpOnly`、`SameSite=Strict` 会话 Cookie；管理 API 不再接受 `x-admin-key` 请求头。
会话空闲 30 分钟后自动失效，每次有效的管理请求会续期。会话使用应用进程内的
Moka 存储，因此不需要额外部署 Redis 或新增数据库表；应用重启或切换到其他实例后
需要重新登录。当前部署形态是单个应用实例，若以后扩展为多实例，应改为共享会话存储。

当 `PUBLIC_BASE_URL` 使用 HTTPS 时，会话 Cookie 自动启用 `Secure`。生产环境必须把
该变量配置为实际的 HTTPS 对外地址；本地 HTTP 开发会关闭 `Secure` 并在启动日志中
给出明确警告。

### 微信支付官方直连配置

本项目实现的是微信支付官方 API v3 的 Native 支付，与易支付的 `epay/wxpay`
参数是两套独立协议。商户平台需准备已绑定商户号的 AppID、商户 API 证书序列号、
证书对应私钥、APIv3 密钥，以及微信支付公钥 ID 和公钥文件。当前实现使用微信支付
公钥模式验签，不接受未验签的下单应答、查单应答或支付通知。

1. 将 `apiclient_key.pem` 和微信支付公钥放入 `deploy/secrets/`，文件不要提交到 Git；
   并确保 rootless Podman 启动用户对文件具有读权限。
2. 在 `deploy/.env` 中完整填写五项业务凭据，将两个 `WXPAY_*_FILE` 分别指向上述
   宿主机文件，并把 `PUBLIC_BASE_URL` 改为
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

## AGENTS
项目暂时没有生产数据，归一到第一个migration即可，不需要新增

## License

<!-- 待定：请补充许可证（如 MIT / Apache-2.0） -->
