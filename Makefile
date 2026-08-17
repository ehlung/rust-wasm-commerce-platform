.PHONY: build test run migrate

build:
	cargo build -p host-runner --release
	cargo build --target wasm32-wasip2 --release \
		-p gateway-component \
		-p product-agent-component \
		-p inventory-agent-component \
		-p order-agent-component

test:
	cargo test --workspace

run:
	cargo run -p host-runner

migrate:
	psql "$$DATABASE_URL" -f migrations/0001_init.sql
