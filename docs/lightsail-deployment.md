# Lightsail 배포 및 보안 DB 연결 가이드

이 문서는 Rust/Wasm Commerce Platform을 AWS Lightsail의 Ubuntu 서버에서 빌드하고, 허가받은 보안 PostgreSQL에 연결해 검증하는 전체 과정을 기록합니다.

## 전체 순서

1. Lightsail 서버 환경 준비
2. 프로젝트를 서버로 전송
3. Host Runner와 Wasm Component 빌드
4. 보안 DB 접속정보 설정
5. 전용 스키마 마이그레이션
6. Host Runner 실행과 API 검증
7. PostgreSQL 저장 결과 확인
8. 필요할 때 systemd와 Nginx로 상시 운영

현재 진행 중인 **3단계 서버 빌드**가 끝나면 4단계부터 이어서 진행합니다.

## 작업 전 보안 원칙

- 실제 DB 주소, 포트, DB명, 사용자명, 비밀번호를 Git에 커밋하지 않음
- 서버의 `.env`에만 접속정보를 저장하고 권한을 `600`으로 제한
- 기존 업무 테이블이 아닌 `rust_wasm_shop` 전용 스키마만 사용
- DB 비밀번호, 내부 데이터, 서버 키가 보이는 화면을 공개 자료에 포함하지 않음
- 명령의 `<LIGHTSAIL_STATIC_IP>` 같은 표시는 실제 값으로 바꾸되 `<`, `>`는 입력하지 않음

## 1단계. Lightsail 서버 환경 준비

### SSH 접속

Mac 터미널에서 키 파일 권한을 한 번 설정합니다.

```bash
chmod 400 /path/to/LightsailDefaultKey-ap-northeast-2.pem
```

이후 서버에 접속합니다.

```bash
ssh -i /path/to/LightsailDefaultKey-ap-northeast-2.pem ubuntu@<LIGHTSAIL_STATIC_IP>
```

빌드가 실행 중이어도 같은 명령으로 두 번째 SSH 세션을 열 수 있습니다. 두 번째 세션을 열어도 첫 번째 세션의 작업은 중단되지 않습니다.

### 서버 자원 확인

```bash
cat /etc/os-release
uname -m
free -h
df -h /
sudo swapon --show
```

메모리가 1GB 정도인 서버에서는 Wasmtime 최종 링크에 Swap이 필요합니다. Swap이 없다면 다음과 같이 4GB를 구성합니다.

```bash
sudo fallocate -l 4G /swapfile
sudo chmod 600 /swapfile
sudo mkswap /swapfile
sudo swapon /swapfile
grep -q '^/swapfile ' /etc/fstab || echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab
```

### 필수 패키지 설치

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev ca-certificates curl git postgresql-client rsync tmux
```

패키지 업데이트 중 재시작할 서비스를 묻는 화면이 나오면 기본 선택을 유지한 채 `<Ok>`를 선택합니다.

### Rust와 Wasm target 설치

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
rustup target add wasm32-wasip2
```

설치 결과를 확인합니다.

```bash
rustc --version
cargo --version
rustup target list --installed
psql --version
free -h
df -h /
```

## 2단계. 프로젝트를 서버로 전송

서버에서 프로젝트 디렉터리를 만듭니다.

```bash
mkdir -p ~/apps/rust-wasm-commerce-platform
```

Mac의 새 터미널에서 프로젝트를 전송합니다. `.env`, Git 기록, 빌드 결과, 인증 키는 제외합니다.

```bash
rsync -avz \
  -e "ssh -i /path/to/LightsailDefaultKey-ap-northeast-2.pem" \
  --exclude target \
  --exclude .git \
  --exclude .env \
  --exclude '*.pem' \
  --exclude '*.key' \
  /path/to/rust-wasm-commerce-platform/ \
  ubuntu@<LIGHTSAIL_STATIC_IP>:/home/ubuntu/apps/rust-wasm-commerce-platform/
```

서버에서 전송 결과와 `.env` 미포함 여부를 확인합니다.

