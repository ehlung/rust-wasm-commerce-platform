# Rust/Wasm Commerce Platform

Rust Host Runner 안에서 상품·재고·주문 WebAssembly Component를 실행하고 PostgreSQL과 연결하는 학습용 백엔드입니다. 모듈형 백엔드에서 시작해 프로세스 분리 단계를 거쳐 MSA로 발전시키는 구조를 비교할 수 있도록 설계했습니다.

## 구조 발전 단계

### 1차: 단일 Host Runner의 모듈형 백엔드

```text
HTTP Client
    ↓
Host Runner (Axum + Wasmtime, :8091)
    ├─ gateway-component.wasm       경로 분류
    ├─ product-agent.wasm           상품 규칙
    ├─ inventory-agent.wasm         재고 규칙
    └─ order-agent.wasm             주문 규칙
            ↓ WIT로 허용된 저장소 기능만 호출
PostgreSQL Adapter + Connection Pool + Transaction
            ↓
PostgreSQL / rust_wasm_shop schema
```

- Wasm: 라우팅과 도메인 검증·오케스트레이션
- Host Runner: HTTP, Wasmtime, DB 연결, TLS, 비밀정보, 트랜잭션
- PostgreSQL: 상품, 주문, 주문 항목과 재고의 영속화
- WIT: Wasm이 사용할 수 있는 DB 기능을 명시적으로 제한하는 계약

현재 구현된 구조입니다. 도메인 코드는 서로 다른 Wasm Component로 분리되지만, 하나의 Host Runner 프로세스와 `8091` 포트, 하나의 PostgreSQL 스키마를 공유합니다.

장점:

- 한 번의 배포와 하나의 네트워크 진입점으로 운영이 단순함
- Wasm Component 경계와 WIT 기반 권한 제한을 먼저 검증할 수 있음
- 주문 생성과 재고 차감을 하나의 PostgreSQL 트랜잭션으로 처리하기 쉬움

한계:

- 한 컴포넌트의 장애나 Host Runner 장애가 전체 API에 영향을 줌
- 상품·재고·주문 기능을 각각 독립적으로 배포하거나 확장할 수 없음
- 코드 모듈은 분리됐지만 실행·배포 단위가 하나이므로 MSA는 아님

### 2차: 같은 서버의 Host Runner 프로세스 분리

```text
Client
  ↓
Gateway Host Runner :8091
  ├─ HTTP → Product Host Runner   :8101 → product-agent.wasm
  ├─ HTTP → Inventory Host Runner :8102 → inventory-agent.wasm
  └─ HTTP → Order Host Runner     :8103 → order-agent.wasm
                                      ↓
                          PostgreSQL / shared schema
```

각 도메인을 별도 Host Runner와 포트로 실행하는 중간 구조입니다. 서비스별 프로세스를 따로 시작하고 재배포할 수 있어 API 통신, 타임아웃, 장애 처리 같은 MSA의 기본 운영 방식을 연습할 수 있습니다.

장점:

- 도메인별 프로세스 실행·재시작·배포 가능
- 특정 서비스의 프로세스 장애가 다른 프로세스로 바로 전파되는 것을 줄임
- 하나의 Lightsail 서버에서 추가 인프라 비용 없이 서비스 통신을 연습할 수 있음

한계:

- 모든 프로세스가 같은 서버를 사용하므로 서버 장애와 자원 부족을 함께 겪음
- PostgreSQL 스키마를 공유하면 데이터 변경이 다른 서비스에 영향을 줄 수 있음
- 프로세스는 분리됐지만 인프라와 데이터 결합이 남아 있어 완전한 MSA는 아님

### 3차: 독립 배포 단위와 데이터 소유권을 갖는 MSA

```text
Client
  ↓
API Gateway
  ├─ Product Service
  │    └─ Host Runner + product-agent.wasm
  │         └─ Product DB/schema
  ├─ Inventory Service
  │    └─ Host Runner + inventory-agent.wasm
  │         └─ Inventory DB/schema
  └─ Order Service
       └─ Host Runner + order-agent.wasm
            └─ Order DB/schema

Service communication
  ├─ synchronous API
  └─ asynchronous event/message
```

