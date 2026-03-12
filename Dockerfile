# ── build stage ───────────────────────────────────────────────────────────────
FROM rust:1.82-slim AS builder

WORKDIR /build

# cache dependency compilation separately from source
COPY Cargo.toml Cargo.lock ./
COPY server/Cargo.toml server/
COPY client/Cargo.toml client/

# create stub sources so cargo can resolve the dependency tree
RUN mkdir -p server/src client/src \
    && echo 'fn main(){}' > server/src/main.rs \
    && echo 'fn main(){}' > client/src/main.rs \
    && cargo build --release -p punchclock-server \
    && rm -rf server/src client/src

# build the real source
COPY server/src server/src
RUN touch server/src/main.rs \
    && cargo build --release -p punchclock-server

# ── runtime stage ─────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/punchclock-server /usr/local/bin/

EXPOSE 8421

ENV API_BASE_URL=http://localhost:8421
ENV RUST_LOG=info

ENTRYPOINT ["punchclock-server"]
