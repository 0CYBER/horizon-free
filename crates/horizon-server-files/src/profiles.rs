//! Finding a player's account by name, as an online-mode server does for the
//! names given to `op`, `whitelist` and `ban` (CachedUserNameToIdResolver,
//! MinecraftServicesProfileRepository.findProfileByName): the services'
//! `profiles` / `getByName` endpoint, found through the discovery document.
use crate::{
    json::{self, Json},
    user_lists::NameAndId,
};
use horizon_protocol::rsa::RsaPublicKey;
use std::{fmt, time::Duration};

/// Mojang's services discovery document.
pub const DEFAULT_DISCOVERY: &str = "https://discovery.minecraftservices.com/minecraft/client";

/// The discovery document's URL: `HORIZON_API_DISCOVERY` overrides it like
/// authlib's discovery-host system property (the value after `-D`).
pub fn discovery_url() -> String {
    std::env::var("HORIZON_API_DISCOVERY")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| DEFAULT_DISCOVERY.to_owned())
}

/// StringUtil.isValidPlayerName: at most 16 UTF-16 units, none a control
/// character, a space or above ASCII.
pub fn valid_player_name(name: &str) -> bool {
    name.encode_utf16().count() <= 16 && name.encode_utf16().all(|c| c > 32 && c < 127)
}

/// How a server resolves a name it was given (CachedUserNameToIdResolver.get,
/// without the usercache.json cache): an offline server makes the offline
/// profile; an online one asks the services (for the name in lower case) and
/// knows nobody they don't.
pub fn resolve(name: &str, online: bool) -> Option<NameAndId> {
    if online && valid_player_name(name) {
        find_by_name(&discovery_url(), &name.to_ascii_lowercase())
    } else {
        (!online).then(|| NameAndId::offline(name))
    }
}

/// The profile repository's find-by-name: the account named
/// `name` (any case), or None when there is none or the services don't
/// answer (vanilla logs both as "Couldn't find profile with name").
pub fn find_by_name(discovery: &str, name: &str) -> Option<NameAndId> {
    look_up_by_name(discovery, name).ok().flatten()
}

/// Why the services gave no answer to a lookup, where vanilla only says it found nobody.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoAnswer {
    /// An HTTP status other than the account or "no account by that name": 429 when they're
    /// rate-limiting this address, which they do often for data-center addresses.
    Status(u16),
    /// No answer at all (DNS, connection, TLS, time-out), or one that isn't the expected JSON.
    Unreachable,
}

impl fmt::Display for NoAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NoAnswer::Status(429) => {
                f.write_str("are rate-limiting this address (HTTP 429 Too Many Requests)")
            }
            NoAnswer::Status(code) => write!(f, "answered HTTP {code}"),
            NoAnswer::Unreachable => f.write_str("didn't answer"),
        }
    }
}

/// find_by_name, telling an account that doesn't exist (`Ok(None)`: the endpoint's 404, or 204)
/// from services that didn't answer (`Err`).
pub fn look_up_by_name(discovery: &str, name: &str) -> Result<Option<NameAndId>, NoAnswer> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent(&format!("Horizon/{}", horizon_core::release::VERSION))
        .build();
    let get = |url: &str| match agent.get(url).call() {
        Ok(answer) if answer.status() == 204 => Ok(None),
        Ok(answer) => answer
            .into_string()
            .map(Some)
            .map_err(|_| NoAnswer::Unreachable),
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(ureq::Error::Status(code, _)) => Err(NoAnswer::Status(code)),
        Err(ureq::Error::Transport(_)) => Err(NoAnswer::Unreachable),
    };
    let doc = get(discovery)?
        .and_then(|d| json::parse(&d, true).ok())
        .ok_or(NoAnswer::Unreachable)?;
    let url = endpoint(&doc, "profiles", "getByName")
        .ok_or(NoAnswer::Unreachable)?
        .replace("{name}", &name.to_lowercase());
    Ok(get(&url)?.and_then(|answer| found(&answer, name)))
}

/// MinecraftServicesProfileRepository.findProfilesByNames' patience: a lookup the services
/// don't answer is tried again after DELAY_BETWEEN_FAILURES, until MAX_FAIL_COUNT failures.
pub const LOOKUP_TRIES: u32 = 3;
pub const LOOKUP_RETRY_DELAY: Duration = Duration::from_millis(750);

