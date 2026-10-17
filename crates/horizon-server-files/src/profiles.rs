//! Finding a player's account by name, as an online-mode server does for the
//! names given to `op`, `whitelist` and `ban` (CachedUserNameToIdResolver,
//! MinecraftServicesProfileRepository.findProfileByName): the services'
//! `profiles` / `getByName` endpoint, found through the discovery document.
use crate::{
    json::{self, Json},
    user_lists::NameAndId,
};
use std::time::Duration;

/// Mojang's services discovery document.
pub const DEFAULT_DISCOVERY: &str = "https://discovery.minecraftservices.com/minecraft/client";

/// The discovery document's URL: `HORIZON_API_DISCOVERY` overrides it like
/// authlib's `-Dminecraft.api.discovery.host`.
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

/// MinecraftServicesProfileRepository.findProfileByName: the account named
/// `name` (any case), or None when there is none or the services don't
/// answer (vanilla logs both as "Couldn't find profile with name").
pub fn find_by_name(discovery: &str, name: &str) -> Option<NameAndId> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("Horizon/", env!("CARGO_PKG_VERSION")))
        .build();
    let doc = json::parse(&agent.get(discovery).call().ok()?.into_string().ok()?, true).ok()?;
    let url = endpoint(&doc, "profiles", "getByName")?.replace("{name}", &name.to_lowercase());
    found(&agent.get(&url).call().ok()?.into_string().ok()?, name)
}

/// MinecraftServicesDiscoveryService.getUrl(service, endpoint).
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
}
