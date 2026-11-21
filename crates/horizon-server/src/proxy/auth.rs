//! Signing a client in for the game: the proxy does the login the game would do in online mode, since only
//! the side that ran it knows the connection's encryption key (the same exchange as the game's
//! lan/conn.rs, without a connection to the account service's other endpoints).

use super::wire::{
    Connection, RsaPrivateKey, login, offline_uuid, random_bytes, server_hash, translated_json,
};
use std::time::Duration;

/// The account service's discovery document; `HORIZON_API_DISCOVERY` names another one, as the game's does.
const DISCOVERY: &str = "https://discovery.minecraftservices.com/minecraft/client";

/// Who is joining, as the account service (or the offline name) says.
pub struct Profile {
    pub uuid: u128,
    pub name: String,
    pub properties: Vec<login::Property>,
}

/// Online mode's key pair, made once.
pub struct Keys {
    key: RsaPrivateKey,
    public_der: Vec<u8>,
}

impl Keys {
    pub fn generate() -> Result<Self, String> {
        let key = RsaPrivateKey::generate(1024)
            .map_err(|e| format!("Failed to generate key pair: {e}"))?;
        Ok(Self {
            public_der: key.public_der(),
            key,
        })
    }
}

/// A name the login accepts: what `ServerLoginPacketListenerImpl.isValidUsername` does.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 16 && name.chars().all(|c| c > ' ' && c < '\u{7f}')
}

/// The profile of `name`: offline, or after the encryption request and the account service's answer.
/// `Ok(Err(json))` is a refusal to show the client.
pub fn profile(
    c: &mut Connection,
    keys: Option<&Keys>,
    name: &str,
) -> Result<Result<Profile, String>, String> {
    let e = |e: super::wire::Error| e.to_string();
    let Some(keys) = keys else {
        return Ok(Ok(Profile {
            uuid: offline_uuid(name),
            name: name.to_owned(),
            properties: Vec::new(),
        }));
    };
    let mut challenge = [0u8; 4];
    random_bytes(&mut challenge).map_err(e)?;
    c.send(
        &login::Clientbound::Hello(login::Hello {
            server_id: String::new(),
            public_key: keys.public_der.clone(),
            challenge: challenge.to_vec(),
            should_authenticate: true,
        })
        .encode(),
    )
    .map_err(e)?;
    let (id, payload) = c.recv().map_err(e)?;
    let key = match login::Serverbound::decode(id, &payload).map_err(e)? {
        login::Serverbound::Key(k) => k,
        _ => return Err("Unexpected packet during login".into()),
    };
    if keys.key.decrypt(&key.challenge).ok().as_deref() != Some(&challenge[..]) {
        return Err("Protocol error".into());
    }
    let secret: [u8; 16] = keys
        .key
        .decrypt(&key.key)
        .ok()
        .and_then(|s| s.try_into().ok())
        .ok_or("Protocol error")?;
    let digest = server_hash("", &secret, &keys.public_der);
    c.enable_encryption(&secret);
    Ok(match has_joined(name, &digest) {
        Ok(Some(p)) => Ok(p),
        Ok(None) => Err(translated_json(
            "multiplayer.disconnect.unverified_username",
            "Failed to verify username!",
        )),
        Err(_) => Err(translated_json(
            "multiplayer.disconnect.authservers_down",
            "Authentication servers are down. Please try again later, sorry!",
        )),
    })
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .build()
}

fn discovery_url() -> String {
    std::env::var("HORIZON_API_DISCOVERY")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| DISCOVERY.to_owned())
}

/// HasJoinedMinecraftServer: `Ok(None)` when the player did not join with this hash.
fn has_joined(name: &str, hash: &str) -> Result<Option<Profile>, String> {
    let doc: serde_json::Value = agent()
        .get(&discovery_url())
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())
        .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()))?;
    let url = doc
        .pointer("/discovery/session/endpoints/verify/uri")
        .and_then(|u| u.as_str())
        .ok_or("no session verify endpoint")?;
    let separator = if url.contains('?') { '&' } else { '?' };
    let query = format!("username={}&serverId={}", encode(name), encode(hash));
    match agent().get(&format!("{url}{separator}{query}")).call() {
        Ok(response) => Ok(parse_joined(&response.into_string().unwrap_or_default())),
        Err(ureq::Error::Status(204 | 403 | 404, _)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-' | b'*' | b'_' => {
                out.push(char::from(b))
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The answer: `id` (a UUID without dashes), `name` and `properties`. The name is the account's.
pub fn parse_joined(body: &str) -> Option<Profile> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let id = v.get("id")?.as_str()?;
    let uuid = u128::from_str_radix(&id.replace('-', ""), 16).ok()?;
    let properties = v
        .get("properties")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
        .filter_map(|p| {
            Some(login::Property {
                name: p.get("name")?.as_str()?.to_owned(),
                value: p.get("value")?.as_str()?.to_owned(),
                signature: p
                    .get("signature")
                    .and_then(|s| s.as_str())
                    .map(str::to_owned),
            })
        })
        .collect();
    Some(Profile {
        uuid,
        name: v.get("name")?.as_str()?.to_owned(),
        properties,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_account_service_answer_gives_the_real_uuid_name_and_skin() {
        let body = r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"Notch","properties":[{"name":"textures","value":"e30=","signature":"s"}]}"#;
        let p = parse_joined(body).unwrap();
        assert_eq!(p.uuid, 0x069a79f444e94726a5befca90e38aaf5);
        assert_eq!(p.name, "Notch");
        assert_eq!(p.properties.len(), 1);
        assert_eq!(p.properties[0].signature.as_deref(), Some("s"));
        assert!(parse_joined("{}").is_none());
    }

    #[test]
    fn names_are_what_the_game_accepts() {
        assert!(valid_name("Steve_01"));
        assert!(!valid_name(""));
        assert!(!valid_name("a name"));
        assert!(!valid_name("seventeen_chars_xx"));
    }
}
