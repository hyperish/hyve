//! The call contract end to end, with the exact code `#[model]` and
//! `#[server]` generate written out by hand (the macro crate's expansion
//! target), a loopback transport that runs the handler on either rail, and
//! cross-rail equality checks.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Waker};

use hyve_core::{Ctx, ErrorKind, HyveError, HyveResult, Scope, Value};
use hyve_serde::__private::rkyv;
use hyve_serde::json::{Json, JsonReader, JsonWriter, from_json, to_json};
use hyve_serde::{Binary, DecodeError, Fingerprint, fnv1a, mix};
use hyve_validate::{
    Definition, FieldPath, FieldSchema, Schema, SchemaNode, SchemaRegistry, Valid, Validate,
    Violations, rules,
};

use crate::*;

fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

// ----- #[model] pub struct NewProject { #[validate(len = 1..=80)] pub title: String } -----

#[derive(Debug, Clone, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
#[rkyv(crate = hyve_serde::__private::rkyv)]
pub struct NewProject {
    pub title: String,
}

impl Json for NewProject {
    fn encode_json(&self, w: &mut JsonWriter) {
        w.begin_object();
        w.key("title");
        Json::encode_json(&self.title, w);
        w.end_object();
    }
    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        r.begin_object()?;
        let mut f0: Option<String> = None;
        while let Some(key) = r.next_key()? {
            match &*key {
                "title" => {
                    if f0.is_some() {
                        return Err(r.duplicate_field("title"));
                    }
                    f0 = Some(Json::decode_json(r)?);
                }
                _ => r.skip_value()?,
            }
        }
        Ok(NewProject {
            title: match f0 {
                Some(v) => v,
                None => <String as Json>::missing().ok_or_else(|| r.missing_field("title"))?,
            },
        })
    }
}

impl Validate for NewProject {
    fn validate_at(&self, path: &mut FieldPath, out: &mut Violations) {
        let mark = path.mark();
        path.push_field("title");
        rules::len(&self.title, 1..=80, path, out);
        Validate::validate_at(&self.title, path, out);
        path.truncate(mark);
    }
}

impl Schema for NewProject {
    fn schema(r: &mut SchemaRegistry) -> SchemaNode {
        r.define::<Self>("NewProject", |r| {
            Definition::Object(vec![FieldSchema {
                name: "title",
                json_name: "title",
                schema: <String as Schema>::schema(r),
                required: !<String as Schema>::optional(),
                constraints: vec![rules::len_constraint(1..=80)],
            }])
        })
    }
}

impl Fingerprint for NewProject {
    const FINGERPRINT: u64 = mix(
        fnv1a(b"NewProject{title}"),
        <String as Fingerprint>::FINGERPRINT,
    );
}

// ----- #[model] pub struct Project { pub title: String, pub owner: String } -----

#[derive(Debug, Clone, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
#[rkyv(crate = hyve_serde::__private::rkyv)]
pub struct Project {
    pub title: String,
    pub owner: String,
}

impl Json for Project {
    fn encode_json(&self, w: &mut JsonWriter) {
        w.begin_object();
        w.key("title");
        Json::encode_json(&self.title, w);
        w.key("owner");
        Json::encode_json(&self.owner, w);
        w.end_object();
    }
    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        r.begin_object()?;
        let (mut f0, mut f1): (Option<String>, Option<String>) = (None, None);
        while let Some(key) = r.next_key()? {
            match &*key {
                "title" => {
                    if f0.is_some() {
                        return Err(r.duplicate_field("title"));
                    }
                    f0 = Some(Json::decode_json(r)?);
                }
                "owner" => {
                    if f1.is_some() {
                        return Err(r.duplicate_field("owner"));
                    }
                    f1 = Some(Json::decode_json(r)?);
                }
                _ => r.skip_value()?,
            }
        }
        Ok(Project {
            title: f0
                .or_else(<String as Json>::missing)
                .ok_or_else(|| r.missing_field("title"))?,
            owner: f1
                .or_else(<String as Json>::missing)
                .ok_or_else(|| r.missing_field("owner"))?,
        })
    }
}

