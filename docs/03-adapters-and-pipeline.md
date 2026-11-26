# 03. 파이프라인과 호환 어댑터 (hyve)

> 상태: 코어 `ref` + 어댑터 분리 구조는 **확정**, 세부 파이프라인은 **제안** · 기준일: 2026-11-24

`hyve`는 모든 계층의 코어 기준(`ref`)이므로, 내부 코어 연산과 외부/타깃 호환 어댑터 사이의 경계를 가장 엄격하게 정의합니다(`hyperish/docs` `02-core-contracts.md` §5).

## 1. 코어(`ref`) 중심 수직 통합 파이프라인

```
               ┌─────────────────────────────────────────────────────┐
               │                        hyve                         │
               │  (범용 매크로 · 상태 · 직렬화 · 검증 · 파서 · API)  │
               └───────────────┬──────────────────┬──────────────────┘
                               │ ref              │ ref
                               ▼                  ▼
                            [ orbic ]           [ serge ]
                            (백엔드)     (Web DOM + Native GPU/CPU UI)
                               │                  │
                               ▼                  ▼
                         [호환 어댑터]        [타깃 & 호환 어댑터]
                         HTTP/WS/DB         Web(DOM/SSR/Ionic) · Native(GPU/CPU/OS)
```

1. **단일 모델·단일 계약**: 데이터 모델, 검증 규칙, 상태 시그널, API 인터페이스는 `hyve` 코어 크레이트에서 한 번만 정의합니다.
2. **제로 보일러플레이트 연결**: `orbic`은 `hyve-api`와 `hyve-serde`를 참조해 핸들러와 전송 파이프를 구성하고, `serge`(웹·네이티브)는 동일한 `hyve-api`와 `hyve-state`를 참조해 UI 상태와 원격 호출을 직결합니다.
3. **개발 툴 파이프라인 (`hyve dev` / `lint` / `test` / `build`)**: CLI가 워크스페이스 그래프를 분석해 코어 변경이 백엔드(`orbic`)와 웹·네이티브 UI(`serge`)에 미치는 영향을 한 번에 빌드·검증합니다.

## 2. 어댑터 경계와 변환 규칙

`hyve`에서 정의하는 어댑터 트레이트(`Adapter`, `CodecAdapter`, `TransportAdapter` 등)는 다음 계약을 강제합니다. 코덱 트레이트는 `hyve-serde`가, 전송 트레이트와 추상 테스트 클라이언트 트레이트는 `hyve-api`가 소유합니다. 시그니처는 구현이 정하되 §3의 계약을 표현할 수 있어야 합니다.

- **코어의 외부 규약 무지성**: `hyve-core`, `hyve-state`에는 HTTP 상태 코드, 브라우저 DOM 노드, 특정 GPU API 타입이 절대 들어가지 않습니다. `hyve-api`도 외부 규약을 해석하지 않으며, `#[server]`의 메서드·경로 같은 바인딩 정보는 어댑터가 해석할 메타데이터로만 보관합니다(`02-architecture.md` §2.4).
- **손실 정책의 명시화**: 내부 모델의 타입(예: 고정밀 수치, 커스텀 열거형, 스트리밍 핸들)을 외부 JSON이나 웹 브리지로 변환할 때 표현 불가능한 값이 있으면 `Reject`(거부) / `Approximate`(근사) / `PassThrough`(보존 필드 통과) 중 하나를 어댑터가 명시적으로 선택해야 합니다.
- **API 규격 어댑터 연결**:
  - 내부 통신(예: `orbic` 내부 MSA, `serge` SSR ↔ `orbic`, 네이티브 IPC)은 고성능 내부 코덱(`hyve-serde`)을 직접 사용할 수 있습니다. 양쪽의 스키마가 어긋나면 표준 레일로 폴백합니다(`02-architecture.md` §2.3).
  - 외부 공개 API나 서드파티 연동이 필요한 경계에서는 `hyperish/docs`의 `03-api-protocol.md`(HTTP/JSON) 및 `04-realtime-communication.md`(WebSocket) 규약을 구현하는 어댑터를 장착합니다.

