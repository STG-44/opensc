pub mod guard;

use axum::{
    Router,
    body::{Body, Bytes},
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use guard::{Guard, Policy, Rejection, Traffic};
use percent_encoding::percent_decode_str;
use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    pin::Pin,
    sync::{Arc, RwLock},
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::sync::OwnedSemaphorePermit;
use tower_http::services::ServeDir;

#[derive(Clone)]
pub struct WebState {
    pub guard: Arc<Guard>,
    pub trusted_proxies: Arc<RwLock<HashSet<IpAddr>>>,
    pub proxy_required: bool,
}

impl WebState {
    pub fn new(policy: Policy, proxies: HashSet<IpAddr>, proxy_required: bool) -> Self {
        Self {
            guard: Arc::new(Guard::new(policy)),
            trusted_proxies: Arc::new(RwLock::new(proxies)),
            proxy_required,
        }
    }

    pub fn replace_proxies(&self, proxies: HashSet<IpAddr>) {
        *self
            .trusted_proxies
            .write()
            .unwrap_or_else(|p| p.into_inner()) = proxies;
    }

    fn client_ip(&self, peer: IpAddr, headers: &HeaderMap) -> Result<IpAddr, StatusCode> {
        let trusted = self
            .trusted_proxies
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .contains(&peer);
        if trusted {
            // Only the immediate trusted proxy may supply this single-address header.
            // X-Forwarded-For and Forwarded are intentionally never used as identity.
            let values = headers.get_all("x-real-ip");
            if values.iter().count() != 1 {
                return Err(StatusCode::BAD_REQUEST);
            }
            return values
                .iter()
                .next()
                .and_then(|v| v.to_str().ok())
                .and_then(|value| value.parse().ok())
                .ok_or(StatusCode::BAD_REQUEST);
        }
        if self.proxy_required {
            Err(StatusCode::FORBIDDEN)
        } else {
            Ok(peer)
        }
    }
}

pub fn app(root: PathBuf, state: WebState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "opensc-ready\n" }))
        .fallback_service(ServeDir::new(root).append_index_html_on_directories(true))
        .layer(middleware::from_fn_with_state(state, protect))
}

