# Rust/Wasm Commerce Platform

Rust Host Runner 안에서 상품·재고·주문 WebAssembly Component를 실행하고 PostgreSQL과 연결하는 학습용 백엔드입니다. 하나의 프로세스와 포트를 사용하는 모듈형 구조부터 시작하며, 이후 컴포넌트를 독립 서비스로 분리해 MSA를 연습할 수 있도록 구성했습니다.

## 1차 구조

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

## Lightsail에서 빌드

### SSH 접속과 재접속

Mac에서 새 터미널을 열고 다음 명령을 실행합니다. `<LIGHTSAIL_STATIC_IP>`는 실제 고정 IPv4로 바꾸며 꺾쇠괄호는 입력하지 않습니다.

```bash
chmod 400 /path/to/LightsailDefaultKey-ap-northeast-2.pem
ssh -i /path/to/LightsailDefaultKey-ap-northeast-2.pem ubuntu@<LIGHTSAIL_STATIC_IP>
```

기존 SSH 창에서 빌드가 실행 중이어도 같은 명령으로 두 번째 SSH 세션을 열 수 있습니다. 새 세션은 기존 빌드를 중단하지 않으며 서버 상태를 별도로 확인할 때 사용합니다.

### 연결 종료에 안전한 빌드

처음 빌드는 Wasmtime과 Cranelift를 컴파일하므로 오래 걸릴 수 있습니다. SSH 연결이 끊겨도 빌드를 유지하려면 `tmux`를 사용합니다.

```bash
sudo apt update
sudo apt install -y tmux
tmux new -s wasm-build
cd ~/apps/rust-wasm-commerce-platform
CARGO_BUILD_JOBS=1 make build
```

`Ctrl+B`를 누른 다음 `D`를 누르면 빌드를 유지한 채 tmux 화면에서 빠져나옵니다. 다시 접속할 때는 다음 명령을 사용합니다.

```bash
tmux attach -t wasm-build
```

### 마지막 빌드 단계가 오래 걸릴 때

`353/354: host-runner(bin)`처럼 마지막에 오래 머무는 것은 Wasmtime이 포함된 Host Runner를 하나의 Linux 실행 파일로 합치는 최종 링크 단계일 수 있습니다. 1GB RAM 환경에서는 Swap을 사용해 특히 느려질 수 있으므로 곧바로 중단하지 않고 두 번째 SSH 세션에서 상태를 확인합니다.

```bash
ps -eo pid,etime,%cpu,%mem,cmd | grep -E 'cargo|rustc|cc|ld' | grep -v grep
free -h
vmstat 2 5
sudo dmesg | grep -i -E 'out of memory|killed process' | tail
```

- `rustc`, `cc`, `ld`가 실행 중이고 CPU 또는 디스크 활동이 있으면 계속 기다림
- Swap 사용량이 늘어나는 것은 느리지만 작업이 진행 중이라는 의미일 수 있음
- 관련 프로세스가 없거나 OOM 기록이 있으면 빌드 실패 원인을 확인한 뒤 재실행
- 같은 프로젝트에서 새로운 `cargo build`를 동시에 실행하지 않음

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
3. 같은 서버에서 주문 컴포넌트를 별도 Host Runner·포트로 분리
4. 독립 배포·장애 격리·확장이 필요해질 때 별도 서버로 분리