## 3. 인라인 서버 함수 전송 계약 (제안, 2026-11-24)

`#[server]` 호출이 경계를 넘을 때 `hyve-api`(계약), `orbic`(서버), `serge-adapter-client`(클라이언트)가 함께 지키는 규칙입니다. 여기 적은 미디어 타입과 헤더 이름은 `hyve-api`가 상수로 정의하고 양쪽은 그 상수만 씁니다. 이름을 바꿀 때는 이 절과 상수를 함께 바꿉니다.

### 3.1 매크로가 남기는 것

- **계약 디스크립터** (모든 빌드): 함수 식별자(크레이트 이름을 포함한 함수 경로), 바인딩 메타데이터(메서드, 경로), 요청·응답 모델의 스키마 메타데이터, 스키마 지문. 지문은 요청 모델, 응답 모델, 오류 값의 형식을 함께 반영합니다(`02-architecture.md` §2.3).
- **스텁** (모든 빌드): 함수 이름으로 호출하는 항목입니다(`02-architecture.md` §2.4).
- **핸들러 본문** (`server` 피처 빌드만): 디스크립터에 묶여 `orbic`의 `.handle(...)`로 등록됩니다.

### 3.2 스텁이 전송을 찾는 방법

- 스텁은 인자로 전송을 받지 않고, 현재 스코프에 제공된 전송(`hyve-api`의 전송 트레이트 구현)을 찾아 호출합니다. 앱이 루트에서 한 번 제공합니다.
- 전송 트레이트는 타입이 있는 호출 수준입니다. 계약 디스크립터와 요청 모델을 받아 응답 모델이나 `HyveError`를 돌려줍니다. 레일 선택, HTTP 변환, 폴백, 봉투 해석(§3.3·§3.4)은 전부 전송 구현의 일이고, 스텁과 `hyve-api`는 HTTP를 모릅니다. 아래에서 "전송"이 한다고 적은 일은 이 구현이 합니다.
- 제공된 전송이 없으면 `Infra` 오류 `transport_unavailable`입니다. 스텁 호출은 조립에 선언되는 주입이 아니라 일반 함수 호출이라, 조립 시점 검증(`hyperish/docs` `02-core-contracts.md` §1)의 대상이 아닙니다.
- 구현은 둘입니다.
  - **원격 전송** (`serge-adapter-client`): 기준 URL(출처와 경로 접두어)을 설정으로 받아 §3.3·§3.4대로 HTTP로 보냅니다. 기본은 고속 레일을 먼저 쓰고, 설정으로 한 레일에 고정할 수 있습니다(외부 호환 점검, 두 레일 비교 테스트). `#[server]`의 경로는 이 기준 URL에 이어 붙이며, 서버에서 그 함수를 등록한 컴포넌트의 마운트 접두어(예: `/v1`)는 기준 URL에 포함합니다.
  - **인프로세스 전송** (`orbic`): 소켓 없이 같은 요청 파이프라인을 호출합니다. 기본은 요청 모델을 직렬화하지 않고 넘기며(`hyperish/docs` `02-core-contracts.md` §5의 Tier 1), 가드는 그대로 거칩니다. 포트 없는 풀스택 테스트와 서버·UI 단일 바이너리가 이 전송을 씁니다. `TestClient`는 레일을 지정해 어댑터의 변환 계층(직렬화 포함)까지 통과시키는 방식으로 두 레일을 검증합니다.
- §3.3·§3.4를 구현하는 곳은 운영용 `serge-adapter-client`와 검증용 `TestClient` 둘입니다. 둘이 같은 규칙을 따르는지는 패밀리 수직 슬라이스 테스트가 확인합니다.
- 뷰가 언마운트되면 호출의 `CancellationToken`이 취소되고, 전송은 진행 중인 요청을 끊습니다. 끊긴 호출은 재시도할 수 없는 `Infra` 오류 `cancelled`로 끝납니다(제안, 2026-11-24, `hyve-api`의 `errors::cancelled`).

### 3.3 고속 레일 요청-응답 (HTTP 운반)

