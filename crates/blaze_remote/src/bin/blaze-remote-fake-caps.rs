//! A gRPC server whose `Capabilities.GetCapabilities` fails with a status given on the command
//! line, and which answers every other method with UNIMPLEMENTED (review 2026-11-19 N3:
//! Bazel's remote worker cannot answer GetCapabilities with NOT_FOUND).
//!
//! usage: blaze-remote-fake-caps --port N --code CODE [--message TEXT] [--exit-after-capabilities]
//!
//! CODE is a gRPC status code name (`NOT_FOUND`, `PERMISSION_DENIED`, ...); with `OK` the server
//! answers the capabilities of a cache (SHA256, action cache updates) and, with
//! `--exit-after-capabilities`, exits shortly after the first answer, so that the client's later
//! calls meet a closed port (review 2026-11-19 R3: the per-action `Remote Cache:` warning). The
//! server listens
//! on 127.0.0.1 and [::1] (when available) until it is killed. Used by
//! `scripts/remote/caps-matrix`.

use std::convert::Infallible;
use std::process::ExitCode;
use std::task::{Context, Poll};

use blaze_remote::api_version::ApiVersion;
use blaze_remote::grpc::stubs::capabilities_server::{Capabilities, CapabilitiesServer};
use blaze_remote::reapi::{
    ActionCacheUpdateCapabilities, CacheCapabilities, GetCapabilitiesRequest, ServerCapabilities,
    digest_function, symlink_absolute_path_strategy,
};
use futures::future::BoxFuture;
use tonic::body::Body;
use tonic::server::NamedService;
use tonic::transport::server::TcpIncoming;
use tonic::{Request, Response, Status};
use tower::Service;

const USAGE: &str = "usage: blaze-remote-fake-caps --port N --code CODE [--message TEXT] [--exit-after-capabilities]\n\
\n\
Answers Capabilities.GetCapabilities with the status CODE (a gRPC code name such as NOT_FOUND)\n\
and TEXT as its description, and every other method with UNIMPLEMENTED. With CODE OK it\n\
answers the capabilities of a cache (SHA256, action cache updates enabled) instead, and with\n\
--exit-after-capabilities it exits 0.5 s after that first answer. Listens on 127.0.0.1:N and\n\
[::1]:N until killed.";

/// The Capabilities service: fails with the configured status.
struct FailingCapabilities {
    code: tonic::Code,
    message: String,
    exit_after_capabilities: bool,
}

#[tonic::async_trait]
impl Capabilities for FailingCapabilities {
    async fn get_capabilities(
        &self,
        _request: Request<GetCapabilitiesRequest>,
    ) -> Result<Response<ServerCapabilities>, Status> {
        if self.code != tonic::Code::Ok {
            return Err(Status::new(self.code, self.message.clone()));
        }
        if self.exit_after_capabilities {
            tokio::spawn(async {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                eprintln!("blaze-remote-fake-caps: exiting after GetCapabilities");
                std::process::exit(0);
            });
        }
        Ok(Response::new(ServerCapabilities {
            low_api_version: Some(ApiVersion::LOW.to_sem_ver()),
            high_api_version: Some(ApiVersion::HIGH.to_sem_ver()),
            cache_capabilities: Some(CacheCapabilities {
                digest_functions: vec![digest_function::Value::Sha256 as i32],
                action_cache_update_capabilities: Some(ActionCacheUpdateCapabilities {
                    update_enabled: true,
                }),
                symlink_absolute_path_strategy: symlink_absolute_path_strategy::Value::Allowed
                    as i32,
                ..Default::default()
            }),
            ..Default::default()
        }))
    }
}

/// Routes the Capabilities service to [`FailingCapabilities`]; any other path is UNIMPLEMENTED
/// (as grpc-java's server answers a method it does not know).
#[derive(Clone)]
struct Router {
    capabilities: CapabilitiesServer<FailingCapabilities>,
}

