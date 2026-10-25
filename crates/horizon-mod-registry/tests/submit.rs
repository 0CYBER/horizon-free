//! The whole submission against a mock GitHub: sign in, submit the sample
//! signed mod (`crates/horizon-mods/tests/index`), and what lands on GitHub;
//! then what is refused.
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{Path as UrlPath, Query, State},
    http::{Request, StatusCode, header},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use horizon_mod_registry::{App, Config, github, router};
use horizon_mods::index::registry::Entry;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tower::ServiceExt;

const PUBLIC: &str = "https://playhorizon.test";
const HMOD_SHA: &str = "e1135345692f6dd8fd4c51920151944b82333e17049c9bb7d756f27b8deefc53";

#[derive(Default)]
struct Mock {
    /// Path -> text on main.
    main: HashMap<String, String>,
    assets: Vec<(String, usize)>,
    branches: Vec<String>,
    /// (branch, path) -> (text, message, replaced blob).
    commits: HashMap<(String, String), (String, String, Option<String>)>,
    pulls: Vec<Value>,
    /// The account the next sign-in gets.
    login: String,
}

type Shared = Arc<Mutex<Mock>>;

async fn mock_github() -> (String, Shared) {
    let state: Shared = Arc::default();
    state.lock().unwrap().login = "alice".into();
    let app = Router::new()
        .route(
            "/login/oauth/access_token",
            post(|Json(body): Json<Value>| async move {
                Json(if body["code"] == "good" {
                    json!({"access_token": "gho_user"})
                } else {
                    json!({"error": "bad_verification_code"})
                })
            }),
        )
        .route(
            "/user",
            get(|State(s): State<Shared>| async move {
                Json(json!({"login": s.lock().unwrap().login, "id": 7}))
            }),
        )
        .route(
            "/repos/o/r/contents/mods/{file}",
            get(
                |State(s): State<Shared>, UrlPath(file): UrlPath<String>| async move {
                    let s = s.lock().unwrap();
                    match s.main.get(&format!("mods/{file}")) {
                        Some(text) => Ok(Json(json!({"content": STANDARD.encode(text), "sha": "blob1"}))),
                        None => Err(StatusCode::NOT_FOUND),
                    }
                },
            )
            .put(
                |State(s): State<Shared>, UrlPath(file): UrlPath<String>, Json(b): Json<Value>| async move {
                    let text = String::from_utf8(STANDARD.decode(b["content"].as_str().unwrap()).unwrap()).unwrap();
                    let key = (b["branch"].as_str().unwrap().to_owned(), format!("mods/{file}"));
                    let value = (text, b["message"].as_str().unwrap().to_owned(), b["sha"].as_str().map(str::to_owned));
                    s.lock().unwrap().commits.insert(key, value);
                    Json(json!({}))
                },
            ),
        )
        .route(
            "/repos/o/r/releases/tags/staging",
            get(|State(s): State<Shared>| async move {
                let names: Vec<Value> = s.lock().unwrap().assets.iter().map(|(n, _)| json!({"name": n})).collect();
                Json(json!({"id": 42, "assets": names}))
            }),
        )
        .route(
            "/repos/o/r/releases/42/assets",
            post(
                |State(s): State<Shared>, Query(q): Query<HashMap<String, String>>, body: Bytes| async move {
                    s.lock().unwrap().assets.push((q["name"].clone(), body.len()));
                    Json(json!({}))
                },
            ),
        )
        .route(
            "/repos/o/r/git/ref/heads/main",
            get(|| async { Json(json!({"object": {"sha": "abc123"}})) }),
        )
        .route(
            "/repos/o/r/git/refs",
            post(|State(s): State<Shared>, Json(b): Json<Value>| async move {
                assert_eq!(b["sha"], "abc123");
                let name = b["ref"].as_str().unwrap().trim_start_matches("refs/heads/").to_owned();
                let mut s = s.lock().unwrap();
                if s.branches.contains(&name) {
                    return Err(StatusCode::UNPROCESSABLE_ENTITY);
                }
                s.branches.push(name);
                Ok(Json(json!({})))
            }),
        )
        .route(
            "/repos/o/r/pulls",
            post(|State(s): State<Shared>, Json(b): Json<Value>| async move {
                let mut s = s.lock().unwrap();
                s.pulls.push(b);
                Json(json!({"html_url": format!("https://github.com/o/r/pull/{}", s.pulls.len())}))
            }),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, state)
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../horizon-mods/tests/index")
}