| 항목 | 규칙 |
|---|---|
| 메서드와 URL | 바인딩된 메서드와 무관하게 `POST`. URL은 표준 레일과 같고 경로 자리표시자도 같은 방식으로 채움 |
| 구분 | `Content-Type: application/vnd.hyperish.rkyv` |
| 함수 지정 | 헤더 `Hyperish-Fn: <함수 식별자>`. 서버는 경로로 컴포넌트 트리의 위치를 찾고 그 경로에 등록된 `#[server]` 함수 중 식별자가 같은 것을 고름. 같은 경로에 메서드만 다른 함수가 여럿 있어도 구분됨 |
| 스키마 지문 | 헤더 `Hyperish-Schema: <지문>` |
| 요청 본문 | `hyve-serde`로 아카이브한 요청 모델. 전송 인자가 0개면 길이 0 |
| 성공 응답 | 표준 레일과 같은 상태 코드, 같은 미디어 타입, 본문은 아카이브한 응답 모델 |
| 오류 응답 | 표준 레일과 같은 상태 코드, 같은 미디어 타입, 본문은 아카이브한 오류 값(`02-architecture.md` §2.1의 필드). 서버가 레일을 고르기 전에 만든 공개 형태를 그대로 싣기 때문에 표준 레일 봉투와 내용이 같음(`orbic/docs/03-adapters-and-pipeline.md` §1). 가드가 핸들러 전에 거부한 경우도 같음 |
| 가드 | 레일과 무관하게 같은 파이프라인을 통과. 메서드에 따라 달라지는 검사(CSRF 등)는 바인딩된 메서드를 기준으로 함 |

- **불일치와 폴백**: 서버는 고속 레일 요청을 처리할 수 없으면 `415`와 표준 레일 오류 봉투(JSON)로 답합니다. 지문이 다르면 `code`는 `schema_mismatch`, 그 경로가 고속 레일을 받지 않거나 함수 식별자를 모르면 `unsupported_media_type`입니다. 전송은 고속 레일 요청에 `415`가 오면 `code`와 무관하게 같은 호출을 표준 레일로 한 번 다시 보냅니다. 서버는 이 판정을 라우트 매칭 단계에서, 가드와 멱등성 게이트보다 먼저 하므로 부수효과와 멱등 키 기록은 생기지 않습니다.
- **응답 해석**: 응답의 `Content-Type`이 고속 레일 미디어 타입이 아니면 전송은 표준 레일 봉투로 해석합니다(중간 프록시가 만든 오류 포함). 어느 쪽으로도 해석할 수 없으면 `decode_failed`입니다.

### 3.4 표준 레일 매핑

- 요청과 응답은 `hyperish/docs` `03-api-protocol.md`의 HTTP/JSON 규약 그대로입니다. 메서드와 경로는 바인딩 메타데이터를 따르고, 본문 없는 메서드의 매핑은 `orbic/docs/01-dx-and-dsl.md` §2가 정합니다.
- 전송은 표준 레일 오류를 다음과 같이 복원합니다. 서버 쪽 변환은 `orbic/docs/03-adapters-and-pipeline.md` §1이 같은 규칙으로 정의합니다.
  - 종류: 상태 코드가 4xx면 `Domain`, `503`이면 `Infra`, 그 밖의 5xx면 `Internal`.
  - `code`, `message`, `details`, `requestId`: 봉투의 `error` 필드 그대로.
  - 재시도 가능 여부: `Retry-After` 헤더가 있으면 참.
- 성공 응답은 `200`이면 `data`를 응답 모델로 읽고, `204`면 값 없는 성공입니다. 목록 응답(`Page<T>`)은 `data`의 항목과 `page`의 커서를 합쳐 읽습니다. 봉투를 쓰고 읽는 규칙은 `hyve-serde`의 `Json::encode_data`/`decode_data`(`to_json_data`/`from_json_data`) 하나이고 `Page<T>`가 이를 재정의하므로, 서버 어댑터와 전송은 봉투를 따로 구현하지 않습니다. 전송은 성공 본문 전체를 스텁에 넘깁니다(제안, 2026-11-24).