/// look_up_by_name, tried again like vanilla's bulk lookup when the services don't answer. For
/// Horizon's own tools (`horizon-server init`, list edits on a stopped server), which run once and
/// can't ask again later; the game's commands keep vanilla's single try.
pub fn look_up_by_name_patiently(
    discovery: &str,
    name: &str,
    delay: Duration,
) -> Result<Option<NameAndId>, NoAnswer> {
    let mut tries = 1;
    loop {
        match look_up_by_name(discovery, name) {
            Err(_) if tries < LOOKUP_TRIES => {
                tries += 1;
                std::thread::sleep(delay);
            }
            answer => return answer,
        }
    }
}

/// Finding a player by id on the session service, as the management protocol does for a player given by
/// id that the user cache doesn't know: the `session` / `getProfileById` endpoint, or None.
pub fn find_by_id(discovery: &str, id: u128) -> Option<NameAndId> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent(&format!("Horizon/{}", horizon_core::release::VERSION))
        .build();
    let doc = json::parse(&agent.get(discovery).call().ok()?.into_string().ok()?, true).ok()?;
    let url =
        endpoint(&doc, "session", "getProfileById")?.replace("{profileId}", &format!("{id:032x}"));
    let separator = if url.contains('?') { '&' } else { '?' };
    let answer = agent
        .get(&format!("{url}{separator}unsigned=false"))
        .call()
        .ok()?
        .into_string()
        .ok()?;
    let name = json::parse(&answer, true).ok()?.get("name")?.as_string()?;
    found(&answer, &name).filter(|user| user.id == id)
}

/// The services' public keys, from the
/// discovery document's `authentication` / `getPublicKeys` endpoint. The
/// profile-key keys (`playerCertificateKeys`, ServicesKeyType.PROFILE_KEY)
/// sign the players' chat keys; secure chat checks them. The error is the
/// line vanilla logs, after which it has no keys.
pub fn fetch_profile_key_keys(discovery: &str) -> Result<Vec<RsaPublicKey>, &'static str> {
    const FAILED: &str = "Failed to request Minecraft Services public key";
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent(&format!("Horizon/{}", horizon_core::release::VERSION))
        .build();
    let get = |url: &str| agent.get(url).call().ok()?.into_string().ok();
    let doc = get(discovery)
        .and_then(|d| json::parse(&d, true).ok())
        .ok_or(FAILED)?;
    let url = endpoint(&doc, "authentication", "getPublicKeys").ok_or(FAILED)?;
    profile_key_keys(&get(&url).ok_or(FAILED)?)
}

/// The key set answer: `{"profilePropertyKeys": [...], "playerCertificateKeys":
/// [...]}`, each `{"publicKey": base64 X.509}`. One bad key makes the whole
/// answer malformed (parseList throws).
pub fn profile_key_keys(answer: &str) -> Result<Vec<RsaPublicKey>, &'static str> {
    use base64::Engine;
    const MALFORMED: &str = "Received malformed Minecraft Services public key data";
    let doc = json::parse(answer, true).map_err(|_| MALFORMED)?;
    let list = |name: &str| -> Result<Vec<RsaPublicKey>, &'static str> {
        let Some(entries) = doc.get(name) else {
            return Ok(Vec::new());
        };
        let Json::Array(entries) = entries else {
            return Err(MALFORMED);
        };
        entries
            .iter()
            .map(|e| {
                let text = e
                    .get("publicKey")
                    .and_then(Json::as_string)
                    .ok_or(MALFORMED)?;
                let der = base64::engine::general_purpose::STANDARD
                    .decode(text)
                    .map_err(|_| MALFORMED)?;
                RsaPublicKey::from_der(&der).map_err(|_| MALFORMED)
            })
            .collect()
    };
    list("profilePropertyKeys")?;
    list("playerCertificateKeys")
}

/// The discovery document's URL for a service and endpoint.
fn endpoint(doc: &Json, service: &str, name: &str) -> Option<String> {
    doc.get("discovery")?
        .get(service)?
        .get("endpoints")?
        .get(name)?
        .get("uri")?
        .as_string()
}