struct Client {
    app: Router,
    cookies: HashMap<String, String>,
}

impl Client {
    async fn send(&mut self, request: Request<Body>) -> (StatusCode, Value, Option<String>) {
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        for c in response.headers().get_all(header::SET_COOKIE) {
            let (pair, _) = c.to_str().unwrap().split_once(';').unwrap();
            let (k, v) = pair.split_once('=').unwrap();
            self.cookies.insert(k.into(), v.into());
        }
        let location = response
            .headers()
            .get(header::LOCATION)
            .map(|l| l.to_str().unwrap().to_owned());
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            location,
        )
    }

    fn cookie_header(&self) -> String {
        self.cookies
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn get(&self, uri: &str) -> Request<Body> {
        Request::get(uri)
            .header(header::COOKIE, self.cookie_header())
            .body(Body::empty())
            .unwrap()
    }

    async fn sign_in(&mut self) -> String {
        let (status, _, location) = self.send(self.get("/api/mods/auth/login")).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let location = location.unwrap();
        assert!(location.contains(
            "redirect_uri=https%3A%2F%2Fplayhorizon.test%2Fapi%2Fmods%2Fauth%2Fcallback"
        ));
        let state = location
            .split("state=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap()
            .to_owned();
        // Another state than the one this browser started with is refused.
        let (status, ..) = self
            .send(self.get("/api/mods/auth/callback?code=good&state=other"))
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _, location) = self
            .send(self.get(&format!("/api/mods/auth/callback?code=good&state={state}")))
            .await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert_eq!(
            location.as_deref(),
            Some("https://playhorizon.test/mods/submit")
        );
        let (status, me, _) = self.send(self.get("/api/mods/me")).await;
        assert_eq!(status, StatusCode::OK);
        me["csrf"].as_str().unwrap().to_owned()
    }

    async fn submit(&mut self, csrf: &str, form: &Value, file: &[u8]) -> (StatusCode, Value) {
        let boundary = "XhorizonX";
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"entry\"\r\n\r\n{form}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"f\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(file);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let request = Request::post("/api/mods/submit")
            .header(header::COOKIE, self.cookie_header())
            .header(header::ORIGIN, PUBLIC)
            .header("x-csrf-token", csrf)
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();
        let (status, json, _) = self.send(request).await;
        (status, json)
    }
}

fn form() -> Value {
    let key = std::fs::read_to_string(fixtures().join("test.pub")).unwrap();
    json!({
        "id": "coords_hud",
        "kind": "mod",
        "name": "Coordinates HUD",
        "summary": "Your coordinates in a corner.",
        "authors": ["Alice"],
        "license": "MIT",
        "source": "https://github.com/alice/coords",
        "key": key.lines().last().unwrap(),
        "signature": std::fs::read_to_string(fixtures().join("coords_hud-0.1.0.hmod.minisig")).unwrap(),
        "version": "0.1.0",
        "game": "26.3",
    })
}

