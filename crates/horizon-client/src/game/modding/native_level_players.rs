//! The players of a level and of a chunk through `LevelRef::online_players` and
//! `ChunkRef::watchers` (docs/modding/src/how-to/native-mods.md, "Entity and level
//! handles"). Both name players by the same UUID enumeration the spawn code
//! reads, and resolve each through `Host::player`, so a guest's turn names its
//! player once and a player whose handle cannot resolve is left out.
use super::Game;
use mod_api::{ChunkRef, LevelRef, PlayerRef};

impl Game<'_> {
    /// `ServerLevel.players`: the online players of an open server level, each once.
    pub(super) fn level_online_players(&self, level: LevelRef) -> Option<Vec<PlayerRef>> {
        self.native_level_world(level)?;
        Some(
            self.level_player_uuids(level.dimension_id())
                .filter_map(|uuid| self.player(uuid))
                .collect(),
        )
    }

    /// `ChunkMap.getPlayers(pos, false)`: the players the chunk was sent to and is
    /// inside the view of.
    pub(super) fn chunk_watchers(&self, chunk: ChunkRef) -> Option<Vec<PlayerRef>> {
        self.native_level_world(chunk.level())?;
        let dimension = chunk.level().dimension_id();
        let [x, z] = chunk.pos();
        Some(
            self.lan
                .iter()
                .flat_map(|h| h.hub.guests.iter())
                .filter(|g| g.tracks_chunk(dimension, (x, z)))
                .filter_map(|g| self.player(g.uuid))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        Dim,
        native_guest_turn_tests::{place_host, with_guests},
    };
    use mod_api::{ChunkRef, LevelRef, PlayerRef, WorldEpoch};

    /// Open the Nether and go back, so it is a parked level of the server.
    fn park_the_nether(game: &mut super::Game<'_>) {
        game.enter_level(Dim::Nether).unwrap();
        game.enter_level(Dim::Overworld).unwrap();
    }

    fn uuids(players: Option<Vec<PlayerRef>>) -> Option<Vec<u128>> {
        players.map(|players| players.into_iter().map(PlayerRef::uuid).collect())
    }

    /// The host and two guests, each named once with their own entity, in and
    /// out of guest turns; a guest in another level is that level's.
    #[test]
    #[ignore = "daily"]
    fn level_players_name_host_and_guests_once_during_turns() {
        with_guests(|game| {
            park_the_nether(game);
            let _serving = super::super::integrated::tests::install_serving(game, false);
            place_host(game);
            let level = game.level(game.dim.id()).unwrap();
            let players = level.online_players(game).unwrap();
            assert_eq!(
                players
                    .iter()
                    .map(|p| (p.uuid(), p.entity().id()))
                    .collect::<Vec<_>>(),
                [(99, 99), (12, 12), (13, 13)]
            );
            game.as_guest(12, |game| {
                assert_eq!(uuids(level.online_players(game)), Some(vec![12, 13]));
                let acting = level.online_players(game).unwrap()[0];
                assert_eq!(acting.entity().id(), 12, "the guest's own entity");
                game.as_guest(13, |game| {
                    assert_eq!(
                        uuids(level.online_players(game)),
                        Some(vec![13]),
                        "the outer guest is displaced and cannot resolve"
                    );
                })
                .unwrap();
            })
            .unwrap();

            let nether = game.level("minecraft:the_nether").unwrap();
            assert_eq!(uuids(nether.online_players(game)), Some(vec![]));
            let host = game.lan.as_mut().unwrap();
            let moved = host.hub.guests.connection_index(13).unwrap();
            host.hub.guests.update(moved, |guest| {
                guest.dimension = "minecraft:the_nether".to_owned();
            });
            assert_eq!(uuids(level.online_players(game)), Some(vec![99, 12]));
            assert_eq!(uuids(nether.online_players(game)), Some(vec![13]));

            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert_eq!(stale.online_players(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn level_players_leave_out_fake_players() {
        with_guests(|game| {
            let _serving = super::super::integrated::tests::install_serving(game, false);
            place_host(game);
            game.next_entity = 100;
            let level = game.level(game.dim.id()).unwrap();
            let fake = game.fake_player(level, 1234, "machine").unwrap();
            assert_eq!(uuids(level.online_players(game)), Some(vec![99, 12, 13]));
            let conn = game
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .find(|g| g.uuid == fake.uuid())
                .unwrap()
                .conn;
            game.as_guest(conn, |game| {
                assert_eq!(uuids(level.online_players(game)), Some(vec![12, 13]));
            })
            .unwrap();
        });
    }

    /// A guest watches a chunk once it is sent inside its view, and stops when
    /// it moves away or the chunk falls past its view distance.
    #[test]
    #[ignore = "daily"]
    fn chunk_watchers_follow_sent_chunks_inside_the_view() {
        with_guests(|game| {
            park_the_nether(game);
            let _serving = super::super::integrated::tests::install_serving(game, false);
            let level = game.level(game.dim.id()).unwrap();
            let chunk = ChunkRef::new(level, [0, 0]);
            assert_eq!(
                uuids(chunk.watchers(game)),
                Some(vec![]),
                "nothing sent yet"
            );
            let host = game.lan.as_mut().unwrap();
            for conn in [12, 13] {
                let i = host.hub.guests.connection_index(conn).unwrap();
                host.hub.guests.update(i, |g| {
                    g.center = Some((0, 0));
                    g.sent_chunks.insert((0, 0));
                });
            }
            assert_eq!(uuids(chunk.watchers(game)), Some(vec![12, 13]));
            game.as_guest(12, |game| {
                assert_eq!(
                    uuids(chunk.watchers(game)),
                    Some(vec![12, 13]),
                    "the acting guest is named once, by its own UUID"
                );
            })
            .unwrap();

            // View distance 8 (the guests' client setting): ChunkTrackingView.contains
            // keeps a chunk while max(|dx| - 2, 0)^2 + max(|dz| - 2, 0)^2 < 8^2.
            let host = game.lan.as_mut().unwrap();
            let far = host.hub.guests.connection_index(13).unwrap();
            assert_eq!(host.hub.guests[far].view_distance, 8);
            host.hub.guests.update(far, |g| g.center = Some((9, 0)));
            assert_eq!(
                uuids(chunk.watchers(game)),
                Some(vec![12, 13]),
                "7 past the buffer"
            );
            let host = game.lan.as_mut().unwrap();
            host.hub.guests.update(far, |g| g.center = Some((10, 0)));
            assert_eq!(
                uuids(chunk.watchers(game)),
                Some(vec![12]),
                "8 past the buffer"
            );

            let host = game.lan.as_mut().unwrap();
            let near = host.hub.guests.connection_index(12).unwrap();
            host.hub.guests.update(near, |g| {
                g.dimension = "minecraft:the_nether".to_owned();
            });
            assert_eq!(
                uuids(chunk.watchers(game)),
                Some(vec![]),
                "another level's chunk"
            );
            let nether = game.level("minecraft:the_nether").unwrap();
            assert_eq!(
                uuids(ChunkRef::new(nether, [0, 0]).watchers(game)),
                Some(vec![12])
            );

            let stale = ChunkRef::new(
                LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension()),
                [0, 0],
            );
            assert_eq!(stale.watchers(game), None);
            assert_eq!(
                uuids(ChunkRef::new(level, [i32::MIN, i32::MAX]).watchers(game)),
                Some(vec![])
            );
        });
    }
}
