//! A player's skin for server mods (SkinsRestorer): `PlayerRef::set_profile_property` changes
//! their profile, `PlayerRef::refresh` sends them to the other players again.
use super::Game;
use horizon_protocol::packets::login::Property;
use mod_api::PlayerRef;

/// `ExtraCodecs.PROPERTY_MAP`'s limits: 16 properties (`MAX_LISTED_PROPERTIES`, which the list also keeps), names of 64
/// characters, values of 32767 and signatures of 1024. The wire counts UTF-16 code units
/// (`FriendlyByteBuf.writeUtf` and the reader's `readUtf` limits), so a character past the
/// basic plane counts twice.
const MAX_PROPERTIES: usize = 16;
const MAX_PROPERTY_NAME: usize = 64;
const MAX_PROPERTY_VALUE: usize = 32767;
const MAX_PROPERTY_SIGNATURE: usize = 1024;

/// A string's length in UTF-16 code units, what the wire limits count.
fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The overrides a mod set (`overrides`, over the properties the player list gives, `listed`) with `property`
/// in place of the one of its name, or added; None when the profile already lists 16 and the name is new.
fn with_property(
    mut overrides: Vec<Property>,
    listed: &[Property],
    property: Property,
) -> Option<Vec<Property>> {
    if !listed.iter().any(|p| p.name == property.name) && listed.len() >= MAX_PROPERTIES {
        return None;
    }
    match overrides.iter_mut().find(|p| p.name == property.name) {
        Some(old) => *old = property,
        None => overrides.push(property),
    }
    Some(overrides)
}

impl Game<'_> {
    /// Sets property `name` of the player's profile, replacing one of the same name.
    pub(super) fn player_set_profile_property(
        &mut self,
        player: PlayerRef,
        name: &str,
        value: &str,
        signature: Option<&str>,
    ) -> bool {
        if name.is_empty()
            || utf16_len(name) > MAX_PROPERTY_NAME
            || utf16_len(value) > MAX_PROPERTY_VALUE
            || signature.is_some_and(|s| utf16_len(s) > MAX_PROPERTY_SIGNATURE)
        {
            return false;
        }
        let Some(i) = self.connected_guest_index(player) else {
            return false;
        };
        let Some(host) = self.lan.as_mut() else {
            return false;
        };
        let property = Property {
            name: name.to_owned(),
            value: value.to_owned(),
            signature: signature.map(str::to_owned),
        };
        // What a mod sets is an override by name over the profile the player list gives (the integrated
        // server's owner has the account's properties, also when they arrive after this call), so one
        // property added neither takes the others away nor hides a skin fetched later.
        let listed = host.hub.listed_profile_properties(i);
        let overrides = host.hub.guests[i].profile_overrides.clone();
        let Some(updated) = with_property(overrides, &listed, property) else {
            return false;
        };
        // Kept apart from the login properties `player_profile` reads.
        host.hub.guests.update(i, |g| g.profile_overrides = updated);
        true
    }

    /// The index of a connected player's record in the hub's guest list.
    fn connected_guest_index(&self, player: PlayerRef) -> Option<usize> {
        let conn = self.online_player_record(player)?.conn;
        self.lan.as_ref()?.hub.guests.connection_index(conn)
    }

    /// Sends the player to every other player again (`Hub::refresh_player`).
    pub(super) fn player_refresh(&mut self, player: PlayerRef) -> bool {
        let Some(i) = self.connected_guest_index(player) else {
            return false;
        };
        let Some(host) = self.lan.as_mut() else {
            return false;
        };
        let uuid = host.hub.guests[i].uuid;
        host.hub.refresh_player(uuid);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn property(name: &str, value: &str) -> Property {
        Property {
            name: name.to_owned(),
            value: value.to_owned(),
            signature: None,
        }
    }

    /// The wire counts UTF-16 units: a character past the basic plane is two.
    #[test]
    fn lengths_are_counted_in_utf16_units() {
        assert_eq!(utf16_len("abc"), 3);
        assert_eq!(utf16_len("\u{1F600}"), 2);
        assert!(utf16_len(&"\u{1F600}".repeat(33)) > MAX_PROPERTY_NAME);
        assert!("\u{1F600}".repeat(32).encode_utf16().count() <= MAX_PROPERTY_NAME);
    }

    /// A mod's property is an override by name over what the player list gives: one of a new name is
    /// added to the overrides (the skin stays in the list), one of the same name replaces, and a full
    /// listed profile refuses a new name only.
    #[test]
    fn a_property_is_an_override_by_name_and_the_sixteenth_is_the_last() {
        let skin = vec![property("textures", "owner")];
        let added = with_property(Vec::new(), &skin, property("example:badge", "1")).unwrap();
        assert_eq!(
            added,
            vec![property("example:badge", "1")],
            "the skin is not copied in"
        );
        let replaced = with_property(added, &skin, property("textures", "mod")).unwrap();
        assert_eq!(replaced.len(), 2);
        assert_eq!(replaced[1].value, "mod");
        let listed: Vec<_> = (0..MAX_PROPERTIES)
            .map(|i| property(&format!("p{i}"), "v"))
            .collect();
        assert!(with_property(Vec::new(), &listed, property("new", "v")).is_none());
        assert_eq!(
            with_property(Vec::new(), &listed, property("p3", "w")).unwrap(),
            vec![property("p3", "w")]
        );
    }
}
