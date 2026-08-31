# Architecture

## 설계 목표

이 프로젝트는 Wasm 샌드박스의 기능 제한과 MSA의 서비스 경계를 함께 검증합니다.

- Wasm Component는 도메인 규칙만 실행
- Host Runner가 HTTP·네트워크·TLS·DB 자원을 제공
- Gateway·Catalog·Order를 독립 실행·배포
- 서비스별 데이터 소유권 유지
- 공유 DB 트랜잭션 없이 주문과 재고의 일관성 관리

## 서비스 경계

| 서비스 | 책임 | 로드하는 Component | 소유 데이터 |
|---|---|---|---|
| Gateway | 외부 요청 분류·전달, 상태 집계 | `gateway-component.wasm` | 없음 |
| Catalog | 상품·가격·재고·재고 예약 | `product-agent`, `inventory-agent` | `rust_wasm_catalog` |
| Order | 주문 생성·조회·취소, 처리 상태 관리 | `order-agent` | `rust_wasm_ordering` |

상품과 재고는 현재 요구사항에서 같은 이유로 함께 변경되고 주문 과정에서도 원자적인 재고 예약이 필요합니다. 별도 서비스로 나누면 분산 트랜잭션만 늘어나므로 하나의 Catalog 경계로 묶었습니다.

Order는 Catalog 테이블을 직접 읽거나 수정하지 않습니다. 상품 견적·재고 예약·예약 해제는 Catalog 내부 API를 통해서만 수행합니다.

## 전체 구조

```text
Public Network
  Client
    ↓
  Gateway :8091
    ├─ Wasm route decision
    ├─ /api/products/** ─────────────┐
    └─ /api/orders/** ────────┐      │
                              │      │
Internal Network              │      │
  Order :8093                 │   Catalog :8092
    ├─ order-agent.wasm       │     ├─ product-agent.wasm
    ├─ Order DB adapter       │     ├─ inventory-agent.wasm
    └─ HTTP client ───────────┘     └─ Catalog DB adapter
           │                               │
           ↓                               ↓
    rust_wasm_ordering              rust_wasm_catalog
          schema                          schema
              └──── PostgreSQL instance ────┘
```

개발·시연 환경에서는 하나의 PostgreSQL 인스턴스 안에서 스키마를 분리합니다. 테이블 간 서비스 경계 FK를 만들지 않으며 각 서비스는 자기 스키마만 조회·변경합니다. Catalog와 Order는 별도의 접속 환경변수를 받으므로 전용 DB 계정 또는 별도 인스턴스로 물리적 권한까지 분리할 수 있습니다.

## Wasm과 Host 경계

```text
Wasm Component
  ↓ WIT Repository 함수
Host Runner
  ├─ 입력을 SQL 또는 서비스 요청으로 변환
  ├─ DB 접속정보·TLS·Connection Pool 소유
  └─ 결과를 WIT record로 반환
```

Wasm의 WASIp2 Context에는 TCP, UDP, 이름 조회, Host 환경변수와 디렉터리를 허용하지 않습니다. 따라서 Component가 DB나 다른 서비스에 임의로 연결할 수 없고, Host가 구현한 WIT 함수만 사용할 수 있습니다.

WIT는 애플리케이션 기능 계약이며 방화벽이나 DB 권한은 아닙니다. 실제 보안은 다음 계층을 함께 적용해야 합니다.

1. Wasm Capability와 WIT 기능 제한
2. Gateway와 내부 서비스 네트워크 분리
3. 서비스 간 인증·인가
4. TLS와 서비스별 DB 계정
5. Secret 저장소와 로그 마스킹

## 주문 생성 흐름

```text
Client → Gateway → Order Component
                    ↓
             Catalog 견적 조회
                    ↓
          Order: PENDING 주문 저장
                    ↓
       Catalog: token으로 재고 예약
              ┌─────┴─────┐
            성공          실패
              ↓             ↓
       Order: CREATED   Order: REJECTED
```

재고 예약은 `token + product_id`를 기본 키로 사용합니다. 같은 토큰의 요청이 반복되어도 재고를 다시 차감하지 않아 네트워크 재시도에 필요한 멱등성을 제공합니다.

Order가 예약 요청의 응답을 받지 못하면 결과를 추측해 확정하지 않고 `PENDING`으로 남깁니다. 이 상태는 운영 단계에서 재조정 작업이 Catalog 예약 상태를 조회해 확정 또는 취소해야 합니다.

## 주문 취소 흐름

```text
Order: CREATED
      ↓
Order: CANCEL_PENDING
      ↓
Catalog: 예약 해제(token)
      ↓
Order: CANCELLED
```

예약 해제도 이미 `RELEASED`인 토큰에는 재고를 중복 복원하지 않습니다. 통신 실패 시 `CANCEL_PENDING`을 유지해 재시도 또는 재조정 대상으로 식별합니다.

## 실행 모드와 배포 단위

하나의 Host Runner 바이너리가 `RUNNER_MODE`에 따라 필요한 Component와 기능만 로드합니다.

| 모드 | 기본 주소 | 역할 |
|---|---|---|
| `monolith` | `127.0.0.1:8091` | 기존 단일 프로세스 구조 |
| `gateway` | `0.0.0.0:8091` | 외부 Gateway |
| `catalog` | `127.0.0.1:8092` | Catalog Service |
| `order` | `127.0.0.1:8093` | Order Service |

같은 바이너리를 사용하더라도 프로세스·컨테이너·설정·배포 주기가 분리되므로 서비스별 독립 배포가 가능합니다. 다만 같은 VM에 세 프로세스를 실행하면 프로세스 장애는 분리되지만 VM 장애와 자원 부족은 공유합니다. 별도 컨테이너 노드나 VM으로 배치해야 인프라 장애 범위까지 분리됩니다.

## 단일 구조와 MSA 비교

| 구분 | 단일 Host Runner | Gateway·Catalog·Order |
|---|---|---|
| 배포 | 전체 백엔드 한 번에 배포 | 서비스별 독립 배포 |
| 장애 | Runner 장애가 전체 API에 영향 | 서비스 단위 부분 장애 |
| 데이터 | 공유 스키마 | 서비스별 전용 스키마 |
| 주문·재고 | 단일 DB 트랜잭션 | 예약·보상·재조정 필요 |
| 통신 | 프로세스 내부 호출 | HTTP와 타임아웃 발생 |
| 운영 난이도 | 낮음 | 로그·지표·추적·재시도 설계 필요 |

MSA는 서비스 수를 늘리는 방식이 아니라 독립적으로 변경·배포·확장해야 하는 경계를 분리하는 방식입니다. 분리 이익이 네트워크와 운영 복잡도보다 클 때 적용합니다.

## 남은 운영 보완

- Gateway와 내부 서비스 간 인증
- PENDING·CANCEL_PENDING 주문 재조정 작업
- 서비스별 DB 계정과 최소 권한
- 구조화 로그, 지표와 분산 추적
- 제한된 재시도, Circuit Breaker와 요청 ID
- 컨테이너별 readiness/liveness 확인과 무중단 배포
