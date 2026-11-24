use super::{
    FriendData, Presence, PresenceStatus, ResultCode, lock, model::presence_of, parse_friends,
};
use crate::auth::{self, AuthSession};
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// `MinecraftServicesFriendsService`: the HTTP client and its caches.
#[derive(Default)]
pub(super) struct Service {
    pub(super) friends_etag: Option<String>,
    presence_etag: Option<String>,
    pub(super) friends_cache: FriendData,
    presence_cache: Vec<Presence>,
    /// `getFriendsPollInterval`/`getPresencePollInterval`: the service's `Retry-After`.
    pub(super) friends_poll: Option<Duration>,
    pub(super) presence_poll: Option<Duration>,
    /// `requestCooldown`: ten seconds between `getFriendData` requests.
    pub(super) cooldown: Option<Instant>,
    /// `requestPending`: one list request at a time.
    pub(super) pending: bool,
}

impl Service {
    /// `canMakeRequest`.
    pub(super) fn can_make_request(&self) -> bool {
        self.cooldown.is_none_or(|at| Instant::now() > at)
    }

    /// `requestFriendData` with the list request's answer. A list that parses replaces the
    /// cached lists and their ETag, which an answer without one clears. A 304, an empty body
    /// or one that doesn't parse keeps both, so the next request still asks with the ETag of
    /// the lists it holds (#15084). False for a body that doesn't parse: authlib's ERROR result.
    pub(super) fn take_friends(&mut self, answer: Answer) -> bool {
        if answer.status == 304 || answer.body.is_empty() {
            return true;
        }
        let Some(data) = parse_friends(&answer.body) else {
            return false;
        };
        self.friends_cache = data;
        self.friends_etag = answer.etag;
        true
    }
}

/// `MinecraftServicesFriendsService.presence`: post one status, cache the answer.
/// `None` means the answer said nothing new (304, an empty body or a failure); the
/// handler keeps its last presence then.
pub(super) fn post_presence(status: PresenceStatus) -> Option<Vec<Presence>> {
    let session = session()?;
    let url = url("updatePresence")?;
    let body = json!({"status": status.wire()}).to_string();
    let etag = lock().service.presence_etag.clone();
    let answer = send_json(&session, "POST", &url, &body, etag.as_deref()).ok()?;
    let mut inner = lock();
    if let Some(retry) = answer.retry_after {
        inner.service.presence_poll = Some(retry);
    }
    if let Some(etag) = answer.etag {
        inner.service.presence_etag = Some(etag);
    }
    if answer.status == 304 || answer.body.is_empty() {
        return None;
    }
    // `PresenceResponse`: a missing `presence` field is an empty list, replacing the
    // cached one; malformed JSON keeps the cache, as authlib's exception path does.
    if let Ok(value) = serde_json::from_str::<Value>(&answer.body) {
        let presence = presence_of(&value);
        inner.service.presence_cache = presence.clone();
        return Some(presence);
    }
    None
}

/// `MinecraftServicesFriendsService.getFriendData`'s request.
pub(super) fn fetch_friends(session: &AuthSession, etag: Option<&str>) -> Result<Answer, Failure> {
    let url = url("getFriends").ok_or(Failure {
        code: ResultCode::ServiceNotAvailable,
    })?;
    send_json(session, "GET", &url, "", etag)
}

/// One HTTP answer: its status, cache headers and body.
pub(super) struct Answer {
    pub(super) status: u16,
    pub(super) etag: Option<String>,
    pub(super) retry_after: Option<Duration>,
    pub(super) body: String,
}

pub(super) struct Failure {
    pub(super) code: ResultCode,
}

/// The discovery document's `player` service endpoint.
pub(super) fn session() -> Option<Arc<AuthSession>> {
    auth::current().filter(|s| !s.expired())
}

pub(super) fn url(name: &str) -> Option<String> {
    let doc = auth::discovery().ok()?;
    auth::endpoint(&doc, "player", name)
}

