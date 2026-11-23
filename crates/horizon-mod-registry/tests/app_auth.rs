//! The bot as a GitHub App, against a mock GitHub: the JWT it signs, the installation token it gets for it,
//! kept, replaced before it ends and once after a refusal; a configuration with only part of the App, refused;
//! and no key or token in any message. `tests/app-key.pem` is a throwaway key made for these tests.
use axum::{
    Json, Router,
    extract::{Path as UrlPath, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use horizon_mod_registry::github::{self, AppCredentials, GitHub};
use ring::signature::{RSA_PKCS1_2048_8192_SHA256, RsaKeyPair, UnparsedPublicKey};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

const KEY: &str = include_str!("app-key.pem");
const INSTALLATION: &str = "99";

struct Mock {
    /// The JWT of every exchange, in order.
    jwts: Vec<String>,
    /// The token every API call carried, in order.
    used: Vec<String>,
    /// When the tokens it hands out end.
    expires_at: String,
    /// How many API calls to refuse with a 401 before answering.
    refuse: usize,
    /// Exchanges asked for, failed ones too.
    attempts: usize,
    /// Every exchange fails (a 500).
    fail: bool,
    /// How long an exchange takes.
    delay: std::time::Duration,
    /// The token handed out instead of `ghs_test<n>`.
    token: Option<String>,
}

type Shared = Arc<Mutex<Mock>>;

fn bearer(headers: &HeaderMap) -> String {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("")
        .to_owned()
}

async fn mock() -> (String, Shared) {
    let state: Shared = Arc::new(Mutex::new(Mock {
        jwts: Vec::new(),
        used: Vec::new(),
        expires_at: "2099-01-01T00:00:00Z".into(),
        refuse: 0,
        attempts: 0,
        fail: false,
        delay: std::time::Duration::ZERO,
        token: None,
    }));
    let app = Router::new()
        .route(
            "/app/installations/{id}/access_tokens",
            post(
                |State(s): State<Shared>, UrlPath(id): UrlPath<String>, headers: HeaderMap| async move {
                    if id != INSTALLATION {
                        return Err((StatusCode::NOT_FOUND, Json(json!({"message": "Not Found"}))));
                    }
                    let (fail, delay) = {
                        let mut s = s.lock().unwrap();
                        s.attempts += 1;
                        (s.fail, s.delay)
                    };
                    tokio::time::sleep(delay).await;
                    if fail {
                        return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"message": "ghs_test_in_error"}))));
                    }
                    let mut s = s.lock().unwrap();
                    s.jwts.push(bearer(&headers));
                    let token = s.token.clone().unwrap_or_else(|| format!("ghs_test{}", s.jwts.len()));
                    Ok((
                        StatusCode::CREATED,
                        Json(json!({"token": token, "expires_at": s.expires_at})),
                    ))
                },
            ),
        )
        .route(
            "/repos/o/r/contents/{*path}",
            get(|State(s): State<Shared>, headers: HeaderMap| async move {
                let mut s = s.lock().unwrap();
                s.used.push(bearer(&headers));
                if s.refuse > 0 {
                    s.refuse -= 1;
                    return Err((StatusCode::UNAUTHORIZED, Json(json!({"message": "Bad credentials"}))));
                }
                Ok(Json(json!({"content": STANDARD.encode("x"), "sha": "blob1"})))
            }),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, state)
}

fn client(base: &str, installation: &str) -> GitHub {
    GitHub::new(
        github::Endpoints {
            web: base.into(),
            api: base.into(),
            uploads: base.into(),
        },
        "o/r".into(),
        "client".into(),
        "secret".into(),
        String::new(),
    )
    .as_app(AppCredentials::new("12345", installation, KEY).unwrap())
}

/// `blob` of a file the mock has: one API call as the bot. Blocking, so off the mock's runtime.
async fn blob(gh: &GitHub) -> Result<Option<String>, github::Failure> {
    let gh = gh.clone();
    tokio::task::spawn_blocking(move || gh.blob("mods/x.toml"))
        .await
        .unwrap()
}

