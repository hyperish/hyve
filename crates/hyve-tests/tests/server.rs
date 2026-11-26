//! `#[server]`: descriptors, stubs, pre-checks, and (with `--features server`)
//! the handler bodies on both rails through a loopback transport.

use std::future::Future;
use std::marker::PhantomData;

use hyve::api::{Method, ServerFn, ServerFnDescriptor, ServerHandler, StubFn, codes};
use hyve::validate::{SchemaNode, SchemaRegistry};
use hyve::{ErrorKind, Id, Scope};
use hyve_tests::{CreateProject, Tag, create_project, get_project, logout, rename_project};

fn descriptor<F: StubFn<A>, A>(_: F) -> &'static ServerFnDescriptor {
    <F::Marker as ServerFn>::descriptor()
}

fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

#[test]
fn descriptors_carry_identity_binding_and_argument_names() {
    let d = descriptor(create_project);
    assert_eq!(d.id, "hyve_tests::create_project");
    assert_eq!(
        (d.method, d.path, d.args),
        (Method::Post, "/projects", &["input"][..])
    );

    let d = descriptor(get_project);
    assert_eq!(
        (d.method, d.path, d.args),
        (Method::Get, "/projects/{id}", &["id"][..])
    );

    let d = descriptor(rename_project);
    assert_eq!((d.method, d.args), (Method::Patch, &["id", "title"][..]));
    let mut r = SchemaRegistry::new();
    assert_eq!(
        (d.request_schema)(&mut r),
        SchemaNode::Ref("RenameProjectRequest".into())
    );
    assert_eq!(
        (d.response_schema)(&mut r),
        SchemaNode::Ref("Project".into())
    );

    let d = descriptor(logout);
    assert_eq!((d.method, d.args.len()), (Method::Delete, 0));
    assert_eq!((d.request_schema)(&mut r), SchemaNode::Null);

    let all = [
        descriptor(create_project).fingerprint,
        descriptor(get_project).fingerprint,
        descriptor(rename_project).fingerprint,
        descriptor(logout).fingerprint,
    ];
    for (i, a) in all.iter().enumerate() {
        assert!(
            all[i + 1..].iter().all(|b| b != a),
            "fingerprints differ per contract"
        );
    }
}

/// Whether `M` has a handler body in this build (autoref specialization; one
/// of the two traits is unused in each build).
struct Probe<M>(PhantomData<M>);
#[allow(dead_code)]
trait HasHandler {
    fn has_handler(&self) -> bool {
        true
    }
}
impl<M: ServerHandler> HasHandler for Probe<M> {}
#[allow(dead_code)]
trait NoHandler {
    fn has_handler(&self) -> bool {
        false
    }
}
impl<M> NoHandler for &Probe<M> {}

#[test]
// The borrow selects between the two traits; it is only "needless" in the
// build where the handler exists.
#[allow(clippy::needless_borrow)]
fn handler_bodies_exist_only_with_the_server_feature() {
    let has = (&Probe::<hyve_tests::__hyve_server_create_project>(PhantomData)).has_handler();
    assert_eq!(has, cfg!(feature = "server"));
}

#[test]
fn stubs_need_a_transport_and_check_valid_arguments_first() {
    let scope = Scope::root();
    let err = block_on(scope.scoped(async { logout().await })).unwrap_err();
    assert_eq!(
        (err.kind(), err.code()),
        (ErrorKind::Infra, codes::TRANSPORT_UNAVAILABLE)
    );

    // Pre-checks fail before the transport is looked up.
    let err = block_on(scope.scoped(async {
        create_project(CreateProject {
            title: String::new(),
        })
        .await
    }));
    assert_eq!(err.unwrap_err().code(), "invalid_request");
    let err = block_on(scope.scoped(async {
        rename_project(
            Id::new(),
            Tag {
                name: "x".repeat(20),
            },
        )
        .await
    }));
    assert_eq!(err.unwrap_err().code(), "invalid_request");
}

#[cfg(feature = "server")]
mod with_server_feature {
    use std::future::IntoFuture;
    use std::sync::Arc;

    use hyve::api::{
        BoxFuture, Call, ErrorBody, ErrorWire, Rail, Reply, ServerCall, TestClient, Transport,
    };
    use hyve::serde::json::{from_json, to_json};
    use hyve::serde::{Binary, DecodeError, Fingerprint};
    use hyve::{HyveResult, Id, Scope};
    use hyve_tests::{
        __hyve_server_create_project, __hyve_server_get_project, __hyve_server_logout,
        __hyve_server_rename_project, CreateProject, Db, Tag, create_project, get_project, logout,
        rename_project,
    };

