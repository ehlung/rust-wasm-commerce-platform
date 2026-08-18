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

- 프로젝트 전용 `rust_wasm_shop` 스키마만 사용
- 보안 DB의 기존 스키마·테이블을 조회하거나 변경하지 않음
- 가능하면 전용 테스트 계정과 최소 권한 사용
- 테스트 종료 후 개인 PostgreSQL로 교체한 다음 공개 저장소 전환

## TLS 인증서 검증

- 기본 설정에서는 PostgreSQL 인증서와 접속 호스트명을 검증
- 운영 환경에서는 인증서의 SAN과 일치하는 호스트명 및 신뢰할 수 있는 CA 인증서 사용
- `PG_TLS_INSECURE=true`는 자체 서명 인증서 또는 IP 불일치가 확인된 격리된 테스트 DB에서만 사용
- 해당 설정에서도 TLS 암호화는 유지되지만 서버 신원 검증을 생략하므로 중간자 공격 방어가 약화됨
- 테스트 종료 후 설정을 제거하고 정상적인 인증서 검증 방식으로 전환

## 유출 시 처리

비밀번호나 키가 한 번이라도 커밋됐다면 파일 삭제만으로 해결되지 않습니다. 즉시 비밀번호·키를 폐기/재발급한 뒤 Git 기록을 정리해야 합니다.
