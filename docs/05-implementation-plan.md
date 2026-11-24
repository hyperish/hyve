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

- **진행 순서 메모 (2026-11-24)**: 7단계(`hyve-api`)를 6단계보다 먼저 했습니다. 7단계는 1·3·4단계에만 기대고, 6단계 매크로가 만드는 코드가 7단계 항목을 가리키기 때문입니다. 생성 코드가 `::hyve` 경로를 쓰므로 진입 크레이트의 골격(재수출과 생성 코드용 경로)을 6단계에서 만들고 9단계에서 마무리합니다. 매크로 테스트는 자기 `server` 피처를 가진 미배포 픽스처 크레이트 `hyve-tests`에 두고, CI가 피처를 끈 빌드와 켠 빌드를 모두 검사합니다. 문서 예시 테스트(§1의 `mvp.rs`)도 `crates/hyve` 대신 `crates/hyve-tests/tests/mvp.rs`에 둡니다. `config::load`가 읽을 `hyve.toml` 픽스처를 배포 크레이트에 넣지 않기 위해서입니다. 코어 경계 기준은 `crates/hyve-tests/tests/boundary.rs`가 `Cargo.lock`과 멤버 매니페스트로 확인합니다.

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

### 진행 상황 (2026-11-24)

| 기준 | 상태 | 근거 |
|---|---|---|
| 문서 예시 | 충족 | `crates/hyve-tests/tests/mvp.rs` (README `SceneConfig`, §1 스케치에서 `asset!` 제외, `hyve.toml` 픽스처) |
| 매크로 | 충족 | `crates/hyve-tests/tests/ui.rs`의 컴파일 실패 21건이 `#[model]`·`#[validate]`·`#[server]`·`#[server_ctx]`·`#[hyve::test]`를 모두 다룸. `crate = <경로>`는 `hyve-tests-indirect`가 확인 |
| 직렬화 | 충족 | `hyve-serde` 테스트: 두 코덱 왕복(`u64::MAX`·`i64::MIN`·무한대·NaN·`-0.0` 비트 단위 포함), 잘린 입력·바이트 변조·깊은 중첩(128 한도)·버퍼를 넘는 길이. 기준 아카이브(`tests/reference.bin`)를 네이티브와 `wasm32`가 모두 읽고 바이트까지 같게 씀(CI `wasm` 작업) |
| 상태 | 충족 | `hyve-state` 테스트: 슬롯 반환, 갈래 의존의 이펙트 1회, `update` 안 읽기 |
| 코어 경계 | 충족 | `crates/hyve-tests/tests/boundary.rs` |
| CLI | **일부** | 공통 계약(미구현은 실패, 알 수 없는 명령·옵션·템플릿은 오류, 워크스페이스 없음은 오류, `test`·`build` 종료 코드는 실제 결과)과 `test`·`build` 기본은 `apps/hyve-cli/tests/cli.rs`가 확인. 프레임워크 단계 호출(`codegen`, `build`)은 구현했고, 가짜 실행 파일로 순서·옵션 전달·종료 코드·버전 불일치·실행 파일 없음을, 로컬의 실제 `hyve-orbic`으로 `hyve codegen --check`를 확인했습니다(2026-11-24). `preview`는 `serge` 단계 호출로 구현했고(옵션 전달·제공 도구 없음 오류를 가짜 실행 파일로 확인, 2026-11-25), 실제 `hyve-serge`는 `serge`의 `apps/serge-cli`가 제공합니다. 실제 `hyve-serge`를 `hyve`로 거친 호출은 **미검증**(`hyve-serge`를 설치하지 않음), `preview`가 `orbic` 로컬 서버를 함께 리로드하는 것과 상태 보존은 **미완**(orbic에 `preview` 단계 없음, HMR은 Phase 2). `init` 템플릿(배포된 버전을 적음)은 **미완**(프리릴리스 기준 밖, `orbic`·`serge` 프리릴리스 뒤) |
| 포크 검증 | **일부** | 달라진 점은 아래 기록. 원본과의 측정 비교는 **미완** |
| 배포 가능성 | 충족(로컬) | `cargo package --workspace --offline`으로 공개 크레이트 10개를 임시 로컬 레지스트리에 패키징하고 빌드 검증. 실제 배포는 하지 않음 |
| CI | **미검증** | 같은 명령(포맷, 린트, 테스트 두 빌드, `1.85` 네이티브·`wasm32` 검사, `wasm32` 테스트)이 로컬에서 통과. GitHub Actions 실행은 푸시 전이라 확인하지 않음 |

### 포크 기록 (2026-11-24)

| 대상 | 원본 | 달라진 점 |
|---|---|---|
| `rkyv` | `0.8.18` 고정 의존 | 원본 그대로. 피처는 `std`·`bytecheck`·`little_endian`·`aligned`·`pointer_width_32`. 신뢰할 수 없는 바이트는 원본의 고수준 검증 대신 `hyve-serde`의 깊이 한도(128) 검증기를 거침 |
| `winnow`, `ariadne`, `bytemuck` | `1.0.5`, `0.6.0`, `1.25.2` 고정 의존 | 원본 그대로 |
| `hyve-state` | `reactive_graph` `0.3.0-rc1` 레퍼런스 | 자체 구현. 같은 점: 푸시-풀 세 상태(Clean/Check/Dirty), 소유자 트리 정리. 다른 점: 스레드 로컬 단일 스레드 아레나와 세대 번호·런타임 번호가 붙은 핸들, 비동기 스포너·`AnySubscriber` 동적 디스패치 없음. 노드는 구조체 배열이고 값은 `Rc<dyn Any>`라서, §3 개조 방향의 SoA 분리(값 배열·구독자 비트셋)는 **아직 하지 않음**. 측정 뒤에 정함 |
| `Ulid` | `ulid` `3.0.0` 레퍼런스 | 자체 구현. 난수는 `getrandom`, 같은 밀리초 안에서 단조 증가 |
| `SharedRingBuf<T>` | `rtrb`·`crossbeam-queue` 레퍼런스 | 자체 구현. 소비형 큐가 아니라 최신 샘플 덮어쓰기와 찢어짐 없는 스냅샷 |
| JSON | `serde_json` 대신 | `#[model]`이 생성하는 자체 코덱 (§2.3 규칙) |

측정(원본 대비 시그널 갱신·이펙트 전파, 바이너리 인코딩·디코딩)은 아직 실행하지 않았습니다(**미완**).

### 프리릴리스 기준과 Phase 완료 기준

- **프리릴리스 기준**: 1~9단계(라이브러리)가 위 기준을 충족하면 `0.1.0-alpha.N`을 배포하고, `orbic`과 `serge`는 그 버전에 의존해 시작합니다(`hyperish/docs` `05-development-rules.md` §5). 10단계 중 프레임워크가 필요 없는 부분(`test`, `build` 기본, CLI 공통 계약)도 여기에 포함합니다.
- **Phase 완료 기준**: 10단계 중 프레임워크에 걸린 부분(`init` 템플릿이 적는 버전, `preview`, `hyve-orbic`·`hyve-serge` 단계 호출)은 `orbic`과 `serge`의 프리릴리스가 나온 뒤에 마칩니다. 이것까지 끝나야 `hyve`의 Phase 1이 끝납니다.
- **CI**: 포맷, 린트, 테스트, 최소 Rust 버전 빌드가 통과합니다.