```bash
cd ~/apps/rust-wasm-commerce-platform
ls
test ! -f .env && echo '.env not transferred'
```

## 3단계. 서버 빌드

### tmux에서 빌드하는 이유

`tmux`는 SSH 터미널과 분리된 서버 측 작업 공간입니다. SSH 연결이 끊기거나 Mac 터미널을 닫아도 빌드가 서버에서 계속 실행됩니다. 빌드 명령 자체는 같고, 실행을 안전하게 유지하는 방식만 다릅니다.

```bash
tmux new -s wasm-build
cd ~/apps/rust-wasm-commerce-platform
CARGO_BUILD_JOBS=1 make build
```

- tmux에서 빠져나오기: `Ctrl+B`를 누른 다음 `D`
- 기존 빌드 화면으로 돌아가기: `tmux attach -t wasm-build`
- 실행 중인 tmux 목록 확인: `tmux ls`

빌드가 끝나면 다음 파일이 생성됩니다.

```text
target/release/host-runner
target/wasm32-wasip2/release/gateway_component.wasm
target/wasm32-wasip2/release/product_agent_component.wasm
target/wasm32-wasip2/release/inventory_agent_component.wasm
target/wasm32-wasip2/release/order_agent_component.wasm
```

### 마지막 빌드 단계 주의사항

`353/354: host-runner(bin)`처럼 마지막 단계에서 오래 머무는 것은 Wasmtime과 Cranelift를 Host Runner 실행 파일로 합치는 **최종 링크 작업**일 수 있습니다. 특히 1GB RAM 서버는 메모리가 부족해 Swap을 반복해서 읽고 쓰므로 매우 느려질 수 있습니다.

이때 같은 빌드를 다시 실행하지 말고, 두 번째 SSH 세션에서 상태를 확인합니다.

```bash
ps -eo pid,etime,%cpu,%mem,state,cmd | grep -E 'cargo|rustc|cc|ld' | grep -v grep
free -h
vmstat 2 5
sudo dmesg | grep -i -E 'out of memory|killed process' | tail
```

판단 기준:

- `rustc`, `cc`, `ld` 프로세스가 있고 CPU·디스크 활동이 보임: 빌드 진행 중이므로 기다림
- `vmstat`의 `si`, `so`, `wa`가 높음: Swap 입출력 때문에 느린 상태이며 멈춘 것은 아닐 수 있음
- 관련 프로세스가 없고 tmux 화면에 오류가 있음: 오류 확인 후 한 번만 재실행
- `Killed` 또는 OOM 기록이 있음: 메모리 부족으로 종료된 것이므로 Swap과 빌드 작업 수 확인
- 같은 디렉터리에서 빌드를 중복 실행하지 않음

빌드 성공 여부는 다음 명령으로 확인합니다.

```bash
test -x target/release/host-runner && echo 'host-runner ready'
find target/wasm32-wasip2/release -maxdepth 1 -name '*.wasm' -print
```

## 4단계. 보안 DB 접속정보 설정

`.env`는 서버에만 생성합니다.

```bash
cd ~/apps/rust-wasm-commerce-platform
cp .env.example .env
chmod 600 .env
nano .env
```

다음 형식으로 입력합니다. 비밀번호에 특수문자가 있다면 URL 인코딩이 필요할 수 있습니다.

```dotenv
DATABASE_URL='postgresql://<DB_USER>:<DB_PASSWORD>@<DB_HOST>:<DB_PORT>/<DB_NAME>?sslmode=require'
APP_ADDR='127.0.0.1:8091'
```

기본값은 PostgreSQL 인증서와 접속 호스트명을 검증합니다. 자체 서명 인증서 또는 인증서의 IP 불일치가 확인된 격리된 테스트 DB에 한해서만 다음 설정을 추가합니다.

```dotenv
PG_TLS_INSECURE='true'
```

이 설정은 TLS 암호화는 유지하지만 인증서와 호스트명 검증을 생략합니다. 운영 환경에서는 사용하지 말고, 인증서의 SAN과 일치하는 호스트명 및 신뢰할 수 있는 CA 인증서를 사용해야 합니다.

