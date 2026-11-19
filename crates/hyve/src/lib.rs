//! `hyve` — All-in-one Rust core framework and toolchain foundation for the Hyperish family.
//!
//! Acts as the single shared `ref` core across `orbic` (backend), `serge` (Web/Native UI),
//! and `opale` (graphics):
//! - `core`: errors, ULID/`Id<T>`, UTC `Timestamp`, cancellation tree, `Ctx<T>`, `SharedRingBuf`
//! - `state`: single `Copy` `Signal<T>`, `memo`, `effect`, `SyncSignal<T>`
//! - `serde`: `HYVE` binary frames with checksum, RFC 8259 JSON adapter, loss policies
//! - `validate`: `Valid<T>`, `Validate`, `HasSchema`
//! - `config` / `trace` / `parser` / `api` + `#[model]`, `#[hyve::test]`, `#[server]`, `asset!`

extern crate self as hyve;

pub use hyve_core as core;
pub use hyve_macros::{asset, model, server, test};
pub use hyve_serde as serde;
pub use hyve_state as state;
pub use hyve_validate as validate;

pub use hyve_core::{
    CancellationToken, Ctx, ErrorKind, HyveError, HyveResult, Id, RequestId, SharedRingBuf,
    Timestamp, TraceId, Ulid,
};
pub use hyve_state::{LocalSignal, Signal, SyncSignal, effect, memo, signal, sync_signal};

/// Startup configuration loader reading `.env`, `hyve.toml`, environment variables, and validating into `#[model]`.
pub mod config {
    use crate::serde::{FromJson, JsonValue};
    use crate::validate::{HasSchema, Validate};
    use hyve_core::{HyveError, HyveResult};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::Path;

    thread_local! {
        static TEST_CONFIG_SOURCE: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    /// Sets a thread-local configuration source override (safe replacement for `std::env::set_var` in tests).
    pub fn set_test_override(src: impl Into<String>) {
        TEST_CONFIG_SOURCE.with(|cell| {
            *cell.borrow_mut() = Some(src.into());
        });
    }

    /// Clears any thread-local test configuration override.
    pub fn clear_test_override() {
        TEST_CONFIG_SOURCE.with(|cell| {
            *cell.borrow_mut() = None;
        });
    }

    fn parse_kv_lines(src: &str, out: &mut HashMap<String, String>) {
        for raw_line in src.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let key = k.trim().to_ascii_lowercase();
                let mut val = v.trim();
                if ((val.starts_with('"') && val.ends_with('"'))
                    || (val.starts_with('\'') && val.ends_with('\'')))
                    && val.len() >= 2
                {
                    val = &val[1..val.len() - 1];
                }
                out.insert(key, val.to_owned());
            }
        }
    }

    fn kv_to_json<T: HasSchema>(kv: &HashMap<String, String>) -> JsonValue {
        let schema = T::schema();
        let mut obj = Vec::new();
        for field in &schema.fields {
            let snake = field.name.to_ascii_lowercase();
            let env_upper = format!("HYVE_{}", snake.to_ascii_uppercase());
            let raw_opt = kv
                .get(&snake)
                .cloned()
                .or_else(|| std::env::var(env_upper).ok())
                .or_else(|| std::env::var(snake.to_ascii_uppercase()).ok());
            let Some(raw) = raw_opt else {
                continue;
            };
            let jval = if field.ty.contains("bool") {
                match raw.as_str() {
                    "true" | "1" => JsonValue::Bool(true),
                    "false" | "0" => JsonValue::Bool(false),
                    _ => JsonValue::Str(raw),
                }
            } else if field.ty == "String" || field.ty.ends_with(":: String") {
                JsonValue::Str(raw)
            } else if let Ok(num) = raw.parse::<f64>() {
                JsonValue::Number(num)
            } else {
                JsonValue::Str(raw)
            };
            obj.push((field.name.to_owned(), jval));
        }
        JsonValue::Object(obj)
    }

    /// Parses and validates configuration of type `T` from key/value text (`.env` / `hyve.toml` style).
    pub fn from_str<T>(src: &str) -> HyveResult<T>
    where
        T: FromJson + Validate + HasSchema,
    {
        let mut kv = HashMap::new();
        parse_kv_lines(src, &mut kv);
        let jval = kv_to_json::<T>(&kv);
        let model = T::from_json(&jval)?;
        model.validate().map_err(|errs| {
            let msg = errs
                .iter()
                .map(|e| format!("{}: {}", e.field, e.message))
                .collect::<Vec<_>>()
                .join("; ");
            HyveError::domain("invalid_config", msg)
        })?;
        Ok(model)
    }

    /// Loads configuration from `hyve.toml`, `.env`, and environment variables inside `dir`.
    pub fn load_from_dir<T>(dir: impl AsRef<Path>) -> HyveResult<T>
    where
        T: FromJson + Validate + HasSchema,
    {
        let mut kv = HashMap::new();
        let dir = dir.as_ref();
        if let Ok(dotenv) = std::fs::read_to_string(dir.join(".env")) {
            parse_kv_lines(&dotenv, &mut kv);
        }
        if let Ok(toml) = std::fs::read_to_string(dir.join("hyve.toml")) {
            parse_kv_lines(&toml, &mut kv);
        }
        let jval = kv_to_json::<T>(&kv);
        let model = T::from_json(&jval)?;
        model.validate().map_err(|errs| {
            let msg = errs
                .iter()
                .map(|e| format!("{}: {}", e.field, e.message))
                .collect::<Vec<_>>()
                .join("; ");
            HyveError::domain("invalid_config", msg)
        })?;
        Ok(model)
    }

    /// Loads and validates `T` from test override (if active), `./.env`, `./hyve.toml`, and env vars.
    pub fn load<T>() -> HyveResult<T>
    where
        T: FromJson + Validate + HasSchema,
    {
        if let Some(src) = TEST_CONFIG_SOURCE.with(|cell| cell.borrow().clone()) {
            return from_str(&src);
        }
        let cwd = std::env::current_dir()
            .map_err(|e| HyveError::infra("config_io_error", e.to_string(), false))?;
        load_from_dir(&cwd)
    }
}