async fn protect(State(state): State<WebState>, request: Request, next: Next) -> Response {
    let raw_path = request.uri().path();
    let decoded = match percent_decode_str(raw_path).decode_utf8() {
        Ok(path) => {
            // ServeDir treats repeated slashes as equivalent filesystem paths.
            // Budget and cache classification must use the same canonical form.
            let mut normalized = format!(
                "/{}",
                path.split('/')
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join("/")
            );
            if path.ends_with('/') && normalized != "/" {
                normalized.push('/');
            }
            normalized
        }
        Err(_) => {
            return finish(
                error(StatusCode::BAD_REQUEST, "Invalid path.\n", None),
                false,
            );
        }
    };
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip());
    let Some(peer) = peer else {
        return finish(
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Client address unavailable.\n",
                None,
            ),
            false,
        );
    };
    // The container healthcheck remains available even during a rate-limit incident.
    if decoded == "/healthz"
        && peer.is_loopback()
        && matches!(*request.method(), Method::GET | Method::HEAD)
    {
        return finish(next.run(request).await, false);
    }
    let ip = match state.client_ip(peer, request.headers()) {
        Ok(ip) => ip,
        Err(status) => return finish(error(status, "Invalid proxy request.\n", None), false),
    };
    let traffic = if decoded.starts_with("/downloads/") || decoded.starts_with("/updates/releases/")
    {
        Traffic::Download
    } else if decoded.starts_with("/updates/") {
        Traffic::Update
    } else {
        Traffic::Page
    };
    let probe = guard::scanner_path(&decoded) || decoded.contains('\\') || decoded.contains('\0');
    let permits = match state.guard.check(ip, traffic, probe, Instant::now()) {
        Ok(permits) => permits,
        Err(Rejection::Limited(seconds)) => {
            return finish(
                error(
                    StatusCode::TOO_MANY_REQUESTS,
                    "Too many requests. Please retry later.\n",
                    Some(seconds),
                ),
                false,
            );
        }
        Err(Rejection::Banned) => {
            return finish(
                error(StatusCode::FORBIDDEN, "Request blocked.\n", Some(900)),
                false,
            );
        }
        Err(Rejection::Capacity) => {
            return finish(
                error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Server busy. Please retry later.\n",
                    Some(60),
                ),
                false,
            );
        }
    };
    if request.uri().to_string().len() > 2048 {
        return finish(
            error(StatusCode::URI_TOO_LONG, "Request URI too long.\n", None),
            false,
        );
    }
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        let mut response = error(
            StatusCode::METHOD_NOT_ALLOWED,
            "Only GET and HEAD are supported.\n",
            None,
        );
        response
            .headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        return finish(response, false);
    }
    if request.headers().contains_key(header::TRANSFER_ENCODING)
        || request
            .headers()
            .get(header::CONTENT_LENGTH)
            .is_some_and(|value| value != "0")
        || request.body().size_hint().lower() > 0
    {
        return finish(
            error(
                StatusCode::BAD_REQUEST,
                "Request body not supported.\n",
                None,
            ),
            false,
        );
    }
    if probe {
        return finish(error(StatusCode::NOT_FOUND, "Not found.\n", None), false);
    }
    let agent = request
        .headers()
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if guard::scanner_agent(agent) {
        return finish(
            error(StatusCode::FORBIDDEN, "Automated scanner blocked.\n", None),
            false,
        );
    }
    let streaming = matches!(traffic, Traffic::Download) && request.method() == Method::GET;
    let held = if streaming {
        let Ok(client) = permits.try_acquire_owned() else {
            return finish(
                error(
                    StatusCode::TOO_MANY_REQUESTS,
                    "Too many active downloads.\n",
                    Some(5),
                ),
                false,
            );
        };
        let Ok(total) = state.guard.downloads.clone().try_acquire_owned() else {
            return finish(
                error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Download capacity reached.\n",
                    Some(5),
                ),
                false,
            );
        };
        Some((client, total))
    } else {
        None
    };
    let no_store = decoded.starts_with("/updates/") && !decoded.starts_with("/updates/releases/");
    let immutable = decoded.starts_with("/updates/releases/");
    let mut response = match tokio::time::timeout(Duration::from_secs(10), next.run(request)).await
    {
        Ok(response) => response,
        Err(_) => {
            return finish(
                error(StatusCode::GATEWAY_TIMEOUT, "Request timed out.\n", None),
                false,
            );
        }
    };
    if immutable && response.status().is_success() {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
    }
    response = finish(response, no_store);
    if let Some((client, total)) = held {
        let (parts, inner) = response.into_parts();
        // Hold both permits until the body finishes or the client disconnects.
        response = Response::from_parts(
            parts,
            Body::new(DownloadBody {
                inner,
                _client: client,
                _total: total,
            }),
        );
    }
    response
}

fn error(status: StatusCode, message: &'static str, retry: Option<u64>) -> Response {
    let mut response = (status, message).into_response();
    if let Some(seconds) = retry {
        response.headers_mut().insert(
            header::RETRY_AFTER,
            HeaderValue::from_str(&seconds.to_string()).expect("numeric retry"),
        );
    }
    response
}

fn finish(mut response: Response, no_store: bool) -> Response {
    let failed = response.status().is_client_error() || response.status().is_server_error();
    let headers = response.headers_mut();
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert(
        "referrer-policy",
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    headers.insert("content-security-policy", HeaderValue::from_static("default-src 'self'; script-src 'none'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'"));
    headers.remove(header::SERVER);
    if no_store || failed {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    } else if !headers.contains_key(header::CACHE_CONTROL) {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    }
    response
}

struct DownloadBody {
    inner: Body,
    _client: OwnedSemaphorePermit,
    _total: OwnedSemaphorePermit,
}

impl http_body::Body for DownloadBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        Pin::new(&mut self.inner).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.size_hint()
    }
}

use http_body::Body as _;
