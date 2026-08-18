# Architecture

## 문서 목적

이 프로젝트는 상품·재고·주문 도메인을 Wasm Component로 분리하고, 실행·배포·데이터 경계를 단계적으로 확장합니다.

| 단계 | 구조 | 목적 |
|---|---|---|
| 1차 | 단일 Host Runner의 모듈형 백엔드 | Wasm 실행, WIT 권한, DB 연결과 도메인 경계 검증 |
| 2차 | 같은 서버의 도메인별 Host Runner | 프로세스·포트·배포 분리와 서비스 통신 연습 |
| 3차 | 독립 배포와 데이터 소유권을 갖는 MSA | 독립 확장, 장애 격리와 서비스별 데이터 관리 |

복잡도를 처음부터 모두 도입하지 않고, 현재 구조에서 실제로 필요한 경계부터 분리하는 것이 기본 원칙입니다.

## 공통 실행 경계

Host Runner는 운영체제와 네트워크에 접근할 수 있는 일반 Rust 프로세스입니다. Wasm Component는 Wasmtime 샌드박스에서 실행되며 DB 소켓이나 자격증명에 직접 접근하지 않습니다.

```text
Wasm Component
  └─ product-repository.list() 같은 WIT 함수 호출
         ↓
Host Runner의 WIT 구현
  └─ PostgreSQL Connection Pool에서 연결 획득
         ↓
SQL 실행 후 WIT record로 결과 반환
```

이 구조의 핵심은 **Wasm이 외부 자원에 임의로 접근하는 것이 아니라 Host가 WIT 계약으로 공개한 기능만 사용하는 것**입니다.

- Wasm Component: 도메인 규칙, 입력 검증과 처리 흐름
- WIT: Wasm과 Host 사이에서 허용되는 함수와 데이터 형식
- Host Runner: HTTP, Wasmtime, 네트워크, TLS, 비밀정보와 DB 연결
- WASIp2 Context: Rust 표준 라이브러리가 요구하는 CLI·I/O·Clock 등의 표준 Interface 구현. Host 환경변수·디렉터리를 상속하지 않고 TCP·UDP·이름 조회도 명시적으로 비활성화
- 동기식 Wasmtime·PostgreSQL 작업: Tokio의 blocking 전용 스레드에서 실행해 비동기 HTTP 처리 스레드 차단과 중첩 Runtime 충돌 방지
- PostgreSQL Adapter: WIT 호출을 SQL과 트랜잭션으로 변환
- PostgreSQL: 상품, 주문, 주문 항목과 재고 영속화

WASI Interface를 Linker에 등록하는 것과 실제 시스템 자원을 허용하는 것은 다릅니다. 이 프로젝트의 WASIp2 Context는 Component에 Host 환경변수·디렉터리를 상속하지 않고 TCP·UDP·이름 조회도 허용하지 않으며, DB 작업은 사용자 정의 WIT Repository만 통과합니다. WIT는 네트워크 방화벽이나 DB 권한을 대신하지 않으므로 Wasm 기능 제한, 서버 네트워크 정책, DB 계정 권한을 함께 적용해야 전체 접근 범위를 제한할 수 있습니다.

## 1차 구조: 단일 Host Runner의 모듈형 백엔드

### 구성

```text
HTTP Client
    ↓
Host Runner (Axum + Wasmtime, :8091)
    ├─ gateway-component.wasm       경로 분류
    ├─ product-agent.wasm           상품 규칙
    ├─ inventory-agent.wasm         재고 규칙
    └─ order-agent.wasm             주문 규칙
            ↓ WIT Repository Interface
PostgreSQL Adapter + Connection Pool + Transaction
            ↓
PostgreSQL / rust_wasm_shop schema
```

모든 Component는 하나의 Host Runner 프로세스 안에서 실행됩니다. 코드와 WIT 기능은 도메인별로 나뉘지만 HTTP 진입점, 프로세스, 배포 파일, DB 연결 풀과 PostgreSQL 스키마는 공유합니다.

### 경계

| 구분 | 1차 구조의 단위 |
|---|---|
| 코드 | 도메인별 Wasm Component |
| 실행 | Host Runner 프로세스 1개 |
| 네트워크 | `8091` 포트 1개 |
| 배포·확장 | 전체 백엔드 1개 단위 |
| 장애 | Host Runner 장애 시 전체 API 영향 |
| 데이터 | `rust_wasm_shop` 스키마 공유 |

### 주문 트랜잭션

단일 Host Runner와 PostgreSQL을 공유하므로 주문 생성에 필요한 작업을 하나의 로컬 DB 트랜잭션으로 처리할 수 있습니다.

1. 상품 행 잠금과 가격·재고 재확인
2. 주문과 주문 항목 저장
3. 상품 재고 차감
4. 전체 성공 시 `commit`
5. 하나라도 실패하면 `rollback`

주문 취소도 주문 상태 변경과 재고 복원을 하나의 트랜잭션으로 처리합니다. 구현과 일관성 관리가 단순하다는 것이 1차 구조의 장점입니다.

### 적합한 상황과 한계

도메인 경계를 검증하는 초기 단계, 소규모 팀, 모든 기능의 트래픽과 배포 주기가 비슷한 환경에 적합합니다. 다만 상품·재고·주문 중 하나만 독립적으로 배포하거나 확장할 수 없으므로 MSA는 아닙니다.

## 2차 구조: 같은 서버의 Host Runner 프로세스 분리

### 구성

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

각 도메인을 별도 Rust 프로세스와 포트로 실행합니다. Gateway는 요청을 해당 서비스로 전달하고, 각 Host Runner는 자신에게 필요한 Wasm Component와 WIT 구현만 로드합니다.