/// Distributed structured tracing spans propagating `TraceId` and `RequestId`.
pub mod trace {
    use hyve_core::{RequestId, Timestamp, TraceId};

    /// Structured trace span linking UI, backend RPCs, database queries, and jobs.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct TraceSpan {
        /// Distributed trace identifier.
        pub trace_id: TraceId,
        /// Per-request correlation identifier.
        pub request_id: RequestId,
        /// Operation or handler name.
        pub name: String,
        /// Span creation timestamp.
        pub started_at: Timestamp,
        /// Completed duration in microseconds (if finished).
        pub duration_micros: Option<u64>,
    }

    impl TraceSpan {
        /// Starts a new trace span with fresh `TraceId` and `RequestId`.
        #[must_use]
        pub fn start(name: impl Into<String>) -> Self {
            Self {
                trace_id: TraceId::generate(),
                request_id: RequestId::generate(),
                name: name.into(),
                started_at: Timestamp::now(),
                duration_micros: None,
            }
        }

        /// Starts a child span preserving the same `TraceId`.
        #[must_use]
        pub fn child(&self, name: impl Into<String>) -> Self {
            Self {
                trace_id: self.trace_id.clone(),
                request_id: RequestId::generate(),
                name: name.into(),
                started_at: Timestamp::now(),
                duration_micros: None,
            }
        }

        /// Marks the span completed with an elapsed duration in microseconds.
        pub fn finish(&mut self, duration_micros: u64) {
            self.duration_micros = Some(duration_micros);
        }
    }
}

/// Zero-copy slice parser primitives and structured diagnostics (`error` + `span` + `fix hint`).
pub mod parser {
    use std::fmt;

