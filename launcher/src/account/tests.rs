//! The sign-in chain against a local mock of the Microsoft, Xbox Live and Minecraft endpoints
//! (recorded response shapes, mock tokens only), and the keychain abstraction in memory.
use super::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

const UUID: &str = "069a79f444e94726a5befca90e38aaf5";

/// What the mock answers and what it was asked.
#[derive(Default)]
struct Mock {
    /// Polls left before the token endpoint stops saying authorization_pending.
    pending: usize,
    xerr: Option<u64>,
    owns_game: bool,
    no_profile: bool,
    /// Every request: method, path and body.
    log: Vec<(String, String, String)>,
}

fn reply(status: u16, body: Value) -> (u16, String) {
    (status, body.to_string())
}

fn route(mock: &mut Mock, method: &str, path: &str, auth: &str, body: &str) -> (u16, String) {
    let form = |key: &str| {
        body.split('&')
            .filter_map(|p| p.split_once('='))
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.replace('+', " ").replace("%3A", ":").replace("%20", " "))
    };
    match (method, path) {
        ("POST", "/ms/devicecode") => {
            assert_eq!(form("client_id").as_deref(), Some("test-client"));
            assert_eq!(form("scope").as_deref(), Some(SCOPE));
            reply(
                200,
                json!({"user_code": "ABCD-EFGH", "device_code": "dev-1", "verification_uri": "https://www.microsoft.com/link", "expires_in": 900, "interval": 1}),
            )
        }
        ("POST", "/ms/token") => match form("grant_type").as_deref() {
            Some("urn:ietf:params:oauth:grant-type:device_code") if mock.pending > 0 => {
                mock.pending -= 1;
                reply(400, json!({"error": "authorization_pending"}))
            }
            Some("urn:ietf:params:oauth:grant-type:device_code") => {
                assert_eq!(form("device_code").as_deref(), Some("dev-1"));
                reply(
                    200,
                    json!({"access_token": "ms-access-1", "refresh_token": "ms-refresh-1", "expires_in": 3600}),
                )
            }
            Some("refresh_token") if form("refresh_token").as_deref() == Some("ms-refresh-1") => {
                reply(
                    200,
                    json!({"access_token": "ms-access-2", "refresh_token": "ms-refresh-2"}),
                )
            }
            _ => reply(400, json!({"error": "invalid_grant"})),
        },
        ("POST", "/xbl") => {
            let v: Value = serde_json::from_str(body).unwrap();
            assert!(v["Properties"]["RpsTicket"]
                .as_str()
                .unwrap()
                .starts_with("d=ms-access-"));
            assert_eq!(v["RelyingParty"], "http://auth.xboxlive.com");
            reply(
                200,
                json!({"Token": "xbl-token", "DisplayClaims": {"xui": [{"uhs": "uhs-1"}]}}),
            )
        }
        ("POST", "/xsts") => {
            let v: Value = serde_json::from_str(body).unwrap();
            assert_eq!(v["Properties"]["UserTokens"][0], "xbl-token");
            assert_eq!(v["RelyingParty"], "rp://api.minecraftservices.com/");
            match mock.xerr {
                Some(code) => reply(401, json!({"Identity": "0", "XErr": code, "Message": ""})),
                None => reply(
                    200,
                    json!({"Token": "xsts-token", "DisplayClaims": {"xui": [{"uhs": "uhs-1", "xid": "2535400000000000"}]}}),
                ),
            }
        }
        ("POST", "/mc/authentication/login_with_xbox") => {
            let v: Value = serde_json::from_str(body).unwrap();
            assert_eq!(v["identityToken"], "XBL3.0 x=uhs-1;xsts-token");
            reply(
                200,
                json!({"username": "mock", "access_token": "mc-token", "token_type": "Bearer", "expires_in": 86400}),
            )
        }
        ("GET", "/mc/entitlements/mcstore") => {
            assert_eq!(auth, "Bearer mc-token");
            let items = if mock.owns_game {
                json!([{"name": "product_minecraft", "signature": "x"}, {"name": "game_minecraft", "signature": "x"}])
            } else {
                json!([])
            };
            reply(200, json!({"items": items, "signature": "x", "keyId": "1"}))
        }
        ("GET", "/mc/minecraft/profile") if mock.no_profile => reply(
            404,
            json!({"path": "/minecraft/profile", "errorType": "NOT_FOUND", "error": "NOT_FOUND"}),
        ),
        ("GET", "/mc/minecraft/profile") => reply(
            200,
            json!({
                "id": UUID, "name": "Notch",
                "skins": [{"id": "s1", "state": "ACTIVE", "url": "http://textures.minecraft.net/texture/abc", "variant": "SLIM"}],
                "capes": [{"id": "c1", "state": "INACTIVE", "url": "http://textures.minecraft.net/texture/cape1", "alias": "Migrator"},
                          {"id": "c2", "state": "ACTIVE", "url": "http://textures.minecraft.net/texture/cape2", "alias": "Vanilla"}],
            }),
        ),
        _ => reply(404, json!({"error": "NotFound"})),
    }
}