impl Service<http::Request<Body>> for Router {
    type Response = http::Response<Body>;
    type Error = Infallible;
    type Future = BoxFuture<'static, Result<http::Response<Body>, Infallible>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: http::Request<Body>) -> Self::Future {
        let path = request.uri().path().to_string();
        let prefix = format!(
            "/{}/",
            <CapabilitiesServer<FailingCapabilities> as NamedService>::NAME
        );
        if path.starts_with(&prefix) {
            let mut capabilities = self.capabilities.clone();
            return Box::pin(async move { capabilities.call(request).await });
        }
        Box::pin(async move {
            Ok(Status::unimplemented(format!(
                "Method not found: {}",
                path.trim_start_matches('/')
            ))
            .into_http())
        })
    }
}

fn parse_code(name: &str) -> Option<tonic::Code> {
    use tonic::Code::*;
    Some(match name {
        "OK" => Ok,
        "CANCELLED" => Cancelled,
        "UNKNOWN" => Unknown,
        "INVALID_ARGUMENT" => InvalidArgument,
        "DEADLINE_EXCEEDED" => DeadlineExceeded,
        "NOT_FOUND" => NotFound,
        "ALREADY_EXISTS" => AlreadyExists,
        "PERMISSION_DENIED" => PermissionDenied,
        "RESOURCE_EXHAUSTED" => ResourceExhausted,
        "FAILED_PRECONDITION" => FailedPrecondition,
        "ABORTED" => Aborted,
        "OUT_OF_RANGE" => OutOfRange,
        "UNIMPLEMENTED" => Unimplemented,
        "INTERNAL" => Internal,
        "UNAVAILABLE" => Unavailable,
        "DATA_LOSS" => DataLoss,
        "UNAUTHENTICATED" => Unauthenticated,
        _ => return None,
    })
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "help" || args[0] == "--help" || args[0] == "-h" {
        println!("{USAGE}");
        return if args.is_empty() {
            ExitCode::from(2)
        } else {
            ExitCode::SUCCESS
        };
    }
    let mut port: Option<u16> = None;
    let mut code: Option<tonic::Code> = None;
    let mut message = String::new();
    let mut exit_after_capabilities = false;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--exit-after-capabilities" {
            exit_after_capabilities = true;
            i += 1;
            continue;
        }
        let value = args.get(i + 1).cloned();
        match (args[i].as_str(), value) {
            ("--port", Some(v)) => port = v.parse().ok(),
            ("--code", Some(v)) => match parse_code(&v) {
                Some(c) => code = Some(c),
                None => {
                    eprintln!("unknown status code: {v}");
                    return ExitCode::from(2);
                }
            },
            ("--message", Some(v)) => message = v,
            (other, _) => {
                eprintln!("bad argument: {other}\n{USAGE}");
                return ExitCode::from(2);
            }
        }
        i += 2;
    }
    let (Some(port), Some(code)) = (port, code) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let router = Router {
        capabilities: CapabilitiesServer::new(FailingCapabilities {
            code,
            message,
            exit_after_capabilities,
        }),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    runtime.block_on(async move {
        let mut servers = Vec::new();
        for addr in [format!("127.0.0.1:{port}"), format!("[::1]:{port}")] {
            match tokio::net::TcpListener::bind(&addr).await {
                Ok(listener) => {
                    let router = router.clone();
                    servers.push(tokio::spawn(async move {
                        tonic::transport::Server::builder()
                            .serve_with_incoming(router, TcpIncoming::from(listener))
                            .await
                    }));
                }
                Err(e) if addr.starts_with('[') => eprintln!("not listening on {addr}: {e}"),
                Err(e) => {
                    eprintln!("cannot listen on {addr}: {e}");
                    return ExitCode::from(1);
                }
            }
        }
        eprintln!("blaze-remote-fake-caps: serving on port {port}");
        for server in servers {
            if let Ok(Err(e)) = server.await {
                eprintln!("server failed: {e}");
                return ExitCode::from(1);
            }
        }
        ExitCode::SUCCESS
    })
}
