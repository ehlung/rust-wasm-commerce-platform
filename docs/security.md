# 보안 DB 연결과 GitHub 보안 기준

## Git에 포함하지 않는 정보

- 보안 DB Host/IP, Port, DB명, Username, Password
- `DATABASE_URL` 실제 값
- Lightsail SSH 개인 키와 TLS 인증서
- 보안 DB의 데이터 덤프, 쿼리 결과, 내부 화면 캡처
- 방화벽 허용 IP와 내부 네트워크 구성 상세

## 저장 위치

| 정보 | 저장 위치 |
|---|---|
| 소스·WIT·빈 스키마 마이그레이션 | GitHub |
| 설정 키 이름과 가짜 예시 | `.env.example` |
| 실제 `DATABASE_URL` | 배포 서버의 `.env` 또는 systemd EnvironmentFile |
| SSH 개인 키 | 로컬 사용자 권한이 제한된 별도 경로 |

## 커밋 전 확인

```bash
git status --short
git diff --cached
rg -n --hidden --glob '!target/**' --glob '!.git/**' \
  '(postgres(ql)?://|DATABASE_URL=.+@|BEGIN (RSA|OPENSSH|EC) PRIVATE KEY|password\s*=)'
```

검색 결과에는 `.env.example`의 가짜 값 외에 실제 접속정보가 없어야 합니다.

## DB 격리

- 단일 실행 모드는 프로젝트 전용 `rust_wasm_shop` 스키마만 사용
- MSA 실행 모드는 Catalog의 `rust_wasm_catalog`과 Order의 `rust_wasm_ordering`을 분리
- 서비스가 다른 서비스의 테이블을 직접 읽거나 변경하지 않도록 Adapter와 내부 API를 분리
- 공개 저장소와 실행 예시는 개인 PostgreSQL만 사용
- 운영 환경에서는 서비스별 DB 계정에 자기 스키마 권한만 부여

하나의 PostgreSQL 인스턴스에 스키마만 분리한 구성은 논리적 소유권을 보여주지만 DB 관리자 계정을 공유하면 권한 격리는 완성되지 않습니다. 실제 배포에서는 서비스별 계정 또는 별도 DB 인스턴스를 사용합니다.

## MSA 네트워크 경계

- 외부에는 Gateway만 공개
- Catalog와 Order의 `/internal/**` API는 내부 네트워크에서만 접근 허용
- 서비스 간 인증을 추가하기 전에는 내부 API를 공용 인터넷에 노출하지 않음
- Gateway와 서비스의 요청 본문을 64KiB로 제한해 과도한 메모리 사용 방지
- 타임아웃과 상태 전이를 함께 사용해 통신 실패를 성공으로 오판하지 않음
- 예약 토큰으로 재고 예약·해제의 중복 실행 방지

## TLS 인증서 검증

- 기본 설정에서는 PostgreSQL 인증서와 접속 호스트명을 검증
- 운영 환경에서는 인증서의 SAN과 일치하는 호스트명 및 신뢰할 수 있는 CA 인증서 사용
- `PG_TLS_INSECURE=true`는 자체 서명 인증서 또는 IP 불일치가 확인된 격리된 테스트 DB에서만 사용
- 해당 설정에서도 TLS 암호화는 유지되지만 서버 신원 검증을 생략하므로 중간자 공격 방어가 약화됨
- 테스트 종료 후 설정을 제거하고 정상적인 인증서 검증 방식으로 전환

## 유출 시 처리

비밀번호나 키가 한 번이라도 커밋됐다면 파일 삭제만으로 해결되지 않습니다. 즉시 비밀번호·키를 폐기/재발급한 뒤 Git 기록을 정리해야 합니다.