/// A one-thread HTTP/1.1 server: one request per connection.
fn serve(mock: Arc<Mutex<Mock>>) -> Endpoints {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let (method, path) = (
                parts.next().unwrap_or("").to_owned(),
                parts.next().unwrap_or("").to_owned(),
            );
            let (mut length, mut auth) = (0, String::new());
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                let header = header.trim_end();
                if header.is_empty() {
                    break;
                }
                let (k, v) = header.split_once(':').unwrap();
                match k.to_ascii_lowercase().as_str() {
                    "content-length" => length = v.trim().parse().unwrap(),
                    "authorization" => auth = v.trim().to_owned(),
                    _ => {}
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let body = String::from_utf8(body).unwrap();
            let (status, text) = {
                let mut m = mock.lock().unwrap();
                m.log.push((method.clone(), path.clone(), body.clone()));
                route(&mut m, &method, &path, &auth, &body)
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            );
        }
    });
    Endpoints {
        microsoft: format!("{base}/ms"),
        xbox_user: format!("{base}/xbl"),
        xsts: format!("{base}/xsts"),
        minecraft: format!("{base}/mc"),
    }
}

fn mock(f: impl FnOnce(&mut Mock)) -> (Arc<Mutex<Mock>>, Endpoints) {
    let mut m = Mock {
        owns_game: true,
        ..Default::default()
    };
    f(&mut m);
    let m = Arc::new(Mutex::new(m));
    let ep = serve(m.clone());
    (m, ep)
}

#[test]
fn the_whole_chain() {
    let (m, ep) = mock(|m| m.pending = 2);
    let code = device_code(&ep, "test-client").unwrap();
    assert_eq!(
        (code.user_code.as_str(), code.verification_uri.as_str()),
        ("ABCD-EFGH", "https://www.microsoft.com/link")
    );
    let ms = wait_for_sign_in(&ep, "test-client", &code, &|| false).unwrap();
    let (account, secrets) = finish_sign_in(&ep, ms).unwrap();
    assert_eq!(
        account,
        Account {
            uuid: UUID.into(),
            name: "Notch".into(),
            skin: Some((
                "http://textures.minecraft.net/texture/abc".into(),
                "slim".into()
            )),
            cape: Some("http://textures.minecraft.net/texture/cape2".into()),
        }
    );
    assert_eq!(secrets.minecraft_token, "mc-token");
    assert_eq!(secrets.xuid.as_deref(), Some("2535400000000000"));
    assert!(secrets.minecraft_expires_at > now() + 80000);
    // Two pending polls, then the token; then Xbox, XSTS, Minecraft, entitlements, profile.
    let paths: Vec<String> = m
        .lock()
        .unwrap()
        .log
        .iter()
        .map(|(_, p, _)| p.clone())
        .collect();
    assert_eq!(
        paths,
        [
            "/ms/devicecode",
            "/ms/token",
            "/ms/token",
            "/ms/token",
            "/xbl",
            "/xsts",
            "/mc/authentication/login_with_xbox",
            "/mc/entitlements/mcstore",
            "/mc/minecraft/profile"
        ]
    );
    assert!(!format!("{secrets:?}").contains("mc-token"));
}

