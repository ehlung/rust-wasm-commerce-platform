.PHONY: build test run migrate migrate-msa run-gateway run-catalog run-order

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

migrate-msa:
	psql "$$DATABASE_URL" -f migrations/0002_msa.sql

run-gateway:
	RUNNER_MODE=gateway APP_ADDR=0.0.0.0:8091 cargo run -p host-runner

run-catalog:
	RUNNER_MODE=catalog CATALOG_DATABASE_URL="$$CATALOG_DATABASE_URL" APP_ADDR=127.0.0.1:8092 cargo run -p host-runner

run-order:
	RUNNER_MODE=order ORDER_DATABASE_URL="$$ORDER_DATABASE_URL" APP_ADDR=127.0.0.1:8093 cargo run -p host-runner
