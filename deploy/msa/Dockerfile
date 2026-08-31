FROM rust:1.97-bookworm AS builder

WORKDIR /workspace
RUN rustup target add wasm32-wasip2
COPY . .
RUN cargo build -p host-runner --release \
    && cargo build --target wasm32-wasip2 --release \
        -p gateway-component \
        -p product-agent-component \
        -p inventory-agent-component \
        -p order-agent-component

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /workspace/target/release/host-runner /app/host-runner
COPY --from=builder /workspace/target/wasm32-wasip2/release/*.wasm /app/components/

ENV COMPONENT_DIR=/app/components
ENTRYPOINT ["/app/host-runner"]
