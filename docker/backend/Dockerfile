FROM rust:1.88-bookworm AS builder

WORKDIR /build
COPY backend/Cargo.toml backend/Cargo.lock ./backend/
COPY backend/src ./backend/src
COPY backend/migrations ./backend/migrations

WORKDIR /build/backend
RUN cargo build --release

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl jq \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /opt/ticket-backend

COPY --from=builder /build/backend/target/release/ticket-backend /opt/ticket-backend/ticket-backend
COPY --from=builder /build/backend/migrations /opt/ticket-backend/migrations
COPY docker/backend/entrypoint.sh /opt/ticket-backend/entrypoint.sh

RUN chmod +x /opt/ticket-backend/ticket-backend /opt/ticket-backend/entrypoint.sh

ENTRYPOINT ["/opt/ticket-backend/entrypoint.sh"]
