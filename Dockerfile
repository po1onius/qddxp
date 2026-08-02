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
    && useradd --system --uid 10001 --create-home app

COPY --from=srv-builder /app/srv/target/release/qddxp-srv /usr/local/bin/qddxp-srv
COPY --from=web-builder /app/web/dist /usr/local/share/qddxp/web

ENV WEB_DIST_DIR=/usr/local/share/qddxp/web

USER app
EXPOSE 3000

CMD ["qddxp-srv"]
