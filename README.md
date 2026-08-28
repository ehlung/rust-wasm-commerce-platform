# Rust/Wasm Commerce Platform

Rust Host Runner가 WebAssembly Component를 실행하고 PostgreSQL과 연결하는 상품·주문 백엔드입니다. 단일 프로세스 구조와 서비스 분리 구조를 같은 코드에서 비교하고, Gateway·Catalog·Order를 독립 실행·배포할 수 있도록 구현했습니다.

## 최종 구조

```text
Client
  ↓ HTTP
API Gateway :8091
  ├─ gateway-component.wasm으로 경로 결정
  ├─ HTTP → Catalog Service :8092
  │           ├─ product-agent-component.wasm
  │           ├─ inventory-agent-component.wasm
  │           └─ rust_wasm_catalog 스키마
  └─ HTTP → Order Service :8093
              ├─ order-agent-component.wasm
              ├─ rust_wasm_ordering 스키마
              └─ HTTP → Catalog 재고 예약·해제 API
```

- **Wasm Component**: 입력 검증과 도메인 처리 흐름
- **WIT**: Component가 Host에 요청할 수 있는 저장소 기능의 계약
- **Host Runner**: HTTP, Wasmtime, TLS, DB 연결과 서비스 통신
- **Catalog Service**: 함께 변경되는 상품·재고를 하나의 서비스 경계로 소유
- **Order Service**: 주문 데이터를 소유하고 Catalog와 예약·보상 방식으로 협력
- **Gateway**: 외부에 공개되는 단일 진입점. 내부 서비스는 직접 공개하지 않음

상품과 재고는 항상 함께 조회·변경되는 작은 도메인이므로 억지로 나누지 않았습니다. 서비스 수보다 **독립 배포 필요성, 데이터 소유권과 변경 이유**를 기준으로 경계를 정했습니다.

## 구조 발전 단계

### 1차 — 단일 Host Runner

`RUNNER_MODE=monolith`로 Gateway·상품·재고·주문 Component를 한 프로세스에서 실행합니다. 모든 데이터는 `rust_wasm_shop` 스키마를 사용하며 주문 저장과 재고 차감을 하나의 DB 트랜잭션으로 처리합니다.

- 장점: 배포와 트랜잭션이 단순함
- 한계: 독립 배포·확장·장애 격리가 불가능함

### 2차 — 같은 서버의 프로세스 분리

`gateway`, `catalog`, `order` 실행 모드로 Host Runner 프로세스와 포트를 분리합니다. 한 서버에서도 서비스 통신, 타임아웃과 부분 장애를 확인할 수 있습니다.

- 장점: 서비스별 재시작·교체 가능
- 한계: 서버 자원과 서버 장애를 함께 공유함

### 3차 — 독립 배포 가능한 MSA

동일한 Runner 이미지를 서로 다른 설정으로 실행하며 Gateway·Catalog·Order를 별도 컨테이너 또는 VM에 배포할 수 있습니다. Catalog와 Order는 서로의 테이블을 직접 변경하지 않고 내부 API로만 협력합니다.

- Catalog 전용 데이터: `rust_wasm_catalog`
- Order 전용 데이터: `rust_wasm_ordering`
- 주문 처리: `PENDING → 재고 예약 → CREATED`
- 주문 취소: `CANCEL_PENDING → 예약 해제 → CANCELLED`
- 재고 예약 토큰: 같은 요청의 중복 차감을 막고 보상 작업을 멱등하게 처리

개발 환경에서는 하나의 PostgreSQL 인스턴스 안에 전용 스키마를 사용합니다. 이는 논리적 소유권을 검증하기 위한 선택이며, 운영 환경에서는 서비스별 DB 계정 또는 DB 인스턴스로 물리적 권한까지 분리할 수 있습니다.

세부 설계는 [아키텍처 문서](docs/architecture.md)를 참고하세요.

## 실행 준비

필요 항목:

- Rust stable과 `wasm32-wasip2` target
- PostgreSQL 14 이상
- `psql`

```bash
rustup target add wasm32-wasip2
cp .env.example .env
```

실제 `DATABASE_URL`은 추적되지 않는 `.env` 또는 배포 환경의 Secret에만 저장합니다.

## 빌드와 테스트

```bash
make build
make test
```

### 단일 Host Runner

```bash
make migrate
make run
```

### MSA 프로세스 분리

```bash
make migrate-msa

# 터미널 1
make run-catalog

# 터미널 2
make run-order

# 터미널 3
make run-gateway
```

Gateway 주소는 `http://127.0.0.1:8091`입니다. Catalog와 Order의 기본 주소는 각각 `127.0.0.1:8092`, `127.0.0.1:8093`입니다.

```bash
chmod +x scripts/msa-smoke-test.sh
scripts/msa-smoke-test.sh
```

### 컨테이너 실행

```bash
cp deploy/msa/.env.example deploy/msa/.env
docker compose --env-file deploy/msa/.env -f deploy/msa/compose.yaml up --build
```

Compose에서는 Gateway만 `8091`로 공개하고 Catalog와 Order는 내부 네트워크에서만 통신합니다.

## API

| Method | Path | 역할 |
|---|---|---|
| `GET` | `/health` | Gateway와 내부 서비스 상태 확인 |
| `GET` | `/api/products` | 상품 목록 조회 |
| `POST` | `/api/products` | 상품 생성 |
| `PATCH` | `/api/products/{id}/stock` | 재고 증감 |
| `POST` | `/api/orders` | 주문 생성과 재고 예약 |
| `GET` | `/api/orders/{id}` | 주문 조회 |
| `POST` | `/api/orders/{id}/cancel` | 주문 취소와 재고 예약 해제 |

```bash
curl -X POST http://127.0.0.1:8091/api/products \
  -H 'content-type: application/json' \
  -d '{"sku":"WASM-BOOK-01","name":"WebAssembly 실전","priceCents":25000,"initialStock":10}'

curl -X POST http://127.0.0.1:8091/api/orders \
  -H 'content-type: application/json' \
  -d '{"lines":[{"productId":1,"quantity":2}]}'
```

## 보안 경계

Wasm Component는 TCP, UDP, 이름 조회, Host 환경변수와 디렉터리를 직접 사용할 수 없습니다. DB 접속정보와 소켓은 Host Runner만 소유하며, Component는 WIT로 공개된 저장소 함수만 호출합니다.

샌드박스는 서비스 인증·방화벽·DB 권한을 대신하지 않습니다. Gateway 공개 범위, 내부 서비스 네트워크, TLS, 서비스별 DB 권한과 비밀정보 관리를 함께 적용해야 합니다.

보안 정보 관리와 공개 저장소 점검 기준은 [보안 문서](docs/security.md)를 참고하세요. Lightsail 실행 과정은 [배포 가이드](docs/lightsail-deployment.md)를 참고하세요.