#[test]
fn refresh_only_when_the_token_runs_out() {
    let (m, ep) = mock(|_| {});
    let account = Account {
        uuid: UUID.into(),
        name: "Notch".into(),
        skin: None,
        cape: None,
    };
    let good = Secrets {
        refresh_token: "ms-refresh-1".into(),
        minecraft_token: "old".into(),
        minecraft_expires_at: now() + 7200,
        xuid: None,
    };
    let (_, kept) = fresh(&ep, "test-client", &account, good).unwrap();
    assert_eq!(kept.minecraft_token, "old");
    assert!(m.lock().unwrap().log.is_empty());
    let stale = Secrets {
        minecraft_expires_at: now() + 60,
        ..kept.clone()
    };
    let (renewed_account, renewed) = fresh(&ep, "test-client", &account, stale).unwrap();
    assert_eq!(
        (
            renewed.refresh_token.as_str(),
            renewed.minecraft_token.as_str()
        ),
        ("ms-refresh-2", "mc-token")
    );
    assert!(renewed_account.skin.is_some());
    let revoked = Secrets {
        refresh_token: "revoked".into(),
        minecraft_expires_at: 0,
        ..kept
    };
    assert_eq!(
        fresh(&ep, "test-client", &account, revoked).unwrap_err(),
        SIGN_IN_AGAIN
    );
}

#[test]
fn ownership_is_asked_again_with_the_saved_token() {
    let secrets = |token: &str| Secrets {
        refresh_token: "ms-refresh-1".into(),
        minecraft_token: token.into(),
        minecraft_expires_at: now() + 7200,
        xuid: None,
    };
    let (m, ep) = mock(|_| {});
    assert_eq!(owns_minecraft(&ep, &secrets("mc-token")), Ok(true));
    assert_eq!(m.lock().unwrap().log.len(), 1);
    let (_, ep) = mock(|m| m.owns_game = false);
    assert_eq!(owns_minecraft(&ep, &secrets("mc-token")), Ok(false));
}

#[test]
fn refusals_have_messages() {
    let tokens = || MsTokens {
        access: "ms-access-1".into(),
        refresh: "r".into(),
    };
    let (_, ep) = mock(|m| m.xerr = Some(2148916233));
    assert!(finish_sign_in(&ep, tokens())
        .unwrap_err()
        .contains("no Xbox profile"));
    let (_, ep) = mock(|m| m.xerr = Some(2148916238));
    assert!(finish_sign_in(&ep, tokens())
        .unwrap_err()
        .contains("child account"));
    let (_, ep) = mock(|m| m.owns_game = false);
    assert_eq!(
        finish_sign_in(&ep, tokens()).unwrap_err(),
        "This account doesn't own Minecraft: Java Edition."
    );
    let (_, ep) = mock(|m| m.no_profile = true);
    assert!(finish_sign_in(&ep, tokens())
        .unwrap_err()
        .contains("no Minecraft profile"));
    assert_eq!(
        oauth_error(&json!({"error": "authorization_declined"})),
        "Sign-in was declined in the browser."
    );
    // Nothing listening: a message, not a URL.
    let dead = Endpoints {
        microsoft: "http://127.0.0.1:9/ms".into(),
        ..Endpoints::live()
    };
    assert_eq!(
        device_code(&dead, "x").unwrap_err(),
        "Could not reach Microsoft. Check your internet connection."
    );
}

#[test]
fn cancelling_stops_the_wait() {
    let (_, ep) = mock(|m| m.pending = 100);
    let code = device_code(&ep, "test-client").unwrap();
    assert_eq!(
        wait_for_sign_in(&ep, "test-client", &code, &|| true).unwrap_err(),
        "Sign-in cancelled."
    );
}