파일 권한을 확인합니다.

```bash
ls -l .env
```

## 5단계. 전용 스키마 마이그레이션

현재 셸에 `.env` 값을 불러온 뒤 마이그레이션을 실행합니다.

```bash
cd ~/apps/rust-wasm-commerce-platform
set -a
source .env
set +a
psql "$DATABASE_URL" -f migrations/0001_init.sql
```

프로젝트 전용 스키마와 테이블만 생성됐는지 확인합니다.

```bash
psql "$DATABASE_URL" -c '\dt rust_wasm_shop.*'
```

## 6단계. Host Runner 실행과 API 검증

먼저 수동으로 실행해 오류를 직접 확인합니다.

```bash
cd ~/apps/rust-wasm-commerce-platform
set -a
source .env
set +a
./target/release/host-runner
```

두 번째 SSH 세션에서 상태를 확인합니다.

```bash
curl http://127.0.0.1:8091/health
```

상품 생성과 조회:

```bash
curl -X POST http://127.0.0.1:8091/api/products \
  -H 'content-type: application/json' \
  -d '{"sku":"WASM-BOOK-01","name":"WebAssembly 실전","priceCents":25000,"initialStock":10}'

curl http://127.0.0.1:8091/api/products
```

응답에서 생성된 상품 ID를 확인한 뒤 주문 생성·조회·취소를 검증합니다.

```bash
curl -X POST http://127.0.0.1:8091/api/orders \
  -H 'content-type: application/json' \
  -d '{"lines":[{"productId":1,"quantity":2}]}'

curl http://127.0.0.1:8091/api/orders/1
curl -X POST http://127.0.0.1:8091/api/orders/1/cancel
```

예시의 `1`은 실제 응답의 상품 ID와 주문 ID로 바꿉니다.

## 7단계. PostgreSQL 저장 결과 확인

새 SSH 세션에서는 환경변수를 다시 불러와야 합니다.

```bash
cd ~/apps/rust-wasm-commerce-platform
set -a
source .env
set +a
```

API 호출 결과가 보안 DB에 저장됐는지 확인합니다.

```bash
psql "$DATABASE_URL" -c 'SELECT id, sku, name, stock FROM rust_wasm_shop.products ORDER BY id;'
psql "$DATABASE_URL" -c 'SELECT id, status, total_cents FROM rust_wasm_shop.orders ORDER BY id;'
psql "$DATABASE_URL" -c 'SELECT * FROM rust_wasm_shop.order_items ORDER BY order_id, product_id;'
```

## 8단계. systemd와 Nginx 적용

수동 실행과 DB 저장까지 확인한 뒤 상시 운영 설정을 적용합니다.

- `deploy/rust-wasm-commerce.service.example`: Host Runner를 systemd 서비스로 실행
- `deploy/nginx.conf.example`: 외부 요청을 `127.0.0.1:8091`의 Host Runner로 전달

권장 구조:

```text
Client
  → Nginx :443
  → Host Runner 127.0.0.1:8091
  → Wasm Components
  → 보안 PostgreSQL
```

Nginx를 사용하는 경우 Lightsail 방화벽에 `8091`을 공개하지 않고, Host Runner도 계속 `127.0.0.1:8091`에만 바인딩합니다. systemd 환경 파일에도 실제 DB 접속정보가 들어가므로 Git에 올리지 않고 파일 권한을 제한합니다.

## 코드 수정 후 다시 배포

Mac에서 같은 `rsync` 명령으로 변경 파일을 다시 전송한 후 서버에서 빌드합니다. 실수로 서버 전용 `.env`를 지우지 않도록 전송 명령에 `--delete`는 사용하지 않습니다.

```bash
tmux new -s wasm-rebuild
cd ~/apps/rust-wasm-commerce-platform
CARGO_BUILD_JOBS=1 make build
```

이미 같은 이름의 tmux 세션이 있다면 `tmux ls`로 확인한 후 기존 세션에 다시 접속합니다.
