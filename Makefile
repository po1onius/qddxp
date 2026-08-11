SHELL := /usr/bin/env bash
.DEFAULT_GOAL := help

PODMAN ?= podman
POSTGRES_IMAGE ?= docker.io/library/postgres:18.4-alpine
DB_CONTAINER ?= qddxp-postgres
POSTGRES_DB ?= qddxp
POSTGRES_USER ?= postgres
POSTGRES_PASSWORD ?= postgres
POSTGRES_PORT ?= 5432

API_PORT ?= 3000
LISTEN_ADDR ?= 0.0.0.0:$(API_PORT)
DATABASE_URL ?= postgres://$(POSTGRES_USER):$(POSTGRES_PASSWORD)@localhost:$(POSTGRES_PORT)/$(POSTGRES_DB)
PUBLIC_BASE_URL ?= http://localhost:$(API_PORT)
WEB_RETURN_URL ?= http://localhost:$(API_PORT)/delivery
WEB_DIST_DIR ?= $(CURDIR)/web/dist
ADMIN_KEY ?= change-me
ORDER_PASSWORD_PEPPER ?= dev-insecure-change-me
WXPAY_EXPIRE_MINUTES ?= 15
RUST_LOG ?= info

EPAY_GATEWAY ?=
EPAY_PID ?=
EPAY_KEY ?=
WXPAY_APP_ID ?=
WXPAY_MCH_ID ?=
WXPAY_MERCHANT_SERIAL_NO ?=
WXPAY_MERCHANT_PRIVATE_KEY_PATH ?=
WXPAY_API_V3_KEY ?=
WXPAY_PUBLIC_KEY_ID ?=
WXPAY_PUBLIC_KEY_PATH ?=

.PHONY: help dev srv web-build db-up

help: ## Show available commands.
	@awk 'BEGIN {FS = ":.*## "}; /^[a-zA-Z0-9_.-]+:.*## / {printf "  %-14s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

dev: srv ## Build web assets and start the backend that serves the app.

srv: db-up web-build ## Start the backend with local development environment variables.
	@echo "app: http://localhost:$(API_PORT)"
	@cd srv && \
		DATABASE_URL='$(DATABASE_URL)' \
		LISTEN_ADDR='$(LISTEN_ADDR)' \
		PUBLIC_BASE_URL='$(PUBLIC_BASE_URL)' \
		WEB_RETURN_URL='$(WEB_RETURN_URL)' \
		WEB_DIST_DIR='$(WEB_DIST_DIR)' \
		ADMIN_KEY='$(ADMIN_KEY)' \
		ORDER_PASSWORD_PEPPER='$(ORDER_PASSWORD_PEPPER)' \
		WXPAY_EXPIRE_MINUTES='$(WXPAY_EXPIRE_MINUTES)' \
		RUST_LOG='$(RUST_LOG)' \
		EPAY_GATEWAY='$(EPAY_GATEWAY)' \
		EPAY_PID='$(EPAY_PID)' \
		EPAY_KEY='$(EPAY_KEY)' \
		WXPAY_APP_ID='$(WXPAY_APP_ID)' \
		WXPAY_MCH_ID='$(WXPAY_MCH_ID)' \
		WXPAY_MERCHANT_SERIAL_NO='$(WXPAY_MERCHANT_SERIAL_NO)' \
		WXPAY_MERCHANT_PRIVATE_KEY_PATH='$(WXPAY_MERCHANT_PRIVATE_KEY_PATH)' \
		WXPAY_API_V3_KEY='$(WXPAY_API_V3_KEY)' \
		WXPAY_PUBLIC_KEY_ID='$(WXPAY_PUBLIC_KEY_ID)' \
		WXPAY_PUBLIC_KEY_PATH='$(WXPAY_PUBLIC_KEY_PATH)' \
		cargo run

web-build: ## Build frontend static assets served by the backend.
	@cd web && npm run build

db-up: ## Start local Postgres on localhost:5432.
	@if $(PODMAN) inspect '$(DB_CONTAINER)' >/dev/null 2>&1; then \
		$(PODMAN) start '$(DB_CONTAINER)' >/dev/null; \
	else \
		$(PODMAN) run -d \
			--name '$(DB_CONTAINER)' \
			-e POSTGRES_DB='$(POSTGRES_DB)' \
			-e POSTGRES_USER='$(POSTGRES_USER)' \
			-e POSTGRES_PASSWORD='$(POSTGRES_PASSWORD)' \
			-p '$(POSTGRES_PORT):5432' \
			'$(POSTGRES_IMAGE)' >/dev/null; \
	fi
	@echo "waiting for database..."
	@until $(PODMAN) exec '$(DB_CONTAINER)' pg_isready -U '$(POSTGRES_USER)' -d '$(POSTGRES_DB)' >/dev/null 2>&1; do sleep 1; done
	@echo "database ready: $(DATABASE_URL)"