/// authlib's `MinecraftClient`: the bearer token, `If-None-Match` when a cache has an
/// `ETag`, and `Retry-After` back to the caller.
pub(super) fn send_json(
    session: &AuthSession,
    method: &str,
    url: &str,
    body: &str,
    etag: Option<&str>,
) -> Result<Answer, Failure> {
    let bearer = format!("Bearer {}", session.access_token());
    let agent = auth::agent();
    let mut request = match method {
        "GET" => agent.get(url),
        "POST" => agent.post(url),
        "PUT" => agent.put(url),
        _ => {
            return Err(Failure {
                code: ResultCode::Error,
            });
        }
    };
    request = request.set("Authorization", &bearer);
    if let Some(etag) = etag {
        request = request.set("If-None-Match", etag);
    }
    let response = if method == "GET" {
        request.call()
    } else {
        request
            .set("Content-Type", "application/json; charset=utf-8")
            .send_string(body)
    }
    .map_err(http_error)?;
    let status = response.status();
    let etag = response.header("ETag").map(str::to_owned);
    let retry_after = response.header("Retry-After").and_then(parse_retry_after);
    if status == 304 {
        return Ok(Answer {
            status,
            etag,
            retry_after,
            body: String::new(),
        });
    }
    let body = response.into_string().map_err(|_| Failure {
        code: ResultCode::Error,
    })?;
    Ok(Answer {
        status,
        etag,
        retry_after,
        body,
    })
}

/// authlib's `MinecraftClient.parseRetryAfter`: whole seconds above zero.
fn parse_retry_after(value: &str) -> Option<Duration> {
    value
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
}

/// `MinecraftServicesFriendsService.handleHttpError`.
fn http_error(error: ureq::Error) -> Failure {
    let code = match error {
        ureq::Error::Status(status, _) => match status {
            429 => ResultCode::TooManyRequests,
            500.. => ResultCode::ServiceNotAvailable,
            403 => ResultCode::Forbidden,
            400 => ResultCode::UnknownProfile,
            401 => ResultCode::Unauthorized,
            _ => ResultCode::Error,
        },
        ureq::Error::Transport(_) => ResultCode::ConnectionIssue,
    };
    Failure { code }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #15084, after authlib's `requestFriendData`: only a list that parses moves the ETag
    /// the next request sends.
    #[test]
    fn only_a_parsed_friends_list_replaces_the_cached_lists_and_their_etag() {
        let list = |name: &str| {
            format!(
                r#"{{"friends":[{{"profileId":"0123456789abcdef0123456789abcdef","name":"{name}"}}]}}"#
            )
        };
        let answer = |status: u16, etag: Option<&str>, body: &str| Answer {
            status,
            etag: etag.map(str::to_owned),
            retry_after: None,
            body: body.to_owned(),
        };
        let held = |s: &Service| {
            let names: Vec<String> = s
                .friends_cache
                .friends
                .iter()
                .map(|f| f.name.clone())
                .collect();
            (s.friends_etag.clone(), names)
        };
        let seeded = || {
            let mut s = Service::default();
            assert!(s.take_friends(answer(200, Some("\"A\""), &list("Alice"))));
            s
        };
        let alice = (Some("\"A\"".to_owned()), vec!["Alice".to_owned()]);
        assert_eq!(held(&seeded()), alice);
        // A new list without an ETag: the new list, and no ETag on the next request.
        let mut s = seeded();
        assert!(s.take_friends(answer(200, None, &list("Bob"))));
        assert_eq!(held(&s), (None, vec!["Bob".to_owned()]));
        // Nothing new, or nothing readable, under another ETag: the old list and its ETag.
        for (status, body, parsed) in [(304, "", true), (200, "", true), (200, "{", false)] {
            let mut s = seeded();
            assert_eq!(s.take_friends(answer(status, Some("\"B\""), body)), parsed);
            assert_eq!(held(&s), alice, "{status} {body:?}");
        }
    }

    #[test]
    fn retry_after_reads_whole_positive_seconds() {
        assert_eq!(parse_retry_after("30"), Some(Duration::from_secs(30)));
        assert_eq!(parse_retry_after(" 5 "), Some(Duration::from_secs(5)));
        assert_eq!(parse_retry_after("0"), None);
        assert_eq!(parse_retry_after("later"), None);
    }
}
