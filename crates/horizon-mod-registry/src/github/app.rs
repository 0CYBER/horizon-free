//! Signing in as the registry's GitHub App instead of with one person's token: a short JWT signed with the
//! App's private key is exchanged for an installation token, which lasts an hour and is kept until shortly
//! before it ends. Nothing here puts the key, the JWT or a token in a message.
use super::{Failure, Result, timed_out};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use ring::{
    rand::SystemRandom,
    signature::{RSA_PKCS1_SHA256, RsaKeyPair},
};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Mutex, MutexGuard, PoisonError, TryLockError},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// The environment's names for the App, all three or none.
pub const APP_ID: &str = "REGISTRY_APP_ID";
pub const INSTALLATION_ID: &str = "REGISTRY_APP_INSTALLATION_ID";
pub const KEY_FILE: &str = "REGISTRY_APP_KEY_FILE";

/// A token is replaced this long before GitHub ends it: longer than a file's upload may take.
const REFRESH_BEFORE: u64 = 10 * 60;
/// After a failed refresh the old token still serves while it lasts this long: past a file's upload.
const USABLE_FOR: u64 = 6 * 60;
/// A failed exchange answers every call this long, instead of each call trying again.
const COOLDOWN: Duration = Duration::from_secs(30);

/// Who the service is to GitHub when it writes to the registry: the App, on one installation.
pub struct AppCredentials {
    /// The App's ID or client ID: the JWT's `iss`.
    iss: String,
    installation: u64,
    key: RsaKeyPair,
}

impl AppCredentials {
    /// `pem` is the App's private key as GitHub hands it out (PKCS#1, `BEGIN RSA PRIVATE KEY`), or PKCS#8.
    pub fn new(
        app_id: &str,
        installation_id: &str,
        pem: &str,
    ) -> std::result::Result<Self, String> {
        let iss = app_id.trim();
        if iss.is_empty() || !iss.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.') {
            return Err(format!("{APP_ID} is not an App ID or client ID"));
        }
        let installation = installation_id
            .trim()
            .parse()
            .map_err(|_| format!("{INSTALLATION_ID} is not a number"))?;
        Ok(AppCredentials {
            iss: iss.to_owned(),
            installation,
            key: private_key(pem)?,
        })
    }

    /// The App from `var` (the environment): None when none of its three variables is set, an error when only
    /// some are or the key file can't be read as a key.
    pub fn from_vars(
        var: impl Fn(&str) -> Option<String>,
    ) -> std::result::Result<Option<Self>, String> {
        let names = [APP_ID, INSTALLATION_ID, KEY_FILE];
        let values = names.map(|name| var(name).filter(|v| !v.trim().is_empty()));
        if values.iter().all(Option::is_none) {
            return Ok(None);
        }
        let missing: Vec<&str> = names
            .iter()
            .zip(&values)
            .filter(|(_, v)| v.is_none())
            .map(|(name, _)| *name)
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "{APP_ID}, {INSTALLATION_ID} and {KEY_FILE} go together: {} not set",
                missing.join(" and ")
            ));
        }
        let [Some(app_id), Some(installation_id), Some(key_file)] = values else {
            unreachable!("all three are set")
        };
        let path = Path::new(key_file.trim());
        let pem = std::fs::read_to_string(path)
            .map_err(|e| format!("{KEY_FILE} {}: {e}", path.display()))?;
        AppCredentials::new(&app_id, &installation_id, &pem)
            .map(Some)
            .map_err(|e| format!("{e} ({KEY_FILE} {})", path.display()))
    }

    /// The App's ID or client ID, for the log.
    pub fn id(&self) -> &str {
        &self.iss
    }

    /// The JWT that asks for an installation token: RS256, issued a minute back for clock drift, valid ten
    /// minutes in all (GitHub's longest).
    fn jwt(&self, now: u64) -> Result<String> {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        // GitHub takes the numeric App ID or the client ID.
        let iss = self
            .iss
            .parse::<u64>()
            .map_or_else(|_| json!(self.iss), |n| json!(n));
        let claims = json!({"iat": now.saturating_sub(60), "exp": now + 540, "iss": iss});
        let message = format!("{header}.{}", URL_SAFE_NO_PAD.encode(claims.to_string()));
        let mut signature = vec![0; self.key.public().modulus_len()];
        self.key
            .sign(
                &RSA_PKCS1_SHA256,
                &SystemRandom::new(),
                message.as_bytes(),
                &mut signature,
            )
            .map_err(|_| Failure {
                status: 0,
                message: "the App's key could not sign".into(),
                timed_out: false,
            })?;
        Ok(format!("{message}.{}", URL_SAFE_NO_PAD.encode(signature)))
    }
}