impl Schema for Project {
    fn schema(r: &mut SchemaRegistry) -> SchemaNode {
        r.define::<Self>("Project", |r| {
            Definition::Object(vec![
                FieldSchema {
                    name: "title",
                    json_name: "title",
                    schema: <String as Schema>::schema(r),
                    required: true,
                    constraints: vec![],
                },
                FieldSchema {
                    name: "owner",
                    json_name: "owner",
                    schema: <String as Schema>::schema(r),
                    required: true,
                    constraints: vec![],
                },
            ])
        })
    }
}

impl Fingerprint for Project {
    const FINGERPRINT: u64 = mix(
        fnv1a(b"Project{title,owner}"),
        mix(
            <String as Fingerprint>::FINGERPRINT,
            <String as Fingerprint>::FINGERPRINT,
        ),
    );
}

// ----- #[server(POST "/projects")]
//       pub async fn create_project(#[server_ctx] db: Ctx<Db>, input: Valid<NewProject>)
//           -> HyveResult<Project> { ... } -----

pub struct Db {
    owner: String,
}

#[doc(hidden)]
#[allow(non_camel_case_types)]
pub struct __hyve_server_create_project;

impl ServerFn for __hyve_server_create_project {
    type Request = NewProject;
    type Response = Project;
    fn descriptor() -> &'static ServerFnDescriptor {
        static DESCRIPTOR: ServerFnDescriptor = ServerFnDescriptor {
            id: concat!(module_path!(), "::", "create_project"),
            method: Method::Post,
            path: "/projects",
            args: &["input"],
            fingerprint: server_fn_fingerprint(
                <NewProject as Fingerprint>::FINGERPRINT,
                <Project as Fingerprint>::FINGERPRINT,
            ),
            request_schema: <NewProject as Schema>::schema,
            response_schema: <Project as Schema>::schema,
        };
        &DESCRIPTOR
    }
}

pub fn create_project(input: NewProject) -> ServerCall<__hyve_server_create_project> {
    let check = Validate::validate(&input).map_err(Violations::into_error);
    ServerCall::new(input).check(check)
}

async fn __hyve_server_body_create_project(
    db: Ctx<Db>,
    input: Valid<NewProject>,
) -> HyveResult<Project> {
    if input.title == "taken" {
        return Err(HyveError::domain("title_taken", "that title is taken")
            .with_detail("title", input.title.clone()));
    }
    if input.title == "boom" {
        return Err(HyveError::internal("database password is hunter2"));
    }
    Ok(Project {
        title: input.into_inner().title,
        owner: db.owner.clone(),
    })
}

impl ServerHandler for __hyve_server_create_project {
    fn handle(scope: Scope, request: NewProject) -> BoxFuture<'static, HyveResult<Project>> {
        Box::pin(async move {
            let db = scope.require::<Db>()?;
            let input = Valid::new(request)?;
            __hyve_server_body_create_project(db, input).await
        })
    }
}

// ----- #[server(GET "/ping")] pub async fn ping() -> HyveResult<()> { Ok(()) } -----

#[doc(hidden)]
#[allow(non_camel_case_types)]
pub struct __hyve_server_ping;

impl ServerFn for __hyve_server_ping {
    type Request = ();
    type Response = ();
    fn descriptor() -> &'static ServerFnDescriptor {
        static DESCRIPTOR: ServerFnDescriptor = ServerFnDescriptor {
            id: concat!(module_path!(), "::", "ping"),
            method: Method::Get,
            path: "/ping",
            args: &[],
            fingerprint: server_fn_fingerprint(
                <() as Fingerprint>::FINGERPRINT,
                <() as Fingerprint>::FINGERPRINT,
            ),
            request_schema: <() as Schema>::schema,
            response_schema: <() as Schema>::schema,
        };
        &DESCRIPTOR
    }
}

