//! The hub's votes through the HTTP interface.
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use horizon_telemetry_server::store::Store;
use horizon_telemetry_server::{App, Config, router};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const DAY: u32 = 20_741;
const VOTER: &str = "0b7c3c9e-3f0a-4d8e-9a39-4c1f5a6b7d21";

struct Hub {
    app: Arc<App>,
}

fn hub(config: Config) -> Hub {
    let app = App::with_clock(Store::open(":memory:").unwrap(), config, Box::new(|| DAY));
    Hub { app }
}

fn config() -> Config {
    Config::default()
}

impl Hub {
    async fn send(&self, req: Request<Body>) -> (StatusCode, Value) {
        let res = router(self.app.clone()).oneshot(req).await.unwrap();
        let code = res.status();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        (code, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send(
            Request::post(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.send(Request::get(path).body(Body::empty()).unwrap())
            .await
    }
}

#[tokio::test]
async fn votes_count_once_per_browser_and_can_be_taken_back() {
    let hub = hub(config());
    let vote = |up: bool, voter: &str| json!({ "item": "smooth-frames", "voter": voter, "up": up });
    assert_eq!(
        hub.post("/hub/vote", vote(true, VOTER)).await,
        (
            StatusCode::OK,
            json!({ "item": "smooth-frames", "votes": 1 })
        )
    );
    assert_eq!(
        hub.post("/hub/vote", vote(true, VOTER)).await.1["votes"],
        1,
        "a second vote changes nothing"
    );
    let other = "f".repeat(32);
    assert_eq!(
        hub.post("/hub/vote", vote(true, &other)).await.1["votes"],
        2
    );
    assert_eq!(
        hub.get("/hub/votes").await,
        (StatusCode::OK, json!({ "votes": { "smooth-frames": 2 } }))
    );
    assert_eq!(
        hub.post("/hub/vote", vote(false, VOTER)).await.1["votes"],
        1
    );
    assert_eq!(
        hub.get("/hub/votes").await.1["votes"]["smooth-frames"],
        1,
        "a vote drops the cached counts"
    );
    for bad in [
        json!({ "item": "Not An Id", "voter": VOTER }),
        json!({ "item": "a", "voter": "short" }),
        json!([]),
    ] {
        assert_eq!(hub.post("/hub/vote", bad).await.0, StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn the_website_may_post_from_the_browser() {
    let hub = hub(config());
    let res = router(hub.app.clone())
        .oneshot(
            Request::options("/hub/vote")
                .header("origin", "https://playhorizon.dev")
                .header("access-control-request-method", "POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let header = |name: &str| res.headers()[name].to_str().unwrap().to_owned();
    assert_eq!(
        header("access-control-allow-origin"),
        "https://playhorizon.dev"
    );
    assert_eq!(header("access-control-allow-methods"), "POST");
    assert_eq!(header("access-control-allow-headers"), "content-type");
}