    /// Lightweight zero-copy string slice cursor for DSL and schema parsers.
    #[derive(Debug, Clone, Copy)]
    pub struct Cursor<'a> {
        src: &'a str,
        pos: usize,
    }

    impl<'a> Cursor<'a> {
        /// Creates a parser cursor at byte index 0 of `src`.
        #[must_use]
        pub const fn new(src: &'a str) -> Self {
            Self { src, pos: 0 }
        }

        /// Consumes leading ASCII whitespace and returns the trimmed prefix.
        pub fn skip_ws(&mut self) -> &'a str {
            let start = self.pos;
            while let Some(b) = self.src.as_bytes().get(self.pos) {
                if b.is_ascii_whitespace() {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            &self.src[start..self.pos]
        }

        /// Attempts to consume an exact literal prefix.
        pub fn eat(&mut self, literal: &str) -> bool {
            if self.rest().starts_with(literal) {
                self.pos += literal.len();
                true
            } else {
                false
            }
        }

        /// Consumes an ASCII identifier `[A-Za-z_][A-Za-z0-9_]*`.
        pub fn take_ident(&mut self) -> Option<&'a str> {
            let bytes = self.rest().as_bytes();
            let &first = bytes.first()?;
            if !(first.is_ascii_alphabetic() || first == b'_') {
                return None;
            }
            let mut end = 1;
            while let Some(&b) = bytes.get(end) {
                if b.is_ascii_alphanumeric() || b == b'_' {
                    end += 1;
                } else {
                    break;
                }
            }
            let ident = &self.rest()[..end];
            self.pos += end;
            Some(ident)
        }

        /// Returns remaining unparsed input.
        #[must_use]
        pub fn rest(&self) -> &'a str {
            &self.src[self.pos..]
        }

        /// Returns current byte offset.
        #[must_use]
        pub const fn position(&self) -> usize {
            self.pos
        }
    }

    /// Actionable diagnostic message following "Error IS Docs": (what failed, macro/span, how to fix).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Diagnostic {
        /// What went wrong.
        pub message: String,
        /// Where (macro/file/line/span) the error occurred.
        pub span_label: String,
        /// Actionable fix hint and example.
        pub fix_hint: String,
    }

    impl Diagnostic {
        /// Constructs a three-part diagnostic.
        #[must_use]
        pub fn new(
            message: impl Into<String>,
            span_label: impl Into<String>,
            fix_hint: impl Into<String>,
        ) -> Self {
            Self {
                message: message.into(),
                span_label: span_label.into(),
                fix_hint: fix_hint.into(),
            }
        }
    }

    impl fmt::Display for Diagnostic {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                f,
                "error: {}\n  --> {}\n  = help: {}",
                self.message, self.span_label, self.fix_hint
            )
        }
    }
}

/// Target-neutral API contracts, compile-time assets, inline server descriptors, and async runner.
pub mod api {
    use crate::serde::{
        BinaryDecode, BinaryEncode, FromJson, ToJson, from_bytes, from_json_str, to_bytes,
        to_json_string,
    };
    use hyve_core::{HyveError, HyveResult};
    use std::any::{Any, TypeId};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::future::Future;
    use std::path::{Path, PathBuf};
    use std::pin::Pin;
    use std::sync::{Arc, Mutex, OnceLock};
    use std::task::{Context, Poll, Waker};

    /// Compile-time declared asset reference created via [`crate::asset!`].
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct AssetRef {
        logical_path: &'static str,
        manifest_dir: &'static str,
    }

    impl AssetRef {
        /// Creates an asset reference anchored at `manifest_dir`.
        #[must_use]
        pub const fn new(logical_path: &'static str, manifest_dir: &'static str) -> Self {
            Self {
                logical_path,
                manifest_dir,
            }
        }

        /// Returns the normalized logical asset path (e.g. `"/shaders/pbr.wgsl"`).
        #[must_use]
        pub const fn path(&self) -> &'static str {
            self.logical_path
        }

        /// Resolves the physical candidate path inside `CARGO_MANIFEST_DIR`.
        #[must_use]
        pub fn resolved_path(&self) -> PathBuf {
            let trimmed = self.logical_path.trim_start_matches('/');
            let direct = Path::new(self.manifest_dir).join(trimmed);
            if direct.exists() {
                return direct;
            }
            let under_assets = Path::new(self.manifest_dir).join("assets").join(trimmed);
            if under_assets.exists() {
                return under_assets;
            }
            direct
        }