### 3.4.1 공유 구현 (제안, 2026-11-25)

- 호출을 HTTP 요청으로 바꾸는 규칙(§3.3의 헤더·본문, §3.4의 메서드·경로 자리표시자·쿼리 문자열)과 응답을 결과로 되돌리는 규칙(상태 코드별 오류 종류, `Retry-After`로 재시도 가능 여부, `204`, 봉투)은 `hyve-api`의 `http` 모듈(`encode`, `decode`, `wants_standard_rail`)이 I/O 없는 함수로 가집니다. 원격 전송(`serge-adapter-client`)과 테스트 클라이언트(`orbic`의 `TestClient`)는 이를 함께 써서 서로 어긋나지 않고, 보내는 일과 자격 증명은 각 전송이 맡습니다.
- 경로·쿼리의 텍스트 파라미터와 모델 사이의 변환(`parse_query`, `from_params`, `percent_encode`, 인자의 JSON 이름)은 `hyve-api`의 `params` 모듈이 가지며, `orbic`의 `Path<T>`·`Query<T>` 추출과 `serge`의 라우트 파라미터가 함께 씁니다.

### 3.5 자격 증명

스텁은 자격 증명을 다루지 않습니다. 쿠키, `X-CSRF-Token`, `Bearer` 토큰을 요청에 붙이는 것은 전송 구현의 일입니다(`serge/docs/03-adapters-and-pipeline.md` §2).

## 4. API 스키마 호환성 검사

`hyve lint --schema released.json current.json`은 oasdiff v1.41.0으로 두 OpenAPI 문서를 비교합니다. 도구는 PATH 또는 `HYVE_OASDIFF` 환경 변수에서 찾습니다. 앱이 평소의 `hyve lint`에서 서버 계약을 검사하려면 Cargo.toml에 다음을 추가합니다.

```toml
[workspace.metadata.hyve.api]
baseline = "api/released.json"
package = "notes-app"
example = "export_api"
```

