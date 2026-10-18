use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use opensc_web::{
    WebState, app,
    guard::{Guard, Policy, Rejection, Traffic},
};
use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant},
};
use tower::ServiceExt;

fn fixture(policy: Policy, proxies: &[&str], required: bool) -> (tempfile::TempDir, Router) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("downloads")).unwrap();
    std::fs::create_dir_all(root.path().join("updates/releases")).unwrap();
    std::fs::write(root.path().join("index.html"), "OpenSC").unwrap();
    std::fs::write(root.path().join(".env"), "must not be served").unwrap();
    std::fs::write(root.path().join("downloads/game.zip"), b"0123456789").unwrap();
    std::fs::write(root.path().join("updates/releases/engine"), b"engine").unwrap();
    std::fs::write(
        root.path().join("updates/manifest-v1.json"),
        br#"{"schema_version":1,"release":null}"#,
    )
    .unwrap();
    let proxies = proxies
        .iter()
        .map(|s| s.parse().unwrap())
        .collect::<HashSet<IpAddr>>();
    let router = app(
        root.path().to_owned(),
        WebState::new(policy, proxies, required),
    );
    (root, router)
}

fn request(path: &str, peer: &str, headers: &[(&str, &str)]) -> Request<Body> {
    let mut builder = Request::builder().uri(path);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let mut request = builder.body(Body::empty()).unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
    request
}

