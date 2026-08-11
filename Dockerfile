FROM rust:1-bookworm AS srv-builder

WORKDIR /app/srv

RUN apt-get update \
    && apt-get install -y --no-install-recommends libpq-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY srv/Cargo.toml srv/Cargo.lock ./
COPY srv/migrations ./migrations
COPY srv/src ./src

RUN cargo build --release


FROM node:22-bookworm-slim AS web-builder

WORKDIR /app/web

COPY web/package.json web/package-lock.json ./
RUN npm ci

COPY web ./


RUN npm run build


FROM debian:bookworm-slim AS qddxp

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libpq5 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home app \
    && install -d -o app -g app /run/assets/qddxp /run/secrets/qddxp

COPY --from=srv-builder /app/srv/target/release/qddxp-srv /usr/local/bin/qddxp-srv
COPY --from=web-builder /app/web/dist /usr/local/share/qddxp/web

# 运行时文件在镜像内使用固定位置。宿主机只需通过 Compose 提供三个具体文件，
# 不再把容器路径暴露为部署参数，也不挂载整个素材或密钥目录。
ENV WEB_DIST_DIR=/usr/local/share/qddxp/web \
    SHOP_LOGO_FILE=/run/assets/qddxp/shop-logo.svg \
    WXPAY_MERCHANT_PRIVATE_KEY_FILE=/run/secrets/qddxp/wechatpay_merchant_private_key.pem \
    WXPAY_PUBLIC_KEY_FILE=/run/secrets/qddxp/wechatpay_public_key.pem

USER app
EXPOSE 3000

CMD ["qddxp-srv"]
