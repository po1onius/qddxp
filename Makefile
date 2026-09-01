SHELL := /usr/bin/env bash

PODMAN ?= podman
# 本地开发与生产共用 deploy/compose.yml，通过独立项目名隔离容器与数据卷，
# 保证测试环境与生产配置一致（单一 compose 文件，无同步漂移问题）。
PODMAN_COMPOSE ?= $(PODMAN) compose -p '$(COMPOSE_PROJECT)' -f '$(CURDIR)/deploy/compose.yml'
COMPOSE_PROJECT ?= qddxp-dev
DB_CONTAINER_NAME ?= qddxp-dev-postgres
POSTGRES_DB ?= qddxp
POSTGRES_USER ?= postgres
POSTGRES_PASSWORD ?= postgres
POSTGRES_PORT ?= 5432

API_PORT ?= 3000
LISTEN_ADDR ?= 0.0.0.0:$(API_PORT)
DATABASE_URL ?= postgres://$(POSTGRES_USER):$(POSTGRES_PASSWORD)@localhost:$(POSTGRES_PORT)/$(POSTGRES_DB)
PUBLIC_BASE_URL ?= http://localhost:$(API_PORT)
WEB_RETURN_URL ?= http://localhost:$(API_PORT)/orders
WEB_DIST_DIR ?= $(CURDIR)/web/dist
SHOP_NAME ?= 小白羊AI小铺
SHOP_LOGO_FILE ?= $(CURDIR)/deploy/assets/shop-logo.svg
ADMIN_KEY ?= change-me
ORDER_PASSWORD_PEPPER ?= dev-insecure-change-me
WXPAY_EXPIRE_MINUTES ?= 15
RUST_LOG ?= info
LOG_DIR ?= $(CURDIR)/logs
TELEGRAM_BOT_TOKEN ?=
TELEGRAM_NOTIFY_CHAT_ID ?=

EPAY_GATEWAY ?=
EPAY_PID ?=
EPAY_KEY ?=
EPAY_ACTIVE ?=
WXPAY_APP_ID ?=
WXPAY_MCH_ID ?=
WXPAY_MERCHANT_SERIAL_NO ?=
WXPAY_MERCHANT_PRIVATE_KEY_FILE ?= $(CURDIR)/deploy/secrets/disabled-placeholder
WXPAY_API_V3_KEY ?=
WXPAY_PUBLIC_KEY_ID ?=
WXPAY_PUBLIC_KEY_FILE ?= $(CURDIR)/deploy/secrets/disabled-placeholder

# podman-compose 在解析 compose 文件时即插值全部变量（含 :? 必填项），即使只启动
# db 服务也要求这些变量存在。导出后 make 传参与 compose 解析、容器配置保持一致。
export POSTGRES_DB POSTGRES_USER POSTGRES_PASSWORD POSTGRES_PORT DB_CONTAINER_NAME \
	SHOP_NAME SHOP_LOGO_FILE \
	WXPAY_MERCHANT_PRIVATE_KEY_FILE WXPAY_PUBLIC_KEY_FILE

.PHONY: dev

# 唯一默认目标：启动本地数据库（podman compose，只起 db 服务）、构建前端并运行后端。
dev:
	@$(PODMAN_COMPOSE) up -d db
	@echo "waiting for database..."
	@until $(PODMAN) exec '$(DB_CONTAINER_NAME)' pg_isready -U '$(POSTGRES_USER)' -d '$(POSTGRES_DB)' >/dev/null 2>&1; do sleep 1; done
	@echo "initializing pgBackRest stanza if missing (db is configured with WAL archiving)..."
	@# stanza-create 幂等：stanza 已存在且有效时直接成功返回。
	@$(PODMAN) exec '$(DB_CONTAINER_NAME)' pgbackrest --stanza=qddxp --log-level-console=info stanza-create
	@echo "database ready: $(DATABASE_URL)"
	@echo "building web assets..."
	@cd web && npm run build
	@echo "app: http://localhost:$(API_PORT)"
	@cd srv && \
		DATABASE_URL='$(DATABASE_URL)' \
		LISTEN_ADDR='$(LISTEN_ADDR)' \
		PUBLIC_BASE_URL='$(PUBLIC_BASE_URL)' \
		WEB_RETURN_URL='$(WEB_RETURN_URL)' \
		WEB_DIST_DIR='$(WEB_DIST_DIR)' \
		SHOP_NAME='$(SHOP_NAME)' \
		SHOP_LOGO_FILE='$(SHOP_LOGO_FILE)' \
		ADMIN_KEY='$(ADMIN_KEY)' \
		ORDER_PASSWORD_PEPPER='$(ORDER_PASSWORD_PEPPER)' \
		WXPAY_EXPIRE_MINUTES='$(WXPAY_EXPIRE_MINUTES)' \
		RUST_LOG='$(RUST_LOG)' \
		LOG_DIR='$(LOG_DIR)' \
		TELEGRAM_BOT_TOKEN='$(TELEGRAM_BOT_TOKEN)' \
		TELEGRAM_NOTIFY_CHAT_ID='$(TELEGRAM_NOTIFY_CHAT_ID)' \
		EPAY_GATEWAY='$(EPAY_GATEWAY)' \
		EPAY_PID='$(EPAY_PID)' \
		EPAY_KEY='$(EPAY_KEY)' \
		EPAY_ACTIVE='$(EPAY_ACTIVE)' \
		WXPAY_APP_ID='$(WXPAY_APP_ID)' \
		WXPAY_MCH_ID='$(WXPAY_MCH_ID)' \
		WXPAY_MERCHANT_SERIAL_NO='$(WXPAY_MERCHANT_SERIAL_NO)' \
		WXPAY_MERCHANT_PRIVATE_KEY_FILE='$(WXPAY_MERCHANT_PRIVATE_KEY_FILE)' \
		WXPAY_API_V3_KEY='$(WXPAY_API_V3_KEY)' \
		WXPAY_PUBLIC_KEY_ID='$(WXPAY_PUBLIC_KEY_ID)' \
		WXPAY_PUBLIC_KEY_FILE='$(WXPAY_PUBLIC_KEY_FILE)' \
		cargo run