예제 바이너리는 앱의 서버 피처로 실행되며 `orbic::openapi_json(app)?`의 반환값을 표준 출력에 씁니다. 이미 생성한 문서가 있으면 `package`와 `example` 대신 `current = "api/current.json"`을 지정합니다. 기준 파일 경로는 워크스페이스 루트 기준입니다. 선택적 쿼리를 추가하는 변경은 통과하고 기존 호출에 필수 쿼리를 추가하면 검사가 실패합니다. [CLI 통합 테스트](../apps/hyve-cli/tests/cli.rs) · [oasdiff 호환성 규칙](https://github.com/oasdiff/oasdiff/blob/main/docs/BREAKING-CHANGES.md)

## 네이티브 배포 산출물

Android 앱은 `hyve build -p my-app --debug --format apk,aab`로 빌드합니다. JDK 17, Android SDK와 NDK, 해당 Rust 타깃, `cargo-apk` 0.10.0을 사용합니다. 기본 NativeActivity의 AAB 변환에는 `HYVE_BUNDLETOOL`에 지정한 Google bundletool JAR를 사용합니다. 위젯, `[package.metadata.android]`의 `window_layout`·`document_picker`·`background_task`·`webview` 중 하나, 또는 `manifest_overlay = "android/AndroidManifest.xml"`를 선택하면 Gradle 8.10.2로 APK와 AAB를 직접 병합·서명합니다. `HYVE_GRADLE`로 실행 파일을 지정할 수 있으며, 네이티브 코드가 있는 ABI만 패키징합니다. 개발용 서명 키는 Android 도구가 로컬에서 만듭니다. 릴리스는 앱의 Cargo Android 서명 설정 또는 `CARGO_APK_RELEASE_KEYSTORE`와 `CARGO_APK_RELEASE_KEYSTORE_PASSWORD`를 사용하고, AAB 업로드 키의 별칭은 `HYVE_ANDROID_SIGNING_ALIAS`로 지정합니다. 만든 파일은 Cargo 타깃 디렉터리의 `debug/apk` 또는 `release/apk`에 있습니다. [Android App Bundle 공식 문서](https://developer.android.com/build/building-cmdline#bundletool-build)

macOS에서 `hyve build --debug --format ios-sim`은 공유 Xcode 스킴을 빌드해 Cargo 타깃의 `debug/ios/<앱>/simulator`에 앱을 만듭니다. `hyve build --format ipa`는 기기용 Rust 바이너리를 Xcode로 아카이브하고 export options plist의 서명 설정으로 로컬 IPA를 내보냅니다. 앱의 `[package.metadata.ios]`에 프로젝트·스킴·내보내기 설정 경로가 있으며 `HYVE_IOS_TEAM`으로 팀을 지정할 수 있습니다. 위젯 선언이 있으면 앱 서명 전에 Swift 브리지와 WidgetKit 확장을 포함합니다. 홈·잠금 화면 위젯의 공유 그룹 기본값은 `group.<번들 ID>`이며 `widget_app_group`으로 지정할 수 있습니다. 시뮬레이터 인수 스크립트가 전용 기기를 생성해 설치·시작과 실제 렌더 프레임을 검사합니다. [iOS 빌드](../apps/hyve-cli/src/ios.rs) · [시뮬레이터 인수](../apps/hyve-cli/tests/ios_simulator.py)

`hyve/shm`의 Windows 링은 현재 소유자 전용 DACL을 설정한 페이지 파일 매핑을 엽니다. 다른 프로세스는 인가된 로컬 채널에서 매핑 토큰을 받은 뒤 소유권과 레이아웃을 다시 검증합니다. `hyve/ipc`는 Windows named pipe와 Unix domain socket을 제공합니다. [Windows 프로세스 간 인수](../crates/hyve-core/tests/mapped_windows.rs)

세 레포의 `v<버전>` 릴리스 워크플로는 CI 통과 후 공개 크레이트 전체의 버전이 태그와 일치하는지 확인하고 Cargo의 의존 순서로 게시합니다. `<크레이트>-v<버전>` 태그는 해당 크레이트를 가리킵니다. [릴리스 워크플로](../.github/workflows/release.yml)


데스크톱 실행 파일은 다음과 같이 버전별 델타를 만듭니다. 키 생성 명령을 다시 실행하면 기존 키 쌍을 확인하고, 공개 키 파일만 빠졌다면 복원합니다.

```sh
hyve update keygen release.pk8 release.pub
hyve update create org.example.notes aarch64-apple-darwin 7 8 notes-v7 notes-v8 release.pk8 notes.delta
hyve update apply org.example.notes aarch64-apple-darwin 7 ./notes release.pub notes.delta
```

앱에 `hyve/update` 피처를 켜고 릴리스 공개 키를 고정하면 `hyve::update::check`에서 앱이 제공하는 피드로 새 델타를 받을 수 있습니다. 서명에 앱·타깃·이전/다음 빌드 번호·두 파일의 SHA-256·출력 길이가 포함됩니다. 설치 디렉터리에서 동시 업데이트를 직렬화하고 전체 결과를 임시 파일에서 확인한 후 원자적으로 교체합니다. Windows에서 현재 실행 파일을 잠그는 설치 형태는 별도 런처가 앱 종료 후 적용해야 합니다. macOS 앱 번들과 모바일 스토어 산출물은 각 플랫폼의 서명을 보존하는 설치 경로를 따릅니다. [업데이터 인수 테스트](../crates/hyve-update/tests/release.rs) · [BSDIFF 호환 코덱](https://docs.rs/qbsdiff/1.4.4/qbsdiff/)

Android 앱에서 `[package.metadata.android]`의 `document_picker = true`를 설정하면 `hyve build --format apk,aab`가 시스템 파일 선택 결과를 NativeActivity에 전달할 프래그먼트를 포함합니다. 사용자는 Android 시스템 파일 선택기에서 문서를 고릅니다. [패키징](../apps/hyve-cli/src/android.rs) · [프래그먼트](../apps/hyve-cli/templates/mobile/android/DocumentPicker.java)