#[test]
fn accounts_keep_secrets_in_the_store_only() {
    let store = MemoryStore::default();
    let mut accounts = Accounts::default();
    let account = Account {
        uuid: UUID.into(),
        name: "Notch".into(),
        skin: None,
        cape: None,
    };
    let secrets = Secrets {
        refresh_token: "r".into(),
        minecraft_token: "mc-secret".into(),
        minecraft_expires_at: 1,
        xuid: None,
    };
    accounts.add(&store, account.clone(), &secrets).unwrap();
    assert_eq!(accounts.selected(), Some(&account));
    assert_eq!(accounts.find("notch"), Some(&account));
    assert_eq!(
        accounts.find("069a79f4-44e9-4726-a5be-fca90e38aaf5"),
        Some(&account)
    );
    let saved = serde_json::to_string(&accounts).unwrap();
    assert!(!saved.contains("mc-secret") && !saved.contains("\"r\""));
    assert!(store.get(UUID).unwrap().unwrap().contains("mc-secret"));
    accounts.remove(&store, UUID).unwrap();
    assert_eq!(
        (accounts.selected(), store.get(UUID).unwrap()),
        (None, None)
    );
    assert!(!store.persistent());
}

#[test]
fn session_files_are_private_and_complete() {
    let dir = std::env::temp_dir().join(format!("horizon-launcher-session-{}", std::process::id()));
    let account = Account {
        uuid: UUID.into(),
        name: "Notch".into(),
        skin: None,
        cape: None,
    };
    let secrets = Secrets {
        refresh_token: "r".into(),
        minecraft_token: "mc-token".into(),
        minecraft_expires_at: 1,
        xuid: Some("25".into()),
    };
    let path = write_session_file(&dir, &account, &secrets).unwrap();
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        v,
        json!({"name": "Notch", "uuid": UUID, "access_token": "mc-token", "user_type": "msa", "xuid": "25"})
    );
    // The refresh token never leaves the keychain.
    assert!(!v.to_string().contains("\"r\""));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let other = write_session_file(&dir, &account, &secrets).unwrap();
    assert_ne!(path, other);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn client_id_is_configuration() {
    // The run-time variable overrides; empty means not configured.
    std::env::set_var("HORIZON_MSA_CLIENT_ID", " ");
    assert_eq!(client_id(), None);
    std::env::set_var(
        "HORIZON_MSA_CLIENT_ID",
        "00000000-1111-2222-3333-444444444444",
    );
    assert_eq!(
        client_id().as_deref(),
        Some("00000000-1111-2222-3333-444444444444")
    );
    std::env::remove_var("HORIZON_MSA_CLIENT_ID");
}

#[test]
fn offline_names_switch_with_accounts() {
    let mut a = Accounts {
        accounts: vec![Account {
            uuid: "abc".into(),
            name: "Ludo".into(),
            skin: None,
            cape: None,
        }],
        selected: Some("abc".into()),
        ..Accounts::default()
    };
    assert_eq!(a.player(), Player::Microsoft("abc".into()));
    assert_eq!(a.name_arg(), None);
    a.play_as(Player::Offline("Builder_1".into())).unwrap();
    assert_eq!(a.player(), Player::Offline("Builder_1".into()));
    assert_eq!(a.name_arg(), Some("Builder_1"));
    // Kept once, as first written, whatever the case.
    a.play_as(Player::Offline("builder_1".into())).unwrap();
    assert_eq!(a.offline, ["Builder_1"]);
    assert_eq!(a.name_arg(), Some("Builder_1"));
    assert!(a.add_offline("no spaces").is_err());
    assert!(a.add_offline("ab").is_err());
    assert!(a.add_offline("seventeen_letters").is_err());
    a.play_as(Player::Microsoft("abc".into())).unwrap();
    assert_eq!(a.name_arg(), None, "an account plays with its own name");
    assert!(a.play_as(Player::Microsoft("zzz".into())).is_err());
    a.play_as(Player::Offline("Builder_1".into())).unwrap();
    a.remove_offline("BUILDER_1").unwrap();
    assert_eq!(a.player(), Player::GameName);
    assert!(a.remove_offline("Builder_1").is_err());
    // An accounts.json from before offline names reads as it was, and writes nothing new.
    let old: Accounts = serde_json::from_str(r#"{"accounts":[],"selected":null}"#).unwrap();
    assert_eq!(old.player(), Player::GameName);
    assert_eq!(
        serde_json::to_string(&old).unwrap(),
        r#"{"accounts":[],"selected":null}"#
    );
}