    use super::{ServerHandler, block_on, descriptor};

    /// Runs the handlers on a chosen rail, encoding like a real adapter would.
    struct Loopback {
        rail: Rail,
        server: Scope,
    }

    impl Loopback {
        async fn run<M: ServerHandler>(&self, call: Call) -> HyveResult<Reply> {
            let fast = self.rail == Rail::Fast;
            let request = if fast {
                M::Request::from_binary(&call.encode_binary()?)
            } else {
                from_json::<M::Request>(&call.encode_json())
            };
            let request = request.map_err(DecodeError::into_request_error)?;
            match M::handle(self.server.child(), request).await {
                Ok(v) if fast => Ok(Reply::Binary(v.to_binary()?)),
                Ok(_) if M::Response::FINGERPRINT == <() as Fingerprint>::FINGERPRINT => {
                    Ok(Reply::NoContent)
                }
                Ok(v) => Ok(Reply::Json(hyve::serde::to_json_data(&v))),
                Err(e) if fast => {
                    let bytes = ErrorWire::from(&e.to_public()).to_binary()?;
                    Err(ErrorWire::from_binary(&bytes)
                        .map_err(DecodeError::into_response_error)?
                        .into())
                }
                Err(e) => {
                    // Kind and retryability travel as status code and
                    // `Retry-After` in real transports.
                    let p = e.to_public();
                    let body = from_json::<ErrorBody>(&to_json(&ErrorBody::from_error(&p)))
                        .map_err(DecodeError::into_response_error)?;
                    Err(body.into_error(p.kind(), p.is_retryable()))
                }
            }
        }
    }

    impl Transport for Loopback {
        fn call(&self, call: Call) -> BoxFuture<'_, HyveResult<Reply>> {
            Box::pin(async move {
                let id = call.descriptor().id;
                if id == descriptor(create_project).id {
                    self.run::<__hyve_server_create_project>(call).await
                } else if id == descriptor(get_project).id {
                    self.run::<__hyve_server_get_project>(call).await
                } else if id == descriptor(rename_project).id {
                    self.run::<__hyve_server_rename_project>(call).await
                } else {
                    self.run::<__hyve_server_logout>(call).await
                }
            })
        }
    }

    struct Client;

    impl TestClient for Client {
        fn transport(&self, rail: Rail) -> Arc<dyn Transport> {
            let server = Scope::root();
            server.provide(Db {
                owner: "ada".into(),
            });
            Arc::new(Loopback { rail, server })
        }
    }

    fn both<C: IntoFuture>(make: impl Fn() -> C) -> (C::Output, C::Output) {
        (
            block_on(Client.on(Rail::Fast, make())),
            block_on(Client.on(Rail::Standard, make())),
        )
    }

    #[test]
    fn handlers_report_their_context_needs() {
        use hyve::api::ContextNeed;
        assert_eq!(
            __hyve_server_create_project::context(),
            [ContextNeed::of::<Db>()]
        );
        assert!(__hyve_server_get_project::context().is_empty());
    }

    #[test]
    fn handlers_answer_the_same_on_both_rails() {
        let (fast, standard) = both(|| {
            create_project(CreateProject {
                title: "Atrium".into(),
            })
        });
        let project = fast.clone().unwrap();
        assert_eq!(
            (project.owner.as_str(), project.title.as_str()),
            ("ada", "Atrium")
        );
        assert_eq!(fast, standard);

        let id = Id::new();
        let (fast, standard) = both(|| rename_project(id, Tag { name: "new".into() }));
        assert_eq!(
            fast.as_ref().map(|p| (p.id, p.title.as_str())),
            Ok((id, "new"))
        );
        assert_eq!(fast, standard);

        assert_eq!(both(logout), (Ok(()), Ok(())));
    }

    #[test]
    fn errors_are_the_same_on_both_rails() {
        let (fast, standard) = both(|| {
            create_project(CreateProject {
                title: "taken".into(),
            })
        });
        assert_eq!(fast.as_ref().unwrap_err().code(), "title_taken");
        assert_eq!(fast, standard);

        let (fast, standard) = both(|| get_project(Id::new()));
        assert_eq!(fast.as_ref().unwrap_err().code(), "project_not_found");
        assert_eq!(fast, standard);

        // Server-side validation, bypassing the stub's pre-check.
        let (fast, standard) = both(|| {
            ServerCall::<__hyve_server_create_project>::new(CreateProject {
                title: String::new(),
            })
        });
        assert_eq!(fast.as_ref().unwrap_err().code(), "invalid_request");
        assert_eq!(fast, standard);
    }
}
