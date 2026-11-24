# 05. 구현 계획 (hyve)

> 상태: **제안** · 기준일: 2026-11-24

Phase 1(`v0.1.0`)의 작업 순서와 완료 기준입니다. Phase 1에 들어가는 크레이트는 [`04-open-questions.md`](04-open-questions.md)의 Phase 컷이, 크레이트 구조와 책임은 [`02-architecture.md`](02-architecture.md)가 정합니다. 패밀리 차원의 완료 기준은 `hyperish/docs`의 `00-vision.md`에 있습니다.

`hyve`는 `orbic`과 `serge`가 모두 의존하므로 세 레포 중 가장 먼저 구현합니다.

## 1. 출발점

- 현재 코드는 서드파티 의존성이 없는 자리표시자입니다. 크레이트는 `hyve`, `hyve-core`, `hyve-macros`, `hyve-serde`, `hyve-state`, `hyve-validate`와 `apps/hyve-cli`뿐이고, 설정·추적·파서·규격 인터페이스는 진입 크레이트 안의 모듈로 들어 있습니다. 고쳐 쓰지 않고 교체합니다.
- 공개 API의 기준은 문서입니다. `crates/hyve/tests/mvp.rs`는 문서 예시를 옮긴 테스트로 다시 씁니다.
- 구현에 앞서 할 일:
  1. 포크 대상의 기준 리비전을 다시 확인해 고정하고 기록합니다(`hyperish/docs` `05-development-rules.md` §7).
  2. 워크스페이스를 `02-architecture.md` §1의 크레이트 구조로 재편합니다.
  3. `Cargo.lock`을 커밋하고, CI에 최소 Rust 버전 빌드를 추가합니다.

## 2. 작업 순서

| 단계 | 크레이트 | 산출물 | 앞선 단계 |
|---|---|---|---|
| 1 | `hyve-core` | 오류 값, 식별자(`Ulid`, `Id<T>`), 시각, 상관관계 ID(`RequestId`/`TraceId`), `CancellationToken`, `Ctx<T>` 스코프 핸들, 인프로세스 `SharedRingBuf<T>` | 없음 |
| 2 | `hyve-diagnostics`, `hyve-parser` | 스팬과 수정 안내가 붙은 진단, DSL·포맷용 파서 기반 | 1 |
| 3 | `hyve-serde` | `rkyv` 포크 편입, 검증된 접근, 스키마 지문, JSON 어댑터 | 1 |
| 4 | `hyve-validate` | `Valid<T>`, 제약 규칙, 스키마 메타데이터 | 1 |
| 5 | `hyve-state` | `reactive_graph` 포크와 자체 아레나, `Signal<T>`·`Memo<T>`·이펙트, 소유 스코프 | 1 |
| 6 | `hyve-macros` | `#[model]`, `#[validate]`, `#[hyve::test]`, `#[server]`/`#[server_ctx]`. 생성 코드가 가리키는 `hyve-api` 항목은 7단계와 함께 맞춥니다 | 2, 3, 4 |
| 7 | `hyve-api` | 계약 디스크립터, 호출 스텁의 전송 트레이트, 전송 계약 상수(`03-adapters-and-pipeline.md` §3), `Path<T>`·`Query<T>`·`Page<T>`·`CursorParams`, 추상 테스트 클라이언트 트레이트 | 1, 3, 4 |
| 8 | `hyve-config` | 우선순위 로더와 시작 시점 검증 | 4, 6 |
| 9 | `hyve` | 재수출과 prelude | 1~8 |
| 10 | `hyve-cli` | `init`, `preview` 기본 실행, `test`, `build` 기본, `codegen`과 `codegen --check`, 프레임워크 단계 호출 | 9 |

`SharedRingBuf`의 OS 공유 메모리 모드, `asset!`, `hyve-trace`, `hyve lint`, 상태 보존 HMR은 Phase 2입니다.

## 3. 완료 기준

- **문서 예시**: `README.md`의 `SceneConfig` 예시와 `01-dx-and-dsl.md` §1 개념 스케치가 컴파일되고 통과합니다. 스케치의 `asset!` 두 줄은 Phase 2라 제외하고, `config::load`가 읽을 값은 테스트 픽스처(`hyve.toml`)로 채웁니다.
- **매크로**: 공개 매크로마다 잘못된 입력에 대한 컴파일 실패 테스트가 있고, 진단은 무엇이 틀렸는지와 고치는 법을 함께 말합니다.
- **직렬화**: 바이너리와 JSON의 왕복 테스트(64비트 정수와 비유한 부동소수 포함), 신뢰할 수 없는 입력(잘린 프레임, 깊은 중첩, 과대 길이) 테스트가 통과합니다. `wasm32` 타깃과 64비트 네이티브 타깃이 서로의 아카이브를 읽는 것을 CI에서 확인합니다.
- **상태**: 소유 스코프를 해제하면 슬롯이 반환되고, 갈래가 여럿인 의존에서도 이펙트가 한 번만 실행되며, `update` 안에서 다른 시그널을 읽어도 패닉하지 않습니다.
- **코어 경계**: 코어 크레이트의 의존 그래프에 비동기 런타임, HTTP, DOM, GPU 크레이트가 없습니다.
- **CLI**: `01-dx-and-dsl.md` §2의 CLI 공통 계약을 테스트로 확인합니다.
- **포크 검증**: 포크본이 원본과 달라진 점과 측정 결과를 남깁니다.
- **배포 가능성**: 공개 크레이트 전부가 패키징됩니다.

### 프리릴리스 기준과 Phase 완료 기준

- **프리릴리스 기준**: 1~9단계(라이브러리)가 위 기준을 충족하면 `0.1.0-alpha.N`을 배포하고, `orbic`과 `serge`는 그 버전에 의존해 시작합니다(`hyperish/docs` `05-development-rules.md` §5). 10단계 중 프레임워크가 필요 없는 부분(`test`, `build` 기본, CLI 공통 계약)도 여기에 포함합니다.
- **Phase 완료 기준**: 10단계 중 프레임워크에 걸린 부분(`init` 템플릿이 적는 버전, `preview`, `hyve-orbic`·`hyve-serge` 단계 호출)은 `orbic`과 `serge`의 프리릴리스가 나온 뒤에 마칩니다. 이것까지 끝나야 `hyve`의 Phase 1이 끝납니다.
- **CI**: 포맷, 린트, 테스트, 최소 Rust 버전 빌드가 통과합니다.