/// Every line of the key's body: none may show up in a message.
fn assert_no_secret(text: &str) {
    assert!(!text.contains("ghs_test"), "{text}");
    assert!(!text.contains("eyJ"), "a JWT in {text}");
    for line in KEY.lines().filter(|l| !l.starts_with("-----")) {
        assert!(!text.contains(line), "the key in {text}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_jwt_is_rs256_signed_by_the_apps_key() {
    let (base, mock) = mock().await;
    let gh = client(&base, INSTALLATION);
    assert_eq!(blob(&gh).await.unwrap().as_deref(), Some("blob1"));
    let jwt = mock.lock().unwrap().jwts[0].clone();
    let parts: Vec<&str> = jwt.split('.').collect();
    assert_eq!(parts.len(), 3, "{jwt}");
    let decode = |part: &str| -> Value {
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).unwrap()).unwrap()
    };
    assert_eq!(decode(parts[0]), json!({"alg": "RS256", "typ": "JWT"}));
    let claims = decode(parts[1]);
    assert_eq!(claims["iss"], json!(12345));
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let (iat, exp) = (
        claims["iat"].as_u64().unwrap(),
        claims["exp"].as_u64().unwrap(),
    );
    assert!(iat <= now && iat + 120 >= now, "iat {iat}, now {now}");
    assert_eq!(exp - iat, 600, "ten minutes in all, GitHub's longest");
    // The public half of the same key verifies it.
    let body: String = KEY.lines().filter(|l| !l.starts_with("-----")).collect();
    let key = RsaKeyPair::from_der(&STANDARD.decode(body).unwrap()).unwrap();
    let signature = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
    UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, key.public().as_ref())
        .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
        .expect("signed by the App's key");
    // A client ID is sent as it is.
    let by_client_id = GitHub::new(
        github::Endpoints {
            web: base.clone(),
            api: base.clone(),
            uploads: base,
        },
        "o/r".into(),
        "client".into(),
        "secret".into(),
        String::new(),
    )
    .as_app(AppCredentials::new("Iv23liTest", INSTALLATION, KEY).unwrap());
    blob(&by_client_id).await.unwrap();
    let jwt = mock.lock().unwrap().jwts[1].clone();
    assert_eq!(
        decode(jwt.split('.').nth(1).unwrap())["iss"],
        json!("Iv23liTest")
    );
    assert_eq!(mock.lock().unwrap().used, ["ghs_test1", "ghs_test2"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn one_token_serves_every_call_and_every_clone() {
    let (base, mock) = mock().await;
    let gh = client(&base, INSTALLATION);
    for _ in 0..3 {
        blob(&gh).await.unwrap();
    }
    blob(&gh.clone()).await.unwrap();
    let mock = mock.lock().unwrap();
    assert_eq!(mock.jwts.len(), 1);
    assert_eq!(mock.used, ["ghs_test1"; 4]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_about_to_end_is_replaced() {
    let (base, mock) = mock().await;
    mock.lock().unwrap().expires_at = "2000-01-01T00:00:00Z".into();
    let gh = client(&base, INSTALLATION);
    blob(&gh).await.unwrap();
    blob(&gh).await.unwrap();
    {
        let mock = mock.lock().unwrap();
        assert_eq!(mock.jwts.len(), 2);
        assert_eq!(mock.used, ["ghs_test1", "ghs_test2"]);
    }
    // Five minutes left is too little: a file's upload may take that long.
    let soon = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;
    mock.lock().unwrap().expires_at = iso(soon);
    blob(&gh).await.unwrap();
    blob(&gh).await.unwrap();
    assert_eq!(mock.lock().unwrap().jwts.len(), 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_token_is_replaced_once() {
    let (base, mock) = mock().await;
    let gh = client(&base, INSTALLATION);
    mock.lock().unwrap().refuse = 1;
    assert_eq!(blob(&gh).await.unwrap().as_deref(), Some("blob1"));
    {
        let mock = mock.lock().unwrap();
        assert_eq!(mock.used, ["ghs_test1", "ghs_test2"]);
    }
    // Refused again: one retry, then GitHub's answer.
    mock.lock().unwrap().refuse = 2;
    let failure = blob(&gh).await.unwrap_err();
    assert_eq!(failure.status, 401);
    assert_no_secret(&failure.message);
    {
        let mock = mock.lock().unwrap();
        assert_eq!(
            mock.used,
            ["ghs_test1", "ghs_test2", "ghs_test2", "ghs_test3"]
        );
        assert_eq!(mock.jwts.len(), 3);
    }
    // The token refused the second time is dropped too: the next call starts with a new one.
    blob(&gh).await.unwrap();
    assert_eq!(mock.lock().unwrap().used.last().unwrap(), "ghs_test4");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_refresh_keeps_the_old_token_while_it_lasts() {
    let (base, mock) = mock().await;
    // Eight minutes left: due for a refresh, still good for any call.
    mock.lock().unwrap().expires_at = iso(now() + 8 * 60);
    let gh = client(&base, INSTALLATION);
    blob(&gh).await.unwrap();
    mock.lock().unwrap().fail = true;
    assert_eq!(blob(&gh).await.unwrap().as_deref(), Some("blob1"));
    // Within the cooldown the old token serves without another exchange.
    blob(&gh).await.unwrap();
    let mock = mock.lock().unwrap();
    assert_eq!(mock.used, ["ghs_test1"; 3]);
    assert_eq!(mock.attempts, 2);
}

/// A failed refresh leaves the old token and a cooldown; when GitHub then refuses that token,
/// it and the cooldown go, so the call's one retry exchanges for a new token.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_old_token_ends_the_cooldown() {
    let (base, mock) = mock().await;
    mock.lock().unwrap().expires_at = iso(now() + 8 * 60);
    let gh = client(&base, INSTALLATION);
    blob(&gh).await.unwrap();
    mock.lock().unwrap().fail = true;
    blob(&gh).await.unwrap();
    {
        let mut mock = mock.lock().unwrap();
        mock.fail = false;
        mock.refuse = 1;
    }
    assert_eq!(blob(&gh).await.unwrap().as_deref(), Some("blob1"));
    let mock = mock.lock().unwrap();
    assert_eq!(
        mock.used,
        ["ghs_test1", "ghs_test1", "ghs_test1", "ghs_test2"]
    );
    assert_eq!(mock.attempts, 3);
}

/// The same while GitHub is still down: the retry makes one exchange, and the calls after it
/// get its failure for the cooldown instead of each trying again.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_old_token_during_an_outage_tries_once() {
    let (base, mock) = mock().await;
    mock.lock().unwrap().expires_at = iso(now() + 8 * 60);
    let gh = client(&base, INSTALLATION);
    blob(&gh).await.unwrap();
    mock.lock().unwrap().fail = true;
    blob(&gh).await.unwrap();
    mock.lock().unwrap().refuse = 1;
    let failure = blob(&gh).await.unwrap_err();
    assert!(failure.message.contains("500"), "{}", failure.message);
    assert!(blob(&gh).await.is_err());
    let mock = mock.lock().unwrap();
    assert_eq!(mock.used, ["ghs_test1"; 3]);
    assert_eq!(mock.attempts, 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_exchange_answers_every_waiting_call() {
    let (base, mock) = mock().await;
    {
        let mut mock = mock.lock().unwrap();
        mock.fail = true;
        mock.delay = std::time::Duration::from_millis(300);
    }
    let gh = client(&base, INSTALLATION);
    let calls: Vec<_> = (0..4)
        .map(|_| tokio::spawn(blob_owned(gh.clone())))
        .collect();
    for call in calls {
        let failure = call.await.unwrap().unwrap_err();
        assert_eq!(failure.status, 0);
        assert!(failure.message.contains("500"), "{}", failure.message);
        assert_no_secret(&failure.message);
    }
    // And a call right after it, without trying again.
    assert!(blob(&gh).await.is_err());
    let mock = mock.lock().unwrap();
    assert_eq!(mock.attempts, 1);
    assert!(mock.used.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_that_isnt_one_is_refused_unseen() {
    let (base, mock) = mock().await;
    mock.lock().unwrap().token = Some("ghs_test_bad\u{7}\r\nX-Other: 1".into());
    let gh = client(&base, INSTALLATION);
    let failure = blob(&gh).await.unwrap_err();
    assert_eq!(failure.status, 0);
    assert_no_secret(&failure.message);
    assert!(mock.lock().unwrap().used.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn what_an_old_token_has_left_counts_from_the_failed_exchanges_end() {
    let (base, mock) = mock().await;
    // Due for a refresh, and two seconds past `USABLE_FOR` (six minutes) when the refresh starts.
    mock.lock().unwrap().expires_at = iso(now() + 6 * 60 + 2);
    let gh = client(&base, INSTALLATION);
    blob(&gh).await.unwrap();
    {
        let mut mock = mock.lock().unwrap();
        mock.fail = true;
        mock.delay = std::time::Duration::from_millis(2500);
    }
    // Past the exchange less than six minutes are left: the old token no longer serves.
    let failure = blob(&gh).await.unwrap_err();
    assert!(failure.message.contains("500"), "{}", failure.message);
    let mock = mock.lock().unwrap();
    assert_eq!(mock.used, ["ghs_test1"]);
    assert_eq!(mock.attempts, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_refused_during_a_refresh_is_not_handed_out_again() {
    let (base, mock) = mock().await;
    mock.lock().unwrap().expires_at = iso(now() + 8 * 60);
    let gh = client(&base, INSTALLATION);
    blob(&gh).await.unwrap();
    {
        let mut mock = mock.lock().unwrap();
        mock.delay = std::time::Duration::from_millis(1500);
        mock.expires_at = "2099-01-01T00:00:00Z".into();
    }
    // One call refreshes, slowly; meanwhile the old token is refused for another, which drops it.
    let refreshing = tokio::spawn(blob_owned(gh.clone()));
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    mock.lock().unwrap().refuse = 1;
    let refused = tokio::spawn(blob_owned(gh.clone()));
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    // Every call from here waits for the new token.
    let later = tokio::spawn(blob_owned(gh.clone()));
    for call in [refreshing, refused, later] {
        call.await.unwrap().unwrap();
    }
    let mock = mock.lock().unwrap();
    assert_eq!(
        mock.used,
        [
            "ghs_test1",
            "ghs_test1",
            "ghs_test2",
            "ghs_test2",
            "ghs_test2"
        ]
    );
    assert_eq!(mock.attempts, 2);
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

async fn blob_owned(gh: GitHub) -> Result<Option<String>, github::Failure> {
    blob(&gh).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_exchange_is_never_taken_for_the_calls_answer() {
    let (base, mock) = mock().await;
    // A wrong installation: GitHub's 404 there must not read as "the file isn't there".
    let gh = client(&base, "98");
    let failure = blob(&gh).await.unwrap_err();
    assert_eq!(failure.status, 0);
    assert!(failure.message.contains("404"), "{}", failure.message);
    assert_no_secret(&failure.message);
    assert!(mock.lock().unwrap().used.is_empty());
}

#[test]
fn the_app_is_all_three_variables_or_none() {
    let dir = std::env::temp_dir().join(format!("horizon-mod-registry-app-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let good = dir.join("key.pem");
    std::fs::write(&good, KEY).unwrap();
    // The key's first lines and its end: base64 that reads, DER that doesn't.
    let cut: Vec<&str> = KEY.lines().take(6).chain(KEY.lines().next_back()).collect();
    let bad = dir.join("bad.pem");
    std::fs::write(&bad, cut.join("\n")).unwrap();
    let vars = |set: &[(&str, &str)]| {
        let set: HashMap<String, String> = set
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        AppCredentials::from_vars(move |name| set.get(name).cloned())
    };
    let key_file = good.to_str().unwrap();
    assert!(vars(&[]).unwrap().is_none());
    assert!(vars(&[("REGISTRY_APP_ID", "  ")]).unwrap().is_none());
    let app = vars(&[
        ("REGISTRY_APP_ID", "12345"),
        ("REGISTRY_APP_INSTALLATION_ID", "99"),
        ("REGISTRY_APP_KEY_FILE", key_file),
    ])
    .unwrap()
    .unwrap();
    assert_eq!(app.id(), "12345");
    let refused = [
        vars(&[("REGISTRY_APP_ID", "12345")]),
        vars(&[
            ("REGISTRY_APP_ID", "12345"),
            ("REGISTRY_APP_KEY_FILE", key_file),
        ]),
        vars(&[
            ("REGISTRY_APP_INSTALLATION_ID", "99"),
            ("REGISTRY_APP_KEY_FILE", key_file),
        ]),
        vars(&[
            ("REGISTRY_APP_ID", "12345"),
            ("REGISTRY_APP_INSTALLATION_ID", "not a number"),
            ("REGISTRY_APP_KEY_FILE", key_file),
        ]),
        vars(&[
            ("REGISTRY_APP_ID", "12345"),
            ("REGISTRY_APP_INSTALLATION_ID", "99"),
            ("REGISTRY_APP_KEY_FILE", bad.to_str().unwrap()),
        ]),
        vars(&[
            ("REGISTRY_APP_ID", "12345"),
            ("REGISTRY_APP_INSTALLATION_ID", "99"),
            (
                "REGISTRY_APP_KEY_FILE",
                dir.join("missing.pem").to_str().unwrap(),
            ),
        ]),
    ];
    let messages: Vec<String> = refused
        .into_iter()
        .map(|r| match r {
            Err(message) => message,
            Ok(_) => panic!("refused"),
        })
        .collect();
    assert!(
        messages[0].contains("REGISTRY_APP_INSTALLATION_ID and REGISTRY_APP_KEY_FILE not set"),
        "{}",
        messages[0]
    );
    assert!(
        messages[1].contains("REGISTRY_APP_INSTALLATION_ID not set"),
        "{}",
        messages[1]
    );
    assert!(
        messages[2].contains("REGISTRY_APP_ID not set"),
        "{}",
        messages[2]
    );
    assert!(
        messages[3].contains("REGISTRY_APP_INSTALLATION_ID is not a number"),
        "{}",
        messages[3]
    );
    assert!(messages[4].contains("the App's key"), "{}", messages[4]);
    for message in &messages {
        assert_no_secret(message);
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_log_names_the_bot_never_its_secret() {
    let token = GitHub::new(
        github::Endpoints::default(),
        "o/r".into(),
        "client".into(),
        "secret".into(),
        "ghs_test_static".into(),
    );
    assert_eq!(token.bot(), "GITHUB_BOT_TOKEN");
    let app = token.as_app(AppCredentials::new("12345", INSTALLATION, KEY).unwrap());
    assert_eq!(app.bot(), "the GitHub App 12345");
}

/// `YYYY-MM-DDTHH:MM:SSZ` of Unix seconds, as GitHub writes it.
fn iso(secs: u64) -> String {
    // Howard Hinnant's civil-from-days.
    let z = (secs / 86400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let s = secs % 86400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}