/// The App's key from its PEM text. The errors name what is wrong, never the text.
fn private_key(pem: &str) -> std::result::Result<RsaKeyPair, String> {
    const NOT_A_KEY: &str = "the App's key is not a PEM RSA private key";
    let mut lines = pem.lines().map(str::trim);
    let begin = lines
        .by_ref()
        .find(|l| l.starts_with("-----BEGIN"))
        .ok_or(NOT_A_KEY)?;
    let pkcs1 = match begin {
        "-----BEGIN RSA PRIVATE KEY-----" => true,
        "-----BEGIN PRIVATE KEY-----" => false,
        _ => return Err(NOT_A_KEY.into()),
    };
    let body: String = lines.take_while(|l| !l.starts_with("-----END")).collect();
    let der = STANDARD.decode(body).map_err(|_| NOT_A_KEY.to_owned())?;
    let key = if pkcs1 {
        RsaKeyPair::from_der(&der)
    } else {
        RsaKeyPair::from_pkcs8(&der)
    };
    key.map_err(|e| format!("the App's key was refused: {e}"))
}

struct Cached {
    token: String,
    /// When GitHub ends it, Unix seconds.
    expires: u64,
}

struct State {
    cached: Option<Cached>,
    /// The last failed exchange: when, and what it answered (a message without secrets, timed out).
    failed: Option<(Instant, String, bool)>,
}

impl State {
    /// The kept token when it lasts more than `left` seconds past `now`.
    fn usable(&self, now: u64, left: u64) -> Option<String> {
        self.cached
            .as_ref()
            .filter(|c| c.expires > now + left)
            .map(|c| c.token.clone())
    }

    /// The answer that needs no exchange: a token that isn't due, or during a cooldown the old token while it
    /// lasts, else the failure. None: an exchange is due.
    fn settled(&self, now: u64) -> Option<Result<String>> {
        if let Some(token) = self.usable(now, REFRESH_BEFORE) {
            return Some(Ok(token));
        }
        let (_, message, timed_out) = self
            .failed
            .as_ref()
            .filter(|(at, ..)| at.elapsed() < COOLDOWN)?;
        Some(self.usable(now, USABLE_FOR).ok_or_else(|| Failure {
            status: 0,
            message: message.clone(),
            timed_out: *timed_out,
        }))
    }
}

/// The App and its current installation token, shared by every clone of the client.
pub(super) struct Installation {
    credentials: AppCredentials,
    state: Mutex<State>,
    /// Held during an exchange, so concurrent calls make one. Never held with `state`'s lock across the call.
    refreshing: Mutex<()>,
}

impl Installation {
    pub(super) fn new(credentials: AppCredentials) -> Installation {
        Installation {
            credentials,
            state: Mutex::new(State {
                cached: None,
                failed: None,
            }),
            refreshing: Mutex::default(),
        }
    }