async fn setup() -> (Client, Shared) {
    let (base, mock) = mock_github().await;
    let scratch =
        std::env::temp_dir().join(format!("horizon-mod-registry-test-{}", std::process::id()));
    let config = Config {
        public_url: PUBLIC.into(),
        session_key: b"a test key of at least thirty-two bytes".to_vec(),
        github: github::GitHub::new(
            github::Endpoints {
                web: base.clone(),
                api: base.clone(),
                uploads: base,
            },
            "o/r".into(),
            "client".into(),
            "secret".into(),
            "bot".into(),
        ),
        scratch,
    };
    let client = Client {
        app: router(App::new(config)),
        cookies: HashMap::new(),
    };
    (client, mock)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_submission_becomes_a_pull_request() {
    let (mut client, mock) = setup().await;
    let file = std::fs::read(fixtures().join("coords_hud-0.1.0.hmod")).unwrap();
    // Not signed in.
    let (status, _) = client.submit("x", &form(), &file).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let csrf = client.sign_in().await;
    let (status, _) = client.submit("wrong", &form(), &file).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, json) = client.submit(&csrf, &form(), &file).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["pull_request"], "https://github.com/o/r/pull/1");
    {
        let m = mock.lock().unwrap();
        assert_eq!(m.assets, [(format!("{HMOD_SHA}.hmod"), file.len())]);
        assert_eq!(m.branches, ["submit/coords_hud-0.1.0"]);
        let (text, message, replaced) = &m.commits[&(
            "submit/coords_hud-0.1.0".into(),
            "mods/coords_hud.toml".into(),
        )];
        let entry: Entry = toml::from_str(text).unwrap();
        assert_eq!(entry.owners, ["alice"]);
        assert_eq!(entry.versions[0].sha256, HMOD_SHA);
        assert_eq!(entry.versions[0].file, "coords_hud-0.1.0.hmod");
        assert!(entry.versions[0].signature.is_some() && entry.key.is_some());
        assert!(message.contains("Co-authored-by: alice <7+alice@users.noreply.github.com>"));
        assert_eq!(replaced, &None);
        let body = m.pulls[0]["body"].as_str().unwrap();
        assert!(body.starts_with("<!-- submitted-by: alice -->"), "{body}");
        assert!(body.contains("may draw text on your screen"), "{body}");
    }
    // The same version again: its branch is there.
    let (status, json) = client.submit(&csrf, &form(), &file).await;
    assert_eq!(status, StatusCode::CONFLICT, "{json}");
    assert_eq!(mock.lock().unwrap().assets.len(), 1, "uploaded once");
}

#[tokio::test(flavor = "multi_thread")]
async fn what_the_form_refuses() {
    let (mut client, mock) = setup().await;
    let file = std::fs::read(fixtures().join("coords_hud-0.1.0.hmod")).unwrap();
    let csrf = client.sign_in().await;
    let refused = |status: StatusCode, json: &Value, part: &str| {
        assert!(
            json["error"].as_str().unwrap_or("").contains(part),
            "{status} {json}"
        );
    };
    // A file that isn't what the page says.
    let mut f = form();
    f["id"] = json!("other_mod");
    let (status, json) = client.submit(&csrf, &f, &file).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    refused(status, &json, "the file's id is \"coords_hud\"");
    // A broken signature.
    let mut f = form();
    f["signature"] = json!(
        std::fs::read_to_string(fixtures().join("coords_hud-0.1.0.hmod.minisig"))
            .unwrap()
            .replace("RUTV", "RUTW")
    );
    let (status, json) = client.submit(&csrf, &f, &file).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{json}");
    // Shader packs aren't open.
    let mut f = form();
    f["kind"] = json!("shader");
    let (status, json) = client.submit(&csrf, &f, &file).await;
    refused(status, &json, "shader packs open");
    // An update by someone who isn't an owner.
    let owned = "id = \"coords_hud\"\nkind = \"mod\"\nname = \"C\"\nsummary = \"s\"\nowners = [\"bob\"]\nlicense = \"MIT\"\nsource = \"https://x.test\"\n\n[[versions]]\nversion = \"0.0.1\"\npublished = \"2026-01-01\"\ngame = \"26.3\"\nfile = \"coords_hud-0.0.1.hmod\"\nsha256 = \"0000000000000000000000000000000000000000000000000000000000000000\"\n";
    mock.lock()
        .unwrap()
        .main
        .insert("mods/coords_hud.toml".into(), owned.into());
    let (status, json) = client.submit(&csrf, &form(), &file).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    refused(status, &json, "bob's");
    // From another site.
    let mut request = Request::post("/api/mods/submit")
        .header(header::COOKIE, client.cookie_header())
        .header(header::ORIGIN, "https://evil.test")
        .header("x-csrf-token", &csrf)
        .body(Body::empty())
        .unwrap();
    request.headers_mut().insert(
        header::CONTENT_TYPE,
        "multipart/form-data; boundary=x".parse().unwrap(),
    );
    let (status, ..) = client.send(request).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let m = mock.lock().unwrap();
    assert!(
        m.assets.is_empty() && m.pulls.is_empty(),
        "nothing reached GitHub"
    );
}
