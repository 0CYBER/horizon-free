//! The compound mods keep on a server player (NeoForge's `Entity.getPersistentData`), read and
//! replaced whole, for mods that carry a player between servers.
use super::{Game, native_server::PlayerRead};
use horizon_core::nbt::{Compound, Tag};
use mod_api::{PlayerDataRefusal, PlayerRef};

/// The key of the mods' compound in a player's saved data (NeoForge's `Entity.PERSISTED_NBT_TAG`).
const PERSISTENT_DATA: &str = "NeoForgeData";
/// The sub-compound a death respawn and the first End exit carry over.
const PLAYER_PERSISTED: &str = "PlayerPersisted";

/// The mods' compound in a player's saved data; empty when there is none.
fn persistent_data(saved: Option<&Tag>) -> Compound {
    match saved.and_then(|t| t.get(PERSISTENT_DATA)) {
        Some(Tag::Compound(data)) => data.clone(),
        _ => Compound::new(),
    }
}

/// Puts `data` in a player's saved data as the mods' compound; an empty one takes the key away.
fn set_persistent_data(saved: &mut Option<Tag>, data: Compound) {
    if !matches!(saved, Some(Tag::Compound(_))) {
        *saved = Some(Tag::Compound(Compound::new()));
    }
    let Some(Tag::Compound(root)) = saved else {
        return;
    };
    if data.is_empty() {
        root.remove(&PERSISTENT_DATA.into());
    } else {
        root.insert(PERSISTENT_DATA.into(), Tag::Compound(data));
    }
}

impl Game<'_> {
    /// The mods' compound on a player: the acting player's saved data is the level's for their
    /// turn, another's is their record's.
    pub(super) fn player_persistent_data(&self, player: PlayerRef) -> Option<Compound> {
        if self.remote.is_some() {
            return None;
        }
        Some(match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => persistent_data(self.level.player_extra.as_ref()),
            PlayerRead::Guest(g) => persistent_data(g.saved.as_ref()),
        })
    }

    /// Replaces the mods' compound on a living player, in their turn so it lands in the record
    /// that is saved. Data that would sit 512 levels deep or more in the player's file
    /// (`isTooDeep`, as `/data merge` refuses it) is refused with nothing written.
    pub(super) fn player_set_persistent_data(
        &mut self,
        player: PlayerRef,
        data: Compound,
    ) -> Result<(), PlayerDataRefusal> {
        self.player_write_gate(player)?;
        // The compound sits one level under the root of the player's file.
        if data
            .values()
            .any(|tag| horizon_client::nbt_path::too_deep(tag, 2))
        {
            return Err(PlayerDataRefusal::TooDeep);
        }
        let set = self.native_as_player_handle(player, |game| {
            game.journal_player(player.uuid());
            set_persistent_data(&mut game.level.player_extra, data);
        });
        if set {
            Ok(())
        } else {
            Err(PlayerDataRefusal::Stale)
        }
    }

    /// `PlayerList.respawn` after a death and after a first exit from the End: of the saved
    /// compound only `PlayerPersisted` goes to the new player (NeoForge's
    /// `PlayerEvent.Clone` keeps that sub-compound). Called before the respawn's clone event.
    pub(in crate::game) fn keep_only_player_persisted(&mut self, conn: u32) {
        if let Some(host) = self.lan.as_mut() {
            host.hub
                .guests
                .update_connection(conn, |g| keep_player_persisted(&mut g.saved));
        }
    }
}

/// Drops every key of the mods' compound but `PlayerPersisted`, whatever its type; an empty
/// compound leaves the key away.
fn keep_player_persisted(saved: &mut Option<Tag>) {
    let data = persistent_data(saved.as_ref());
    let mut kept = Compound::new();
    if let Some(persisted) = data.get(&PLAYER_PERSISTED.into()) {
        kept.insert(PLAYER_PERSISTED.into(), persisted.clone());
    }
    if saved.is_some() {
        set_persistent_data(saved, kept);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(entries: Vec<(&str, Tag)>) -> Option<Tag> {
        let mut data = Compound::new();
        for (key, tag) in entries {
            data.insert(key.into(), tag);
        }
        let mut saved = Compound::new();
        saved.insert("Other".into(), Tag::Int(1));
        saved.insert(PERSISTENT_DATA.into(), Tag::Compound(data));
        Some(Tag::Compound(saved))
    }

    /// Only `PlayerPersisted` is kept, of any type; the player file's other keys stay.
    #[test]
    fn only_player_persisted_is_kept_whatever_its_type() {
        let mut compound = saved(vec![
            ("lost", Tag::Int(1)),
            (PLAYER_PERSISTED, Tag::String("kept".into())),
        ]);
        keep_player_persisted(&mut compound);
        assert_eq!(
            compound,
            saved(vec![(PLAYER_PERSISTED, Tag::String("kept".into()))])
        );
        let mut none = saved(vec![("lost", Tag::Int(1))]);
        keep_player_persisted(&mut none);
        let Some(Tag::Compound(root)) = &none else {
            panic!("the player file stays")
        };
        assert!(
            root.get(&PERSISTENT_DATA.into()).is_none(),
            "nothing kept, no key"
        );
        assert_eq!(root.get(&"Other".into()), Some(&Tag::Int(1)));
    }
}