pub fn ping() -> ServerCall<__hyve_server_ping> {
    ServerCall::new(())
}

impl ServerHandler for __hyve_server_ping {
    fn handle(_scope: Scope, _request: ()) -> BoxFuture<'static, HyveResult<()>> {
        Box::pin(async { Ok(()) })
    }
}

// ----- a loopback transport: the "server" side of both rails -----

struct Loopback {
    rail: Rail,
    server: Scope,
    calls: AtomicUsize,
}

impl Loopback {
    fn new(rail: Rail) -> Arc<Self> {
        let server = Scope::root();
        server.provide(Db {
            owner: "ada".into(),
        });
        Arc::new(Loopback {
            rail,
            server,
            calls: AtomicUsize::new(0),
        })
    }

    async fn run<M: ServerHandler>(&self, call: Call) -> HyveResult<Reply> {
        let scope = self.server.child();
        match self.rail {
            Rail::Fast => {
                let body = call.encode_binary()?;
                let request =
                    M::Request::from_binary(&body).map_err(DecodeError::into_request_error)?;
                match M::handle(scope, request).await {
                    Ok(response) => Ok(Reply::Binary(response.to_binary()?)),
                    Err(e) => {
                        let body = ErrorWire::from(&e.to_public()).to_binary()?;
                        let wire = ErrorWire::from_binary(&body)
                            .map_err(DecodeError::into_response_error)?;
                        Err(wire.into())
                    }
                }
            }
            Rail::Standard => {
                let body = call.encode_json();
                let request =
                    from_json::<M::Request>(&body).map_err(DecodeError::into_request_error)?;
                match M::handle(scope, request).await {
                    Ok(response)
                        if <M::Response as Fingerprint>::FINGERPRINT
                            == <() as Fingerprint>::FINGERPRINT =>
                    {
                        drop(response);
                        Ok(Reply::NoContent)
                    }
                    Ok(response) => Ok(Reply::Json(hyve_serde::to_json_data(&response))),
                    Err(e) => {
                        let public = e.to_public();
                        // Kind and retryability travel as status code and
                        // `Retry-After` in real transports.
                        let (kind, retryable) = (public.kind(), public.is_retryable());
                        let body = to_json(&ErrorBody::from_error(&public));
                        let body = from_json::<ErrorBody>(&body)
                            .map_err(DecodeError::into_response_error)?;
                        Err(body.into_error(kind, retryable))
                    }
                }
            }
        }
    }
}

impl Transport for Loopback {
    fn call(&self, call: Call) -> BoxFuture<'_, HyveResult<Reply>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let id = call.descriptor().id;
            if id == __hyve_server_create_project::descriptor().id {
                self.run::<__hyve_server_create_project>(call).await
            } else if id == __hyve_server_ping::descriptor().id {
                self.run::<__hyve_server_ping>(call).await
            } else {
                Err(errors::transport_unavailable("unknown function"))
            }
        })
    }
}

struct Client;

impl TestClient for Client {
    fn transport(&self, rail: Rail) -> Arc<dyn Transport> {
        Loopback::new(rail)
    }
}

fn on_rail<C: std::future::IntoFuture>(rail: Rail, call: C) -> C::Output {
    block_on(Client.on(rail, call))
}

// ----- tests -----

#[test]
fn without_a_transport_the_call_fails_with_transport_unavailable() {
    let err = block_on(
        Scope::root().scoped(async { create_project(NewProject { title: "x".into() }).await }),
    )
    .unwrap_err();
    assert_eq!(
        (err.kind(), err.code()),
        (ErrorKind::Infra, codes::TRANSPORT_UNAVAILABLE)
    );
    let err = block_on(async { ping().await }).unwrap_err();
    assert_eq!(err.code(), codes::TRANSPORT_UNAVAILABLE, "no scope at all");
}