    pub(super) fn id(&self) -> &str {
        self.credentials.id()
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The installation token: the kept one, or a new one when it ends within `REFRESH_BEFORE`. While one
    /// call fetches it, the others use the old token while it lasts `USABLE_FOR`, else wait for that call; a
    /// failed exchange answers every call for `COOLDOWN` (with the old token while it lasts).
    pub(super) fn token(
        &self,
        agent: &ureq::Agent,
        api: &str,
        timeout: Duration,
    ) -> Result<String> {
        let refreshing = {
            let state = self.state();
            let now = unix_now();
            if let Some(answer) = state.settled(now) {
                return answer;
            }
            // Decided under `state`'s lock: a token another call just dropped (`forget`) is never handed out.
            match self.refreshing.try_lock() {
                Ok(guard) => Some(guard),
                Err(TryLockError::Poisoned(e)) => Some(e.into_inner()),
                Err(TryLockError::WouldBlock) => match state.usable(now, USABLE_FOR) {
                    Some(token) => return Ok(token),
                    None => None,
                },
            }
        };
        let _refreshing = match refreshing {
            Some(guard) => guard,
            None => self
                .refreshing
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        };
        // What the exchange this call waited for left.
        let now = unix_now();
        if let Some(answer) = self.state().settled(now) {
            return answer;
        }
        match self.exchange(agent, api, timeout, now) {
            Ok(fresh) => {
                let token = fresh.token.clone();
                let mut state = self.state();
                state.cached = Some(fresh);
                state.failed = None;
                Ok(token)
            }
            Err(e) => {
                let mut state = self.state();
                state.failed = Some((Instant::now(), e.message.clone(), e.timed_out));
                // The exchange took time: what is left is counted from now.
                state.usable(unix_now(), USABLE_FOR).ok_or(e)
            }
        }
    }

    /// Drops `token` after GitHub refused it, unless another call already replaced it.
    pub(super) fn forget(&self, token: &str) {
        let mut state = self.state();
        if state.cached.as_ref().is_some_and(|c| c.token == token) {
            state.cached = None;
            // The cooldown only kept this token serving: with it gone, the retry must try an exchange.
            state.failed = None;
        }
    }

    /// `POST /app/installations/{id}/access_tokens`. Every failure is status 0: a caller that reads a 404 as
    /// "not there" must never take the exchange's own 404 for its answer.
    fn exchange(
        &self,
        agent: &ureq::Agent,
        api: &str,
        timeout: Duration,
        now: u64,
    ) -> Result<Cached> {
        let jwt = self.credentials.jwt(now)?;
        let response = agent
            .post(&format!(
                "{api}/app/installations/{}/access_tokens",
                self.credentials.installation
            ))
            .timeout(timeout)
            .set("Authorization", &format!("Bearer {jwt}"))
            .set("Accept", "application/vnd.github+json")
            .set("X-GitHub-Api-Version", "2022-11-28")
            .call()
            .map_err(refused)?;
        let unreadable = |timed_out| Failure {
            status: 0,
            message: "the App's token: GitHub's answer has no token and expiry".into(),
            timed_out,
        };
        let answer: Value = response
            .into_json()
            .map_err(|e| unreadable(timed_out(&e)))?;
        match (
            answer["token"].as_str(),
            answer["expires_at"].as_str().and_then(unix_time),
        ) {
            // Only a token's own characters: anything else would make the HTTP client refuse the header and
            // print it in its error.
            (Some(token), Some(expires))
                if !token.is_empty()
                    && token
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b)) =>
            {
                Ok(Cached {
                    token: token.to_owned(),
                    expires,
                })
            }
            _ => Err(unreadable(false)),
        }
    }
}

/// A failed exchange: GitHub's status, or the kind of the transport's error. Never their text, which may
/// repeat what was sent.
fn refused(e: ureq::Error) -> Failure {
    match e {
        ureq::Error::Status(status, _) => Failure {
            status: 0,
            message: format!("the App's token: GitHub answered {status}"),
            timed_out: false,
        },
        other => Failure {
            status: 0,
            timed_out: timed_out(&other),
            message: format!("the App's token: {}", other.kind()),
        },
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Unix seconds of GitHub's `YYYY-MM-DDTHH:MM:SSZ`.
fn unix_time(text: &str) -> Option<u64> {
    let b = text.as_bytes();
    if b.len() != 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
    {
        return None;
    }
    let n = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (h, min, s) = (n(11..13)?, n(14..16)?, n(17..19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || h > 23 || min > 59 || s > 60 {
        return None;
    }
    // Howard Hinnant's days-from-civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + h * 3600 + min * 60 + s).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn githubs_times_read_as_unix_seconds() {
        assert_eq!(unix_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix_time("2016-07-11T22:14:10Z"), Some(1_468_275_250));
        assert_eq!(unix_time("2024-02-29T12:00:00Z"), Some(1_709_208_000));
        assert_eq!(unix_time("2016-07-11 22:14:10"), None);
        assert_eq!(unix_time("2016-13-11T22:14:10Z"), None);
        assert_eq!(unix_time(""), None);
    }

    #[test]
    fn a_key_that_is_not_one_is_refused_without_its_text() {
        let pem = include_str!("../../tests/app-key.pem");
        assert!(private_key(pem).is_ok());
        assert!(private_key(&pem.replace('\n', "\r\n")).is_ok());
        let body_line = pem.lines().nth(3).unwrap();
        let cut: String = pem
            .lines()
            .take(6)
            .chain(pem.lines().next_back())
            .collect::<Vec<_>>()
            .join("\n");
        let broken: [&str; 5] = [
            "",
            "not a key",
            &pem.replace("RSA PRIVATE KEY", "EC PRIVATE KEY"),
            &cut,
            &pem.replace(body_line, "!!!!"),
        ];
        for broken in broken {
            let Err(error) = private_key(broken) else {
                panic!("refused")
            };
            assert!(!error.contains(body_line), "{error}");
        }
    }
}
