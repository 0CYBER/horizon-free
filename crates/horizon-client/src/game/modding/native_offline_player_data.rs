//! A saved player file (`players/data/<uuid>.dat`) read and written whole for native server mods,
//! for a player who is not online: the vanilla player compound `PlayerDataStorage` keeps.
use super::Game;
use horizon_client::{
    lan::playerdata,
    save::{read_dat, write_dat},
};
use horizon_core::nbt::{Compound, Tag};
use mod_api::OfflinePlayerDataRefusal;

impl Game<'_> {
    /// Whether the player `uuid` is connected here: the host's own player or a guest.
    fn offline_player_online(&self, uuid: u128) -> bool {
        // The owner is not in the hub's guests, and `player` skips them while a guest is acting.
        self.level.player_uuid.map(playerdata::ints_uuid) == Some(uuid)
            || self.player(uuid).is_some()
            || self
                .lan
                .as_ref()
                .is_some_and(|host| host.hub.guests.iter().any(|g| g.uuid == uuid))
    }

    /// The player file of `uuid` as a compound, None for no file, a file that is not a compound,
    /// a client or a remote world.
    pub(super) fn offline_player_data(&self, uuid: u128) -> Option<Compound> {
        if self.remote.is_some() {
            return None;
        }
        match read_dat(&playerdata::path(&self.world_root, uuid))? {
            Tag::Compound(data) => Some(data),
            _ => None,
        }
    }

    /// Replaces the player file of `uuid` with `data`, refusing while the player is connected.
    pub(super) fn set_offline_player_data(
        &mut self,
        uuid: u128,
        data: Compound,
    ) -> Result<(), OfflinePlayerDataRefusal> {
        if self.remote.is_some() || self.lan.is_none() {
            return Err(OfflinePlayerDataRefusal::NoServer);
        }
        if self.offline_player_online(uuid) {
            return Err(OfflinePlayerDataRefusal::Online);
        }
        playerdata::directory(&self.world_root);
        let path = playerdata::path(&self.world_root, uuid);
        let written = write_dat(&path, Tag::Compound(data));
        if written.is_err() {
            // `write_dat` moves the old file to its backup before the new one goes in: a write
            // that failed in between must not leave the player with no file.
            let backup = path.with_extension("dat_old");
            if !path.exists() && backup.exists() {
                let _ = std::fs::rename(backup, &path);
            }
        }
        written.map_err(|_| OfflinePlayerDataRefusal::Write)
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::*;

    /// A file that is not there reads None; a refused write leaves no file behind, and a file
    /// written as the save does reads back whole, with its values as they were.
    #[test]
    #[ignore = "daily"]
    fn an_offline_player_file_is_read_whole_and_refused_without_a_server() {
        with_game(|game| {
            let dir = tempfile::tempdir().unwrap();
            game.world_root = dir.path().into();
            let mut data = Compound::new();
            data.insert("Score".into(), Tag::Int(7));
            assert_eq!(game.offline_player_data(42), None, "no file yet");
            assert_eq!(
                game.set_offline_player_data(42, data.clone()),
                Err(OfflinePlayerDataRefusal::NoServer),
                "no server open",
            );
            assert!(!playerdata::path(dir.path(), 42).exists());
            playerdata::directory(dir.path());
            write_dat(&playerdata::path(dir.path(), 42), Tag::Compound(data)).unwrap();
            let read = game.offline_player_data(42).unwrap();
            assert_eq!(read.get(&"Score".into()), Some(&Tag::Int(7)));
            assert_eq!(game.offline_player_data(43), None, "another UUID");
        });
    }

    /// A connected guest and the host's own player (while a guest is acting) are refused with
    /// `Online` and their files are untouched; another UUID is written and reads back, and a
    /// second write replaces it whole.
    #[test]
    #[ignore = "daily"]
    fn an_offline_player_file_is_refused_for_the_guest_and_the_owner_and_written_for_others() {
        use super::super::native_entity_refs::tests::bind_acting_guest;
        const OWNER: u128 = 11;
        const GUEST: u128 = 77;
        let score = |n| {
            let mut data = Compound::new();
            data.insert("Score".into(), Tag::Int(n));
            data
        };
        with_game(|game| {
            let dir = tempfile::tempdir().unwrap();
            game.world_root = dir.path().into();
            game.lan_acting = Some(5);
            game.lan_acting_entity = Some(30);
            game.lan_acting_uuid = Some(GUEST);
            game.lan_acting_name = Some("Guest".into());
            game.level.player_uuid = Some(playerdata::uuid_ints(OWNER));
            bind_acting_guest(game);
            for uuid in [OWNER, GUEST] {
                assert_eq!(
                    game.set_offline_player_data(uuid, score(1)),
                    Err(OfflinePlayerDataRefusal::Online),
                    "{uuid} is connected",
                );
                assert!(!playerdata::path(dir.path(), uuid).exists());
            }
            game.set_offline_player_data(42, score(1)).unwrap();
            game.set_offline_player_data(42, score(2)).unwrap();
            let read = game.offline_player_data(42).unwrap();
            assert_eq!(read.get(&"Score".into()), Some(&Tag::Int(2)));
        });
    }
}
