#!/usr/bin/env bash
set -euo pipefail

base_url="${BASE_URL:-http://127.0.0.1:8091}"
sku="MSA-$(date +%s)"

curl --fail --silent --show-error "${base_url}/health"
product_json="$(curl --fail --silent --show-error -X POST "${base_url}/api/products" \
  -H 'content-type: application/json' \
  -d "{\"sku\":\"${sku}\",\"name\":\"MSA 검증 상품\",\"priceCents\":25000,\"initialStock\":10}")"
product_id="$(printf '%s' "$product_json" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')"

test -n "$product_id"
order_json="$(curl --fail --silent --show-error -X POST "${base_url}/api/orders" \
  -H 'content-type: application/json' \
  -d "{\"lines\":[{\"productId\":${product_id},\"quantity\":2}]}")"
order_id="$(printf '%s' "$order_json" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')"

test -n "$order_id"
curl --fail --silent --show-error "${base_url}/api/orders/${order_id}"
curl --fail --silent --show-error -X POST "${base_url}/api/orders/${order_id}/cancel"
curl --fail --silent --show-error "${base_url}/api/products"

printf '\nMSA smoke test passed: product=%s order=%s\n' "$product_id" "$order_id"
