//! Game profiles the server knows by UUID or name (`profile_by_uuid`, `profile_by_name`), for server
//! mods; never asked of Mojang.
use super::Game;
use horizon_protocol::packets::login::Property;
use mod_api::{GameProfile, ProfileProperty};

fn profile_of(uuid: u128, name: &str, properties: &[Property]) -> GameProfile {
    GameProfile::new(
        uuid,
        name.to_owned(),
        properties
            .iter()
            .map(|p| ProfileProperty::new(p.name.clone(), p.value.clone(), p.signature.clone()))
            .collect(),
    )
}

/// Names equal as the user cache compares them (`UserCache` lower-cases both with `Locale.ROOT`, not
/// ASCII only), so a player online as `Élodie` is found by `élodie` as well.
fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

impl Game<'_> {
    /// The profile of the first online, non-fake player `matches`, with the properties the player list gives
    /// (the integrated server's owner has the account's, also when they arrived after login, with a mod's
    /// overrides by name over them). `player_profile` keeps the login properties only.
    fn online_profile(
        &self,
        matches: impl Fn(&horizon_client::lan::guest::Guest) -> bool,
    ) -> Option<GameProfile> {
        let host = self.lan.as_ref()?;
        let g = host
            .hub
            .online_players()
            .find(|g| !g.is_fake() && matches(g))?;
        let properties = host.hub.guests.connection_index(g.conn).map_or_else(
            || g.properties.clone(),
            |i| host.hub.listed_profile_properties(i),
        );
        Some(profile_of(g.uuid, &g.name, &properties))
    }

    /// An online player's profile by UUID, else the user cache's name.
    pub(super) fn profile_by_uuid(&self, uuid: u128) -> Option<GameProfile> {
        if let Some(profile) = self.online_profile(|g| g.uuid == uuid) {
            return Some(profile);
        }
        if !self.is_dedicated_server() {
            return None;
        }
        let user = super::dedicated::cached_user_by_id(uuid)?;
        Some(GameProfile::new(user.id, user.name.clone(), Vec::new()))
    }

    /// An online player's profile by name (any case), else an unexpired user cache entry.
    pub(super) fn profile_by_name(&self, name: &str) -> Option<GameProfile> {
        if let Some(profile) = self.online_profile(|g| same_name(&g.name, name)) {
            return Some(profile);
        }
        if !self.is_dedicated_server() {
            return None;
        }
        let user = super::dedicated::cached_user_by_name(name)?;
        Some(GameProfile::new(user.id, user.name.clone(), Vec::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::same_name;

    #[test]
    fn names_compare_in_any_case_not_only_ascii() {
        assert!(same_name("Steve", "sTEVE"));
        assert!(same_name("\u{c9}lodie", "\u{e9}lodie"));
        assert!(!same_name("Steve", "Steve2"));
    }
}