### 1차에서 달라지는 점

| 구분 | 2차 구조의 단위 |
|---|---|
| 코드 | 도메인별 Wasm Component |
| 실행 | 도메인별 Host Runner 프로세스 |
| 네트워크 | 서비스별 내부 포트 |
| 배포·재시작 | 프로세스별 수행 가능 |
| 장애 | 프로세스 장애는 일부 격리, 서버 장애는 공유 |
| 데이터 | PostgreSQL과 스키마 공유로 결합이 남음 |

같은 Lightsail 안에서도 여러 systemd 서비스로 실행하면 특정 Host Runner만 재시작하거나 교체할 수 있습니다. 그러나 CPU, 메모리, 디스크와 서버 네트워크를 공유하므로 서버 자체가 중단되면 모든 서비스가 함께 중단됩니다.

### 서비스 통신과 주문 처리

Gateway와 도메인 서비스 사이에 HTTP 같은 네트워크 호출이 추가됩니다. 따라서 호출 실패, 타임아웃, 재시도와 부분 장애를 처리해야 합니다.

공유 PostgreSQL을 유지하면 Order Host Runner가 주문과 재고 테이블을 하나의 DB 트랜잭션으로 변경할 수 있지만, 이 방식은 도메인 간 데이터 결합을 남깁니다. 2차 구조에서는 프로세스 통신과 운영 경계를 먼저 학습하고, 데이터 소유권 분리는 3차 구조에서 해결합니다.

### 적합한 상황과 한계

별도 인프라 비용을 늘리지 않고 서비스 간 통신과 프로세스별 배포를 연습하거나, 일부 프로세스의 장애를 분리하려는 단계에 적합합니다. 독립 인프라와 데이터 소유권이 없으므로 완전한 MSA로 보지는 않습니다.

## 3차 구조: 독립 배포 단위와 데이터 소유권을 갖는 MSA

### 구성

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

각 서비스는 별도 VM 또는 컨테이너에 배포할 수 있고, 자신의 DB 또는 독점 스키마와 DB 계정을 가집니다. 다른 서비스의 테이블을 직접 수정하지 않고 API나 이벤트를 통해 협력합니다.

### 독립 경계

| 구분 | 3차 구조의 단위 |
|---|---|
| 코드 | 서비스별 저장소와 Wasm Component |
| 실행 | 서비스별 Host Runner |
| 네트워크 | 서비스별 주소와 내부 API |
| 배포·확장 | 서비스별 독립 수행 |
| 장애 | 서비스와 인프라 단위 격리 |
| 데이터 | 서비스별 DB 또는 독점 스키마 |
| 운영 | 서비스별 상태 확인, 로그, 지표와 알림 |

### 주문과 재고의 일관성

서비스별 DB를 사용하면 주문과 재고를 하나의 PostgreSQL 트랜잭션으로 묶을 수 없습니다. 주문 흐름은 다음과 같은 분산 처리 방식으로 바뀝니다.

```text
Order Service: 주문을 PENDING으로 생성
        ↓
Inventory Service: 재고 예약
        ↓ 성공
Order Service: 주문 확정

재고 예약 실패 또는 후속 처리 실패
        ↓
주문 실패 처리 또는 예약 재고 해제
```

이 과정에는 요청 중복을 막는 멱등성 키, 재시도, 보상 작업, 이벤트 중복 처리와 최종 일관성 설계가 필요합니다. 서비스 분리로 얻는 독립성과 함께 증가하는 대표적인 복잡도입니다.

### 운영 요구사항

- 서비스별 빌드와 배포 파이프라인
- Gateway와 내부 서비스 인증
- 타임아웃, 제한된 재시도와 Circuit Breaker 검토
- 중앙 로그, 분산 추적, 서비스별 지표와 알림
- 비밀정보와 DB 계정의 서비스별 분리
- 상태 확인과 무중단 배포 전략

3차 구조는 특정 도메인만 독립적으로 배포·확장하고 장애 범위를 줄여야 할 때 적합합니다. 이런 요구가 없다면 1차 또는 2차 구조가 더 단순하고 경제적일 수 있습니다.

## 단계별 비교

| 구분 | 1차 | 2차 | 3차 |
|---|---|---|---|
| 구조 성격 | 모듈형 백엔드 | 프로세스 분리 단계 | MSA |
| Host Runner | 1개 | 도메인별 | 서비스별·독립 배포 |
| 서버 | 1대 | 1대 | 서비스별 분리 가능 |
| 데이터 | 공유 스키마 | 공유 결합 유지 | 서비스별 소유 |
| 트랜잭션 | 단일 DB 트랜잭션 | 공유 DB 트랜잭션 가능 | Saga·예약·보상 필요 |
| 확장 | 전체 확장 | 같은 서버 자원 안에서 프로세스 조정 | 서비스별 독립 확장 |
| 장애 격리 | 낮음 | 프로세스 수준 | 서비스·인프라 수준 |
| 운영 복잡도 | 낮음 | 중간 | 높음 |

## 전환 기준

다음 요구가 실제로 확인될 때 다음 단계로 전환합니다.

- 도메인별 배포 주기가 달라짐
- 특정 기능만 독립적으로 확장해야 함
- 일부 기능의 장애가 전체 API에 영향을 주지 않아야 함
- 서비스별 데이터 접근 권한과 책임을 분리해야 함
- 증가한 네트워크·관측성·데이터 일관성 복잡도를 운영할 준비가 됨

포트나 파일을 나누는 것만으로 MSA가 되지는 않습니다. **실행, 배포, 확장, 장애와 데이터 소유권이 서비스 경계에 맞게 분리되어야 합니다.**