각 서비스가 별도 VM 또는 컨테이너에 배포되고, 자신의 데이터와 실행 환경을 소유하는 목표 구조입니다. 특정 서비스만 독립적으로 배포·확장할 수 있으며 한 서버의 장애가 전체 시스템 장애로 이어지는 범위를 줄입니다.

필요한 변화:

- 서비스별 독립 빌드·배포·상태 확인과 로그 수집
- 서비스별 DB 또는 스키마 소유권 분리
- 서비스 사이의 인증, 타임아웃, 재시도와 장애 대응
- 공유 DB 트랜잭션 대신 API 또는 이벤트를 이용한 데이터 일관성 설계
- Gateway에서 라우팅하고 각 서비스는 필요한 기능만 외부에 공개

3차 구조가 독립 배포·장애 격리·확장이라는 MSA의 핵심 특성을 갖는 최종 발전 형태입니다. 다만 서비스 수가 늘수록 네트워크 통신과 운영 복잡도도 증가하므로, 실제 분리는 독립 확장이나 장애 격리가 필요한 도메인부터 적용합니다.

세부 실행 경계와 주문 트랜잭션은 [docs/architecture.md](docs/architecture.md)를 참고하세요.

## 보안 DB 정보를 GitHub에 남기지 않는 원칙

실제 DB 주소·포트·DB명·사용자명·비밀번호는 **소스나 문서에 작성하지 않습니다.** Host Runner는 `DATABASE_URL` 환경변수만 읽으며 `.env`, 인증서, DB 덤프는 `.gitignore`에서 제외됩니다.

비공개 저장소도 비밀정보 저장소가 아닙니다. 보안 DB 연결 테스트에는 로컬 또는 서버의 추적되지 않는 `.env`를 사용하고, 공개 포트폴리오에는 개인 PostgreSQL 접속정보만 사용합니다. 실제 보안 DB 데이터와 내부 실행 화면도 커밋하지 않습니다.

자세한 점검표는 [docs/security.md](docs/security.md)를 참고하세요.

## 실행 준비

필요 항목:

- Rust stable, `wasm32-wasip2` target
- PostgreSQL 14 이상
- `psql`

```bash
rustup target add wasm32-wasip2
cp .env.example .env
```

`.env`의 `DATABASE_URL`에는 허가받은 테스트 DB 또는 개인 DB를 입력합니다.

## 빌드와 실행

```bash
make build
make migrate
make test
make run
```

기본 주소는 `127.0.0.1:8091`입니다.

## Lightsail 배포

서버 준비, 프로젝트 전송, 장시간 빌드, 보안 DB 연결과 실행 검증 과정은 [Lightsail 배포 및 보안 DB 연결 가이드](docs/lightsail-deployment.md)를 참고하세요.

## API

| Method | Path | 역할 |
|---|---|---|
| `GET` | `/health` | Host Runner와 Wasm 구성 확인 |
| `GET` | `/api/products` | 상품 목록 조회 |
| `POST` | `/api/products` | 상품 생성 |
| `PATCH` | `/api/products/{id}/stock` | 재고 증감 |
| `POST` | `/api/orders` | 주문 생성·재고 차감 트랜잭션 |
| `GET` | `/api/orders/{id}` | 주문 조회 |
| `POST` | `/api/orders/{id}/cancel` | 주문 취소·재고 복원 트랜잭션 |

상품 생성 예시:

```bash
curl -X POST http://127.0.0.1:8091/api/products \
  -H 'content-type: application/json' \
  -d '{"sku":"WASM-BOOK-01","name":"WebAssembly 실전","priceCents":25000,"initialStock":10}'
```

주문 생성 예시:

```bash
curl -X POST http://127.0.0.1:8091/api/orders \
  -H 'content-type: application/json' \
  -d '{"lines":[{"productId":1,"quantity":2}]}'
```

## 다음 단계

1. React 관리 화면 연결
2. 인증·권한과 관측성 추가
3. 2차 구조로 전환해 도메인별 Host Runner와 포트 분리
4. 서비스별 데이터 소유권과 통신 방식을 설계해 3차 MSA 구조로 전환