#[test]
fn invalid_valid_arguments_fail_before_the_transport() {
    let transport = Loopback::new(Rail::Fast);
    let app = Scope::root();
    provide_transport(&app, transport.clone());
    let err = block_on(app.scoped(async {
        create_project(NewProject {
            title: String::new(),
        })
        .await
    }))
    .unwrap_err();
    assert_eq!(err.code(), "invalid_request");
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn both_rails_return_the_same_value() {
    let input = NewProject {
        title: "hello".into(),
    };
    let fast = on_rail(Rail::Fast, create_project(input.clone())).unwrap();
    let standard = on_rail(Rail::Standard, create_project(input)).unwrap();
    assert_eq!(
        fast,
        Project {
            title: "hello".into(),
            owner: "ada".into()
        }
    );
    assert_eq!(fast, standard);
    assert_eq!(on_rail(Rail::Fast, ping()), Ok(()));
    assert_eq!(on_rail(Rail::Standard, ping()), Ok(()));
}

#[test]
fn both_rails_restore_the_same_errors() {
    // Domain error with details.
    let fast = on_rail(
        Rail::Fast,
        create_project(NewProject {
            title: "taken".into(),
        }),
    )
    .unwrap_err();
    let standard = on_rail(
        Rail::Standard,
        create_project(NewProject {
            title: "taken".into(),
        }),
    )
    .unwrap_err();
    assert_eq!(fast, standard);
    assert_eq!(
        (fast.kind(), fast.code()),
        (ErrorKind::Domain, "title_taken")
    );

    // Server-side validation (skip the stub's pre-check to reach the server):
    // the details, including rule parameters, survive both rails unchanged.
    let invalid = || {
        ServerCall::<__hyve_server_create_project>::new(NewProject {
            title: String::new(),
        })
    };
    let fast = on_rail(Rail::Fast, invalid()).unwrap_err();
    let standard = on_rail(Rail::Standard, invalid()).unwrap_err();
    assert_eq!(fast.code(), "invalid_request");
    assert_eq!(fast.details(), standard.details());
    assert_eq!(fast, standard);
    let Some(Value::List(fields)) = fast.details().get("fields") else {
        panic!("{fast:?}")
    };
    assert_eq!(fields[0].get("path"), Some(&Value::from("$.title")));
    assert_eq!(fields[0].get("min"), Some(&Value::Int(1)));

    // Internal errors never leak their message on either rail.
    for rail in [Rail::Fast, Rail::Standard] {
        let e = on_rail(
            rail,
            create_project(NewProject {
                title: "boom".into(),
            }),
        )
        .unwrap_err();
        assert_eq!(
            (e.kind(), e.code()),
            (ErrorKind::Internal, "internal_error")
        );
        assert!(!e.message().contains("hunter2"));
    }
}

#[test]
fn stub_fn_recovers_the_descriptor_from_the_function_item() {
    fn descriptor_of<F: StubFn<A>, A>(_: F) -> &'static ServerFnDescriptor {
        <F::Marker as ServerFn>::descriptor()
    }
    let d = descriptor_of(create_project);
    assert_eq!(d.id, "hyve_api::tests::create_project");
    assert_eq!(
        (d.method, d.path, d.args),
        (Method::Post, "/projects", &["input"][..])
    );
    assert_eq!(d.fingerprint_hex().len(), 16);
    let mut r = SchemaRegistry::new();
    assert_eq!(
        (d.request_schema)(&mut r),
        SchemaNode::Ref("NewProject".into())
    );
    assert_eq!(descriptor_of(ping).args.len(), 0);
}

#[test]
fn fingerprints_cover_request_response_and_format() {
    let a = server_fn_fingerprint(
        <NewProject as Fingerprint>::FINGERPRINT,
        <Project as Fingerprint>::FINGERPRINT,
    );
    let b = server_fn_fingerprint(
        <NewProject as Fingerprint>::FINGERPRINT,
        <() as Fingerprint>::FINGERPRINT,
    );
    let c = server_fn_fingerprint(
        <Project as Fingerprint>::FINGERPRINT,
        <NewProject as Fingerprint>::FINGERPRINT,
    );
    assert!(a != b && a != c && b != c);
}