        /// Returns `true` if the asset file exists on disk relative to `CARGO_MANIFEST_DIR`.
        #[must_use]
        pub fn exists(&self) -> bool {
            let trimmed = self.logical_path.trim_start_matches('/');
            let root = Path::new(self.manifest_dir);
            root.join(trimmed).exists() || root.join("assets").join(trimmed).exists()
        }
    }

    /// Selects between Standard External Compatibility Rail and Fast Vertical Binary Rail.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum DualRailMode {
        /// Standard RFC 8259 JSON / HTTP-WebSocket rail for external compatibility.
        StandardJson,
        /// Zero-copy-oriented `hyve-serde` binary rail for `serge` <-> `orbic` and MSA.
        FastBinary,
    }

    /// Context passed into an invoked `#[server]` handler to resolve extractors and payloads cleanly.
    pub struct ServerInvocationContext {
        providers: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
        payload_bytes: Vec<u8>,
        mode: DualRailMode,
    }

    impl ServerInvocationContext {
        /// Creates a new server invocation context with payload bytes and negotiated rail mode.
        #[must_use]
        pub fn new(payload_bytes: Vec<u8>, mode: DualRailMode) -> Self {
            Self {
                providers: HashMap::new(),
                payload_bytes,
                mode,
            }
        }

        /// Inserts a copyable/cloneable extractor value (such as `Ctx<Db>` or `Ctx<CurrentUser>`).
        pub fn insert<T: Clone + Send + Sync + 'static>(&mut self, value: T) {
            self.providers.insert(TypeId::of::<T>(), Box::new(value));
        }

        /// Resolves a provided extractor type `T`.
        pub fn resolve<T: Clone + 'static>(&self) -> HyveResult<T> {
            self.providers
                .get(&TypeId::of::<T>())
                .and_then(|b| b.downcast_ref::<T>())
                .cloned()
                .ok_or_else(|| {
                    HyveError::internal(format!(
                        "missing provided context for {}",
                        std::any::type_name::<T>()
                    ))
                })
        }

        /// Decodes the incoming payload according to the negotiated [`DualRailMode`] or frame header.
        pub fn decode_payload<T: BinaryDecode + FromJson>(&self) -> HyveResult<T> {
            if self.mode == DualRailMode::FastBinary || self.payload_bytes.starts_with(b"HYVE") {
                from_bytes(&self.payload_bytes)
            } else {
                let s = std::str::from_utf8(&self.payload_bytes).map_err(|_| {
                    HyveError::domain("invalid_request", "payload must be valid UTF-8")
                })?;
                from_json_str(s)
            }
        }

        /// Encodes the outgoing response according to the negotiated [`DualRailMode`].
        pub fn encode_response<T: BinaryEncode + ToJson>(&self, value: &T) -> HyveResult<Vec<u8>> {
            match self.mode {
                DualRailMode::FastBinary => Ok(to_bytes(value)),
                DualRailMode::StandardJson => Ok(to_json_string(value).into_bytes()),
            }
        }
    }

    /// Type-erased async server function handler closure.
    pub type BoxServerFuture = Pin<Box<dyn Future<Output = HyveResult<Vec<u8>>> + 'static>>;
    /// Invokable handler callback carried inside [`EndpointDescriptor`].
    pub type ServerHandlerFn =
        Arc<dyn Fn(ServerInvocationContext) -> BoxServerFuture + Send + Sync>;

    /// Metadata and handler descriptor emitted by `#[server(METHOD "/path")]`.
    #[derive(Clone)]
    pub struct EndpointDescriptor {
        /// Rust symbol name of the server function.
        pub name: &'static str,
        /// HTTP/RPC method (`"GET"`, `"POST"`, etc.).
        pub method: &'static str,
        /// Endpoint path (`"/projects"`, etc.).
        pub path: &'static str,
        /// Type-erased async handler invoked by `orbic`.
        pub handler: ServerHandlerFn,
    }

    impl EndpointDescriptor {
        /// Constructs a new [`EndpointDescriptor`].
        #[must_use]
        pub fn new(
            name: &'static str,
            method: &'static str,
            path: &'static str,
            handler: ServerHandlerFn,
        ) -> Self {
            Self {
                name,
                method,
                path,
                handler,
            }
        }
    }

    /// Trait implemented by future types emitted from `#[server]` so `.handle(fn_item)` works directly.
    pub trait IntoEndpointDescriptor {
        /// Returns the static [`EndpointDescriptor`] associated with this endpoint call future.
        fn endpoint_descriptor() -> EndpointDescriptor;
    }

    /// Enables a `#[server]` endpoint descriptor to be mounted directly via `.handle(...)`.
    pub trait IntoServerEndpoint<Marker> {
        /// Extracts the [`EndpointDescriptor`] for registration into a backend component.
        fn into_endpoint(self) -> EndpointDescriptor;
    }

    /// Marker distinguishing direct [`EndpointDescriptor`] values from `#[server]` function items.
    pub struct DirectEndpointMarker;

    impl IntoServerEndpoint<DirectEndpointMarker> for EndpointDescriptor {
        fn into_endpoint(self) -> EndpointDescriptor {
            self
        }
    }

    impl<F, Req, Fut> IntoServerEndpoint<fn(Req) -> Fut> for F
    where
        F: Fn(Req) -> Fut + 'static,
        Fut: IntoEndpointDescriptor,
    {
        fn into_endpoint(self) -> EndpointDescriptor {
            Fut::endpoint_descriptor()
        }
    }

    /// Pluggable Dual-Rail client transport used when calling `#[server]` functions directly on the client.
    pub trait ServerCallTransport: Send + Sync {
        /// Preferred wire encoding for this transport.
        fn mode(&self) -> DualRailMode {
            DualRailMode::FastBinary
        }

        /// Executes an RPC call against `method` + `path` and returns raw encoded bytes.
        fn call_raw(
            &self,
            method: &str,
            path: &str,
            payload_bytes: &[u8],
        ) -> Pin<Box<dyn Future<Output = HyveResult<Vec<u8>>> + 'static>>;
    }

    fn global_transport() -> &'static Mutex<Option<Arc<dyn ServerCallTransport>>> {
        static GLOBAL: OnceLock<Mutex<Option<Arc<dyn ServerCallTransport>>>> = OnceLock::new();
        GLOBAL.get_or_init(|| Mutex::new(None))
    }

    thread_local! {
        static THREAD_TRANSPORT: RefCell<Option<Arc<dyn ServerCallTransport>>> = const { RefCell::new(None) };
    }

    /// Registers a thread-local client transport (used in deterministic unit tests and UI event handlers).
    pub fn set_thread_transport(transport: Option<Arc<dyn ServerCallTransport>>) {
        THREAD_TRANSPORT.with(|cell| {
            *cell.borrow_mut() = transport;
        });
    }

    /// Registers a process-wide default transport for `#[server]` client calls.
    pub fn set_global_transport(transport: Option<Arc<dyn ServerCallTransport>>) {
        if let Ok(mut guard) = global_transport().lock() {
            *guard = transport;
        }
    }

    fn active_transport() -> Option<Arc<dyn ServerCallTransport>> {
        if let Some(t) = THREAD_TRANSPORT.with(|cell| cell.borrow().clone()) {
            return Some(t);
        }
        global_transport().lock().ok().and_then(|g| g.clone())
    }

    /// Executes an inline `#[server]` RPC call over the active Dual-Rail transport.
    pub async fn dispatch_server_call<Req, Res>(
        method: &'static str,
        path: &'static str,
        payload: &Req,
    ) -> HyveResult<Res>
    where
        Req: BinaryEncode + ToJson,
        Res: BinaryDecode + FromJson,
    {
        let transport = active_transport().ok_or_else(|| {
            HyveError::infra(
                "no_client_transport",
                format!("no client transport registered for #[server] call {method} {path}"),
                false,
            )
        })?;
        let mode = transport.mode();
        let req_bytes = match mode {
            DualRailMode::FastBinary => to_bytes(payload),
            DualRailMode::StandardJson => to_json_string(payload).into_bytes(),
        };
        let resp_bytes = transport.call_raw(method, path, &req_bytes).await?;
        if mode == DualRailMode::FastBinary || resp_bytes.starts_with(b"HYVE") {
            from_bytes(&resp_bytes)
        } else {
            let s = std::str::from_utf8(&resp_bytes).map_err(|_| {
                HyveError::infra("invalid_response", "response must be valid UTF-8", false)
            })?;
            from_json_str(s)
        }
    }

    /// Helper trait unifying `()` and `Result<(), E>` test return types in `#[hyve::test]`.
    pub trait IntoTestOutcome {
        /// Asserts or unwraps the test outcome.
        fn into_test_outcome(self);
    }

    impl IntoTestOutcome for () {
        fn into_test_outcome(self) {}
    }

    impl<E: std::fmt::Debug> IntoTestOutcome for Result<(), E> {
        fn into_test_outcome(self) {
            self.expect("test returned Err");
        }
    }

    /// Lightweight zero-`unsafe`, zero-dependency future runner for single-threaded MVP async flows & tests.
    pub fn block_on<F: Future>(future: F) -> F::Output {
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut pinned = Box::pin(future);
        loop {
            match pinned.as_mut().poll(&mut cx) {
                Poll::Ready(out) => return out,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }
}

/// Ergonomic single-import prelude (`use hyve::prelude::*;`).
pub mod prelude {
    pub use crate::{
        CancellationToken, Ctx, ErrorKind, HyveError, HyveResult, Id, LocalSignal, RequestId,
        SharedRingBuf, Signal, SyncSignal, Timestamp, TraceId, Ulid, api, asset, config, effect,
        memo, model, parser, server, signal, sync_signal, trace,
    };
    pub use hyve_serde::{
        BinaryDecode, BinaryEncode, CodecAdapter, FromJson, JsonCodecAdapter, JsonValue,
        LossPolicy, ToJson, from_bytes, from_json_str, to_bytes, to_json_string,
    };
    pub use hyve_validate::{
        FieldConstraint, FieldSchema, HasSchema, ModelSchema, Valid, Validate, ValidationError,
    };
}
