# 04. 열린 결정 (hyper)

> 기준일: 2026-11-17

`hyper` 레포 범위의 미정 사항 목록입니다. 범용 명세 전체에 걸친 미정 사항은 [`hyperish/docs/docs/07-open-questions.md`](https://github.com/hyperish/docs/blob/main/docs/07-open-questions.md)에서 추적합니다.

## 열려 있는 결정

| # | 결정 항목 | 왜 중요한가 | 지금의 방향(제안) |
|---|---|---|---|
| 1 | 포크 대상 베이스 크레이트 선정 (직렬화·파서·매크로·반응성) | 포크 + 1단계 `unit optimization`(추상화 제거 및 코어 직결)의 첫 대상 확정 필요 | 라이선스(MIT/Apache-2.0)와 벤치마크 확인 후 선정 |
| 2 | `hyper preview`의 핫리로드·핫패칭 메커니즘 | Rust 컴파일 시간을 극복하고 즉각적인 피드백 루프를 만드는 핵심 | 바이너리 핫패칭(Subsecond 계열) 및 상태(`hyper-state`) 보존 리로드 아키텍처 검토 |

## 결정됨

- **Rust/Cargo 대비 Bun 포지셔닝 확정** (2026-11-17): 올인원 CLI(`init`, `preview`, `lint`, `test`, `build`) + 기본 내장 범용 코어 모듈(`00-overview.md`)
- **`hyper-state` 반응성 코어의 `signal(0)` 표기 및 이원화 스레드 모델(`LocalSignal` / `SyncSignal`) 확정** (2026-11-17 ~ 2026-11-18): `02-architecture.md` §2.2
- **웹 전용 OpenAPI 제외 및 범용 스키마·검증(`hyper-validate`)·인터페이스(`hyper-api`) 경계 확정** (2026-11-18): `00-overview.md`, `02-architecture.md`