#[test]
fn empty_requests_archive_to_zero_bytes() {
    let call = Call::new::<__hyve_server_ping>((), Default::default());
    assert_eq!(call.encode_binary().unwrap().len(), 0);
    assert_eq!(call.encode_json(), "null");
    assert_eq!(Reply::NoContent.decode::<()>(), Ok(()));
    assert_eq!(
        Reply::NoContent.decode::<Project>().unwrap_err().code(),
        codes::DECODE_FAILED
    );
}

#[test]
fn in_process_requests_are_handed_over_without_encoding() {
    let call = Call::new::<__hyve_server_create_project>(
        NewProject { title: "t".into() },
        Default::default(),
    );
    let call = call.into_request::<String>().unwrap_err();
    assert_eq!(call.into_request::<NewProject>().unwrap().title, "t");
    assert_eq!(Reply::value(5u32).decode::<u32>(), Ok(5));
    assert_eq!(
        Reply::value(5u32).decode::<u64>().unwrap_err().code(),
        codes::DECODE_FAILED
    );
}

#[test]
fn error_body_has_the_envelope_shape() {
    let e = HyveError::domain("title_taken", "taken")
        .with_request_id(hyve_core::RequestId::parse("req-1").unwrap());
    let json = to_json(&ErrorBody::from_error(&e));
    assert_eq!(
        json,
        r#"{"code":"title_taken","message":"taken","details":{},"requestId":"req-1"}"#
    );
    let back = from_json::<ErrorBody>(&json)
        .unwrap()
        .into_error(ErrorKind::Domain, false);
    assert_eq!(back, e);
}

#[test]
fn page_and_cursor_shapes() {
    let page = Page::new(
        vec![Project {
            title: "a".into(),
            owner: "b".into(),
        }],
        Some("c1".into()),
    );
    let json = to_json(&page);
    assert_eq!(
        json,
        r#"{"items":[{"title":"a","owner":"b"}],"nextCursor":"c1","hasMore":true}"#
    );
    assert_eq!(from_json::<Page<Project>>(&json).unwrap(), page);
    assert_eq!(
        Page::<Project>::from_binary(&page.to_binary().unwrap()).unwrap(),
        page
    );
    assert_eq!(
        from_json::<CursorParams>("{}").unwrap(),
        CursorParams::default()
    );
    assert_eq!(
        from_json::<CursorParams>(r#"{"after":"c1","limit":20}"#)
            .unwrap()
            .limit,
        Some(20)
    );

    let mut r = SchemaRegistry::new();
    assert_eq!(
        r.schema_of::<Page<Project>>(),
        SchemaNode::Ref("Page_Project".into())
    );
    assert_eq!(
        r.schema_of::<Page<String>>(),
        SchemaNode::Ref("Page_String".into())
    );
    assert!(r.get("Project").is_some());
}

#[test]
fn success_bodies_use_the_data_envelope() {
    use hyve_serde::{from_json_data, to_json_data};
    assert_eq!(to_json_data(&7u32), r#"{"data":7}"#);
    assert_eq!(from_json_data::<u32>(r#"{"meta":1,"data":7}"#).unwrap(), 7);
    assert!(from_json_data::<u32>(r#"{"data":7,"data":8}"#).is_err());
    assert!(from_json_data::<u32>("{}").is_err());

    let page = Page {
        items: vec![1u32, 2],
        next_cursor: Some("c".into()),
        has_more: true,
    };
    let json = to_json_data(&page);
    assert_eq!(
        json,
        r#"{"data":[1,2],"page":{"nextCursor":"c","hasMore":true}}"#
    );
    assert_eq!(from_json_data::<Page<u32>>(&json).unwrap(), page);
    let last = r#"{"data":[],"page":{"hasMore":false}}"#;
    assert_eq!(
        from_json_data::<Page<u32>>(last).unwrap(),
        Page {
            items: vec![],
            next_cursor: None,
            has_more: false
        }
    );
}