/// The answer, authlib's NameAndId: `{"id", "name"}`, the id undashed.
pub fn found(answer: &str, name: &str) -> Option<NameAndId> {
    let profile = json::parse(answer, true).ok()?;
    let found = profile.get("name")?.as_string()?;
    let id = profile.get("id")?.as_string()?.replace('-', "");
    (found.eq_ignore_ascii_case(name) && id.len() == 32).then_some(())?;
    Some(NameAndId {
        id: u128::from_str_radix(&id, 16).ok()?,
        name: found,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_answers() {
        assert!(valid_player_name("Alex_123"));
        assert!(!valid_player_name("two words"));
        assert!(!valid_player_name("ThisNameIsTooLong"));
        assert!(!valid_player_name("caf\u{e9}"));
        assert_eq!(resolve("Alex", false), Some(NameAndId::offline("Alex")));
        assert_eq!(resolve("not valid", true), None);
        let answer = r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"Notch"}"#;
        assert_eq!(
            found(answer, "notch"),
            Some(NameAndId {
                id: 0x069a79f444e94726a5befca90e38aaf5,
                name: "Notch".into()
            })
        );
        assert_eq!(found(answer, "Steve"), None);
        assert_eq!(found("{}", "Notch"), None);
        assert_eq!(found("", "Notch"), None);
    }

    /// A pretend services host on 127.0.0.1: the discovery document, then each name lookup gets
    /// the next of `lookups` (an HTTP status and body). Returns the discovery URL.
    fn services(lookups: &'static [(u16, &'static str)]) -> String {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let doc = format!(
            r#"{{"discovery":{{"profiles":{{"endpoints":{{"getByName":{{"uri":"{base}/name/{{name}}"}}}}}}}}}}"#
        );
        std::thread::spawn(move || {
            let mut lookups = lookups.iter();
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut line = String::new();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                reader.read_line(&mut line).unwrap();
                while reader.read_line(&mut String::new()).unwrap() > 2 {}
                let (status, body) = if line.contains("/discovery") {
                    (200, doc.as_str())
                } else {
                    *lookups.next().unwrap_or(&(500, ""))
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        format!("{base}/discovery")
    }

    const NOTCH: &str = r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"Notch"}"#;

    #[test]
    fn a_lookup_tells_nobody_from_no_answer() {
        let notch = Some(NameAndId {
            id: 0x069a79f444e94726a5befca90e38aaf5,
            name: "Notch".into(),
        });
        assert_eq!(
            look_up_by_name(&services(&[(200, NOTCH)]), "notch"),
            Ok(notch)
        );
        assert_eq!(
            look_up_by_name(&services(&[(404, "{}")]), "nobody"),
            Ok(None)
        );
        assert_eq!(look_up_by_name(&services(&[(204, "")]), "nobody"), Ok(None));
        assert_eq!(
            look_up_by_name(&services(&[(429, "<html>")]), "notch"),
            Err(NoAnswer::Status(429))
        );
        // Vanilla's find-by-name: both are nobody.
        assert_eq!(find_by_name(&services(&[(429, "<html>")]), "notch"), None);
        assert_eq!(
            look_up_by_name("http://127.0.0.1:1/discovery", "notch"),
            Err(NoAnswer::Unreachable)
        );
        assert_eq!(
            NoAnswer::Status(429).to_string(),
            "are rate-limiting this address (HTTP 429 Too Many Requests)"
        );
    }

    #[test]
    fn a_patient_lookup_tries_three_times() {
        let quick = Duration::ZERO;
        let found = services(&[(429, ""), (503, ""), (200, NOTCH)]);
        assert_eq!(
            look_up_by_name_patiently(&found, "notch", quick).map(|u| u.map(|u| u.name)),
            Ok(Some("Notch".into()))
        );
        let refused = services(&[(429, ""), (429, ""), (429, ""), (200, NOTCH)]);
        assert_eq!(
            look_up_by_name_patiently(&refused, "notch", quick),
            Err(NoAnswer::Status(429))
        );
        // Nobody by that name is an answer: not tried again.
        let nobody = services(&[(404, "{}"), (200, NOTCH)]);
        assert_eq!(look_up_by_name_patiently(&nobody, "notch", quick), Ok(None));
    }

    #[test]
    fn services_key_sets() {
        // The 1024-bit test key of horizon-protocol's RSA tests, base64.
        let key = "MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQCrAmB0+SRQqPcm84lbsRF2yo8GXzEAzUFDusT302dO3P+VtBoFJdI2WlScEG7GaFiBs5IXdCD9Mf9cFE/xG6JDEedRzSxrXRgXtq9mhqQ8ldYSQ2EwBeaUW2CTzZUiPOxeiFaCbanS7hsyM2KsHOvqs2WcvEUxmptUDdqBakeY/wIDAQAB";
        let answer = format!(
            r#"{{"profilePropertyKeys":[{{"publicKey":"{key}"}}],"playerCertificateKeys":[{{"publicKey":"{key}"}},{{"publicKey":"{key}"}}]}}"#
        );
        let keys = profile_key_keys(&answer).unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].size(), 128);
        assert_eq!(profile_key_keys("{}").unwrap().len(), 0);
        let bad = r#"{"profilePropertyKeys":[{"publicKey":"AAAA"}],"playerCertificateKeys":[]}"#;
        assert!(profile_key_keys(bad).is_err());
        assert!(profile_key_keys("not json").is_err());
    }
}