#[tokio::test]
async fn launcher_can_fetch_manifest_without_a_browser_or_user_agent() {
    let (_root, app) = fixture(Policy::default(), &[], false);
    let response = app
        .oneshot(request("/updates/manifest-v1.json", "127.0.0.1:1000", &[]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert!(
        response.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("script-src 'none'")
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(
        std::str::from_utf8(&body)
            .unwrap()
            .contains("\"release\":null")
    );
}

#[tokio::test]
async fn spoofed_forwarding_headers_do_not_reset_the_client_budget() {
    let policy = Policy {
        page_burst: 2,
        page_per_second: 0.001,
        ..Policy::default()
    };
    let (_root, app) = fixture(policy, &[], false);
    for i in 1..=3 {
        let forged = format!("198.51.100.{i}");
        let response = app
            .clone()
            .oneshot(request(
                "/",
                "192.0.2.10:1000",
                &[("x-real-ip", &forged), ("x-forwarded-for", &forged)],
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if i <= 2 {
                StatusCode::OK
            } else {
                StatusCode::TOO_MANY_REQUESTS
            }
        );
        if i == 3 {
            assert!(response.headers().contains_key("retry-after"));
            assert_eq!(response.headers()["cache-control"], "no-store");
        }
    }
}

#[tokio::test]
async fn only_the_trusted_proxy_can_supply_the_client_identity() {
    let policy = Policy {
        page_burst: 1,
        page_per_second: 0.001,
        ..Policy::default()
    };
    let (_root, app) = fixture(policy, &["10.0.0.2"], true);
    for (peer, headers, expected) in [
        (
            "10.0.0.3:10",
            vec![("x-real-ip", "198.51.100.1")],
            StatusCode::FORBIDDEN,
        ),
        ("10.0.0.2:10", vec![], StatusCode::BAD_REQUEST),
        (
            "10.0.0.2:10",
            vec![("x-real-ip", "198.51.100.1, 198.51.100.2")],
            StatusCode::BAD_REQUEST,
        ),
        (
            "10.0.0.2:10",
            vec![("x-real-ip", "198.51.100.1"), ("x-real-ip", "198.51.100.2")],
            StatusCode::BAD_REQUEST,
        ),
        (
            "10.0.0.2:10",
            vec![("x-real-ip", "198.51.100.1")],
            StatusCode::OK,
        ),
        (
            "10.0.0.2:10",
            vec![
                ("x-real-ip", "198.51.100.1"),
                ("x-forwarded-for", "198.51.100.99"),
            ],
            StatusCode::TOO_MANY_REQUESTS,
        ),
        (
            "10.0.0.2:10",
            vec![("x-real-ip", "198.51.100.2")],
            StatusCode::OK,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(request("/", peer, &headers))
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
}

#[tokio::test]
async fn changing_query_or_encoding_the_path_cannot_bypass_download_limits() {
    let policy = Policy {
        download_burst: 1,
        download_per_second: 0.001,
        ..Policy::default()
    };
    let (_root, app) = fixture(policy, &[], false);
    let first = app
        .clone()
        .oneshot(request("/downloads/game.zip?x=1", "192.0.2.1:10", &[]))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    drop(first);
    let second = app
        .clone()
        .oneshot(request("/%64ownloads/game.zip?x=2", "192.0.2.1:11", &[]))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    for path in [
        "//downloads/game.zip?x=3",
        "/downloads//game.zip?x=4",
        "//updates//releases/engine",
    ] {
        let alias = app
            .clone()
            .oneshot(request(path, "192.0.2.1:12", &[]))
            .await
            .unwrap();
        assert_eq!(
            alias.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "alias {path}"
        );
    }
    let manifest = app
        .oneshot(request("/updates/manifest-v1.json", "192.0.2.1:10", &[]))
        .await
        .unwrap();
    assert_eq!(manifest.status(), StatusCode::OK);
}

#[tokio::test]
async fn range_and_head_requests_keep_correct_download_semantics() {
    let (_root, app) = fixture(Policy::default(), &[], false);
    let response = app
        .clone()
        .oneshot(request(
            "/downloads/game.zip",
            "127.0.0.1:10",
            &[("range", "bytes=2-5")],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.headers()["content-range"], "bytes 2-5/10");
    assert_eq!(
        &response.into_body().collect().await.unwrap().to_bytes()[..],
        b"2345"
    );
    let mut head = request("/downloads/game.zip", "127.0.0.1:10", &[]);
    *head.method_mut() = axum::http::Method::HEAD;
    let response = app.oneshot(head).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-length"], "10");
    assert!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty()
    );
}

#[tokio::test]
async fn download_slots_are_held_until_body_drop_including_client_disconnect() {
    let policy = Policy {
        downloads_per_ip: 1,
        downloads_total: 1,
        ..Policy::default()
    };
    let (_root, app) = fixture(policy, &[], false);
    let first = app
        .clone()
        .oneshot(request("/downloads/game.zip", "192.0.2.1:10", &[]))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let same_ip = app
        .clone()
        .oneshot(request("/downloads/game.zip", "192.0.2.1:11", &[]))
        .await
        .unwrap();
    assert_eq!(same_ip.status(), StatusCode::TOO_MANY_REQUESTS);
    let another_ip = app
        .clone()
        .oneshot(request("/downloads/game.zip", "192.0.2.2:10", &[]))
        .await
        .unwrap();
    assert_eq!(another_ip.status(), StatusCode::SERVICE_UNAVAILABLE);
    drop(first);
    let available = app
        .oneshot(request("/downloads/game.zip", "192.0.2.2:11", &[]))
        .await
        .unwrap();
    assert_eq!(available.status(), StatusCode::OK);
}

#[tokio::test]
async fn scanner_paths_cannot_read_files_even_when_present_on_disk() {
    let (_root, app) = fixture(Policy::default(), &[], false);
    for path in [
        "/.env",
        "/%2eenv",
        "/downloads/%2e%2e/.env",
        "/wp-login.php",
    ] {
        let response = app
            .clone()
            .oneshot(request(path, "192.0.2.1:10", &[]))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(
            !String::from_utf8(
                response
                    .into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .to_vec()
            )
            .unwrap()
            .contains("must not be served")
        );
    }
}

#[tokio::test]
async fn scanner_user_agents_are_denied_but_command_line_clients_are_allowed() {
    let (_root, app) = fixture(Policy::default(), &[], false);
    for (agent, expected) in [
        ("sqlmap/1.9", StatusCode::FORBIDDEN),
        ("Bytespider", StatusCode::FORBIDDEN),
        ("curl/8.10", StatusCode::OK),
        ("OpenSC Launcher/0.2.0", StatusCode::OK),
    ] {
        let response = app
            .clone()
            .oneshot(request("/", "192.0.2.1:10", &[("user-agent", agent)]))
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
}

#[tokio::test]
async fn unsupported_methods_and_request_bodies_are_rejected() {
    let (_root, app) = fixture(Policy::default(), &[], false);
    let mut post = request("/", "127.0.0.1:10", &[]);
    *post.method_mut() = axum::http::Method::POST;
    let response = app.clone().oneshot(post).await.unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers()["allow"], "GET, HEAD");
    let mut body = request("/", "127.0.0.1:10", &[("content-length", "4")]);
    *body.body_mut() = Body::from("data");
    assert_eq!(
        app.oneshot(body).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn healthchecks_survive_proxy_restrictions_and_releases_are_immutable() {
    let (_root, restricted) = fixture(Policy::default(), &[], true);
    assert_eq!(
        restricted
            .oneshot(request("/healthz", "127.0.0.1:10", &[]))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let (_root, app) = fixture(Policy::default(), &[], false);
    let response = app
        .oneshot(request("/updates/releases/engine", "127.0.0.1:10", &[]))
        .await
        .unwrap();
    assert_eq!(
        response.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
}

#[test]
fn repeated_scans_produce_a_temporary_ban_that_expires() {
    let gate = Guard::new(Policy::default());
    let ip = "192.0.2.1".parse().unwrap();
    let now = Instant::now();
    for _ in 0..4 {
        assert!(gate.check(ip, Traffic::Page, true, now).is_ok());
    }
    assert!(matches!(
        gate.check(ip, Traffic::Page, true, now),
        Err(Rejection::Banned)
    ));
    assert!(matches!(
        gate.check(ip, Traffic::Page, false, now + Duration::from_secs(899)),
        Err(Rejection::Banned)
    ));
    assert!(
        gate.check(ip, Traffic::Page, false, now + Duration::from_secs(901))
            .is_ok()
    );
}

#[test]
fn ipv6_interface_rotation_and_mapped_ipv4_do_not_create_new_budgets() {
    let gate = Guard::new(Policy {
        page_burst: 1,
        page_per_second: 0.001,
        ..Policy::default()
    });
    let now = Instant::now();
    assert!(
        gate.check(
            "2001:db8:1:2::1".parse().unwrap(),
            Traffic::Page,
            false,
            now
        )
        .is_ok()
    );
    assert!(matches!(
        gate.check(
            "2001:db8:1:2::99".parse().unwrap(),
            Traffic::Page,
            false,
            now
        ),
        Err(Rejection::Limited(_))
    ));
    assert!(
        gate.check("192.0.2.1".parse().unwrap(), Traffic::Page, false, now)
            .is_ok()
    );
    assert!(matches!(
        gate.check(
            "::ffff:192.0.2.1".parse().unwrap(),
            Traffic::Page,
            false,
            now
        ),
        Err(Rejection::Limited(_))
    ));
}

#[test]
fn bounded_client_storage_does_not_evict_an_active_download() {
    let policy = Policy {
        max_clients: 1,
        downloads_per_ip: 1,
        ..Policy::default()
    };
    let gate = Guard::new(policy);
    let now = Instant::now();
    let active = gate
        .check("192.0.2.1".parse().unwrap(), Traffic::Download, false, now)
        .unwrap()
        .try_acquire_owned()
        .unwrap();
    let later = now + Duration::from_secs(1300);
    assert!(matches!(
        gate.check("192.0.2.2".parse().unwrap(), Traffic::Page, false, later),
        Err(Rejection::Capacity)
    ));
    drop(active);
    assert!(
        gate.check(
            "192.0.2.2".parse().unwrap(),
            Traffic::Page,
            false,
            later + Duration::from_secs(61)
        )
        .is_ok()
    );
}
