use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use opensc_report_protocol::{API_PATH, Diagnostics, Receipt, Report};
use opensc_web::{WebState, app, guard::Policy, reports::Store};
use std::{collections::HashSet, sync::Arc};
use tower::ServiceExt;

fn report(id: &str) -> Report {
    Report {
        schema_version: 1,
        request_id: id.repeat(32),
        category: "interface".into(),
        title: "Inventory freezes input".into(),
        steps: "Open training, open and close inventory, click another button".into(),
        expected: "Other buttons work".into(),
        actual: "Inventory opens again".into(),
        diagnostics: Diagnostics {
            launcher_version: "0.4.0".into(),
            engine_version: "test".into(),
            platform: "linux-x86_64".into(),
            ..Diagnostics::default()
        },
    }
}

fn fixture() -> (tempfile::TempDir, Router, Arc<Store>) {
    let root = tempfile::tempdir().unwrap();
    let public = root.path().join("public");
    std::fs::create_dir(&public).unwrap();
    std::fs::write(public.join("index.html"), "OpenSC").unwrap();
    let store = Arc::new(Store::open(&root.path().join("private.sqlite3")).unwrap());
    let mut state = WebState::new(Policy::default(), HashSet::new(), false);
    state.reports = Some(store.clone());
    let router = app(public, state);
    (root, router, store)
}

fn request(body: Vec<u8>, ip: u8) -> Request<Body> {
    let mut request = Request::post(API_PATH)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        format!("192.0.2.{ip}:1000")
            .parse::<std::net::SocketAddr>()
            .unwrap(),
    ));
    request
}

async fn post(router: &Router, report: &Report, ip: u8) -> axum::response::Response {
    router
        .clone()
        .oneshot(request(serde_json::to_vec(report).unwrap(), ip))
        .await
        .unwrap()
}

async fn receipt(response: axum::response::Response) -> Receipt {
    assert_eq!(response.status(), StatusCode::CREATED);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[tokio::test]
async fn repeated_delivery_is_idempotent_and_similar_reports_share_a_ticket() {
    let (_root, router, store) = fixture();
    let first = receipt(post(&router, &report("a"), 1).await).await;
    let retry = receipt(post(&router, &report("a"), 1).await).await;
    let repeated = receipt(post(&router, &report("b"), 1).await).await;
    assert_eq!(first.ticket_id, retry.ticket_id);
    assert_eq!(first.ticket_id, repeated.ticket_id);
    assert!(retry.duplicate && repeated.duplicate && !first.duplicate);
    assert_eq!(store.list().unwrap()[0]["occurrences"], 2);
    assert_eq!(
        store
            .show(&first.ticket_id)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let mut changed = report("a");
    changed.title = "Different report content".into();
    assert_eq!(
        post(&router, &changed, 2).await.status(),
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn report_size_schema_and_rate_are_bounded_and_storage_is_private() {
    let (_root, router, _store) = fixture();
    let mut invalid = report("a");
    invalid.schema_version = 2;
    assert_eq!(
        post(&router, &invalid, 1).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let oversized = vec![b' '; opensc_report_protocol::MAX_BODY + 1];
    assert_eq!(
        router
            .clone()
            .oneshot(request(oversized, 2))
            .await
            .unwrap()
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    for id in ["b", "c", "d"] {
        assert_eq!(
            post(&router, &report(id), 3).await.status(),
            StatusCode::CREATED
        );
    }
    let limited = post(&router, &report("e"), 3).await;
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(limited.headers().contains_key("retry-after"));
    let mut get = Request::get("/private.sqlite3")
        .body(Body::empty())
        .unwrap();
    get.extensions_mut().insert(ConnectInfo(
        "192.0.2.4:1000".parse::<std::net::SocketAddr>().unwrap(),
    ));
    assert_eq!(
        router.clone().oneshot(get).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    let mut get = Request::get(API_PATH).body(Body::empty()).unwrap();
    get.extensions_mut().insert(ConnectInfo(
        "192.0.2.4:1000".parse::<std::net::SocketAddr>().unwrap(),
    ));
    assert_eq!(
        router.oneshot(get).await.unwrap().status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
}

#[tokio::test]
async fn status_changes_and_online_backup_survive_reopening_database() {
    let (root, router, store) = fixture();
    let ticket = receipt(post(&router, &report("a"), 1).await)
        .await
        .ticket_id;
    store
        .set_status(
            &ticket,
            "triaged",
            "Reproduced with modal hiding pressed HUD",
        )
        .unwrap();
    store
        .set_status(&ticket, "fixed", "Clear stale press markers")
        .unwrap();
    assert!(store.set_status(&ticket, "invalid", "bad state").is_err());
    let backup = root.path().join("backup.sqlite3");
    store.backup(&backup).unwrap();
    let restored = Store::open(&backup).unwrap();
    assert_eq!(restored.list().unwrap()[0]["status"], "fixed");
    assert_eq!(restored.show(&ticket).unwrap().as_array().unwrap().len(), 1);
}
