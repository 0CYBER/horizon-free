//! A server mod's chunk areas for one player (arismoko/horizon#3716, step 9b'): `PlayerRef::set_chunk_areas`
//! on the hub's viewed areas (`lan::viewed_levels`), which own the caps, the tickets and what is
//! sent. Each mod's areas are kept apart by the id of the mod whose call this is. Also a mod's
//! view centre for a player (`PlayerRef::set_view_center`, horizon-rust/horizon#19219), on
//! `Hub::set_view_center`.
use super::Game;
use horizon_client::lan::viewed_levels::areas as engine;
use mod_api::{
    ChunkArea, ChunkAreaMode, ChunkAreaRefusal, ChunkCap, ChunkPos, LevelRef, PlayerRef,
};

fn mode(mode: ChunkAreaMode) -> engine::ChunkAreaMode {
    match mode {
        ChunkAreaMode::Loaded => engine::ChunkAreaMode::Loaded,
        ChunkAreaMode::Ticking => engine::ChunkAreaMode::Ticking,
    }
}

fn cap(cap: engine::ChunkCap) -> ChunkCap {
    match cap {
        engine::ChunkCap::LoadedPerPlayer => ChunkCap::LoadedPerPlayer,
        engine::ChunkCap::TickingPerPlayer => ChunkCap::TickingPerPlayer,
        engine::ChunkCap::LoadedPerLevel => ChunkCap::LoadedPerLevel,
        engine::ChunkCap::TickingPerLevel => ChunkCap::TickingPerLevel,
    }
}

/// The hub's refusal as the API's, its level a handle of the open `level` names.
fn refusal(
    refused: engine::ChunkAreaRefusal,
    level: impl Fn(&str) -> LevelRef,
) -> ChunkAreaRefusal {
    use engine::ChunkAreaRefusal as R;
    match refused {
        R::NotOnline => ChunkAreaRefusal::NotOnline,
        R::RadiusTooLarge { radius, max } => ChunkAreaRefusal::RadiusTooLarge { radius, max },
        R::CenterOutOfRange {
            center: (x, z),
            max,
        } => ChunkAreaRefusal::CenterOutOfRange {
            center: ChunkPos::new(x, z),
            max,
        },
        R::TooManyAreas { asked, max } => ChunkAreaRefusal::TooManyAreas { asked, max },
        R::TooManyLevels { asked, cap } => ChunkAreaRefusal::TooManyLevels { asked, cap },
        R::TooManyChunks {
            cap: hit,
            level: name,
            asked,
            limit,
        } => ChunkAreaRefusal::TooManyChunks {
            cap: cap(hit),
            level: level(&name),
            asked,
            limit,
        },
    }
}

impl Game<'_> {
    /// For a server mod: its areas for `player`, in place of its last ones, through
    /// `Hub::set_viewed_areas`. Refused, changing nothing, for a player not online, a level
    /// handle of an earlier open, a call outside a mod's hook, and what the hub refuses.
    pub(super) fn player_set_chunk_areas(
        &mut self,
        player: PlayerRef,
        areas: &[ChunkArea],
    ) -> Result<(), ChunkAreaRefusal> {
        let owner =
            mod_api::content::crash::current_native_mod().ok_or(ChunkAreaRefusal::Unavailable)?;
        let epoch = self
            .native_lifecycle
            .epoch()
            .ok_or(ChunkAreaRefusal::NotOnline)?;
        if self.online_player_record(player).is_none()
            && self.native_acting_player() != Some(player)
        {
            return Err(ChunkAreaRefusal::NotOnline);
        }
        if areas.iter().any(|a| a.level.epoch() != epoch) {
            return Err(ChunkAreaRefusal::Unavailable);
        }
        if areas.len() > engine::MAX_AREAS {
            return Err(ChunkAreaRefusal::TooManyAreas {
                asked: areas.len() as u32,
                max: engine::MAX_AREAS as u32,
            });
        }
        let asked = areas
            .iter()
            .map(|a| engine::ChunkArea {
                level: a.level.dimension_id().to_owned(),
                center: (a.center.x, a.center.z),
                radius: a.radius,
                mode: mode(a.mode),
            })
            .collect();
        let host = self.lan.as_mut().ok_or(ChunkAreaRefusal::NotOnline)?;
        host.hub
            .set_viewed_areas(player.uuid(), owner, asked)
            .map_err(|refused| {
                refusal(refused, |id| {
                    LevelRef::new(epoch, mod_api::__private::dimension_key(id))
                })
            })
    }

    /// For a server mod: centres `player`'s chunk tracking on `center` in their own level, or on
    /// the player again with `None`, through `Hub::set_view_center`. False, changing nothing, for
    /// a player not online, a centre out of range and off a server.
    pub(super) fn player_set_view_center(
        &mut self,
        player: PlayerRef,
        center: Option<ChunkPos>,
    ) -> bool {
        if self.native_lifecycle.epoch().is_none() {
            return false;
        }
        if self.online_player_record(player).is_none()
            && self.native_acting_player() != Some(player)
        {
            return false;
        }
        let Some(host) = self.lan.as_mut() else {
            return false;
        };
        host.hub
            .set_view_center(player.uuid(), center.map(|c| (c.x, c.z)))
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use crate::game::{
        Dim, Game,
        view_pass::tests::{Sent, host_a_viewer, open_game, sent},
    };
    use horizon_client::lan::{Out, viewed_levels::areas as engine};
    use mod_api::{
        ChunkArea,
        ChunkAreaMode::*,
        ChunkAreaRefusal as R, ChunkCap, ChunkPos, LevelRef,
        content::crash::{ModCall, ModGuard},
    };
    use std::{
        sync::mpsc::Receiver,
        time::{Duration, Instant},
    };

    fn in_mod(id: &'static str) -> ModGuard {
        mod_platform::content::crash::enter_native_mod(ModCall {
            id,
            hook: "test",
            part: None,
        })
    }

    fn area(
        level: LevelRef,
        (x, z): (i32, i32),
        radius: u32,
        mode: mod_api::ChunkAreaMode,
    ) -> ChunkArea {
        ChunkArea::new(level, ChunkPos::new(x, z), radius, mode)
    }

    /// What `owner` holds for the viewer, as the hub keeps it.
    fn held(game: &Game<'_>, owner: &str) -> Vec<engine::ChunkArea> {
        game.lan.as_ref().unwrap().hub.viewed_areas(2, owner)
    }

    /// Ticks the server until `done` holds of what the viewer was sent of the Nether since the
    /// last call (at most 2 minutes); that, and how many hub ticks it took.
    fn tick_until(
        game: &mut Game<'_>,
        rx: &Receiver<Out>,
        done: impl Fn(&Sent) -> bool,
    ) -> (Sent, u64) {
        let start = game.lan.as_ref().unwrap().hub.ticks;
        let mut all = Sent::default();
        let deadline = Instant::now() + Duration::from_secs(120);
        while !done(&all) && Instant::now() < deadline {
            game.tick_server(false).unwrap();
            let s = sent(rx);
            all.opened |= s.opened;
            all.closed |= s.closed;
            all.chunks.extend(s.chunks);
            all.forgotten.extend(s.forgotten);
            std::thread::sleep(Duration::from_millis(20));
        }
        all.chunks.sort_unstable();
        all.forgotten.sort_unstable();
        (all, game.lan.as_ref().unwrap().hub.ticks - start)
    }

    /// A mod's `Loaded` Nether area reaches the viewer's client (`open`, then its chunks); every
    /// refusal comes back typed with the hub's values and keeps the areas; another mod's area
    /// stays through all of it; withdrawing the mod's areas forgets the chunks 52 ticks later,
    /// then closes the level.
    #[test]
    #[ignore = "daily"]
    fn a_mods_areas_reach_the_client_and_refusals_come_back_typed() {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let home =
            std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap()).join("chunk-areas");
        let _ = std::fs::remove_dir_all(&home);
        let mut resources = None;
        let mut game = open_game(&mut resources, &root, &home);
        let rx = host_a_viewer(&mut game);
        // The handles carry the open's epoch.
        game.native_lifecycle
            .begin_open(mod_api::Authority::IntegratedServer);
        let player = game.player(2).expect("the viewer is online");
        // Levels not loaded yet: an area loads them.
        let epoch = player.entity().epoch();
        let nether = LevelRef::new(epoch, Dim::Nether.key());
        let end = LevelRef::new(epoch, Dim::End.key());
        let portal = area(nether, (40, 40), 1, Loaded);
        let other = area(end, (0, 0), 0, Loaded);
        let portal_held = vec![engine::ChunkArea {
            level: "minecraft:the_nether".into(),
            center: (40, 40),
            radius: 1,
            mode: engine::ChunkAreaMode::Loaded,
        }];
        let other_held = vec![engine::ChunkArea {
            level: "minecraft:the_end".into(),
            center: (0, 0),
            radius: 0,
            mode: engine::ChunkAreaMode::Loaded,
        }];

        // Outside a mod's call, no mod owns the areas.
        assert_eq!(
            player.set_chunk_areas(&mut game, &[portal]),
            Err(R::Unavailable)
        );
        {
            let _call = in_mod("other");
            assert_eq!(player.set_chunk_areas(&mut game, &[other]), Ok(()));
        }
        {
            let _call = in_mod("portal");
            assert_eq!(player.set_chunk_areas(&mut game, &[portal]), Ok(()));
            assert_eq!(
                player.set_chunk_areas(&mut game, &[portal; 33]),
                Err(R::TooManyAreas { asked: 33, max: 32 })
            );
            assert_eq!(held(&game, "portal"), portal_held);

            assert_eq!(held(&game, "portal"), portal_held);

            let stranger = mod_api::__private::game::player_ref(player.entity(), 3);
            let elsewhere = LevelRef::new(
                nether.epoch(),
                mod_api::__private::dimension_key("test:elsewhere"),
            );
            let stale = LevelRef::new(
                mod_api::WorldEpoch::new(nether.epoch().get() + 1),
                nether.dimension(),
            );
            let many = vec![portal; 32];
            let cases: [(&str, _, &[ChunkArea], _); 10] = [
                ("a player not online", stranger, &[portal], R::NotOnline),
                (
                    "a stale level",
                    player,
                    &[area(stale, (40, 40), 1, Loaded)],
                    R::Unavailable,
                ),
                (
                    "radius 33",
                    player,
                    &[area(nether, (0, 0), 33, Loaded)],
                    R::RadiusTooLarge {
                        radius: 33,
                        max: 32,
                    },
                ),
                (
                    "a centre past the border's chunk",
                    player,
                    &[area(nether, (1_875_001, 0), 0, Loaded)],
                    R::CenterOutOfRange {
                        center: ChunkPos::new(1_875_001, 0),
                        max: 1_875_000,
                    },
                ),
                (
                    "33 areas, the other mod's included",
                    player,
                    &many,
                    R::TooManyAreas { asked: 33, max: 32 },
                ),
                (
                    "a third level for a client that views 2",
                    player,
                    &[portal, area(elsewhere, (0, 0), 0, Loaded)],
                    R::TooManyLevels { asked: 3, cap: 2 },
                ),
                (
                    "a Loaded radius 22: 47 × 47 with its ring",
                    player,
                    &[area(nether, (0, 0), 22, Loaded)],
                    R::TooManyChunks {
                        cap: ChunkCap::LoadedPerPlayer,
                        level: nether,
                        asked: 47 * 47,
                        limit: 2048,
                    },
                ),
                (
                    "a Ticking radius 11: 23 × 23",
                    player,
                    &[area(nether, (0, 0), 11, Ticking)],
                    R::TooManyChunks {
                        cap: ChunkCap::TickingPerPlayer,
                        level: nether,
                        asked: 23 * 23,
                        limit: 441,
                    },
                ),
                (
                    "49 Loaded chunks beside the other mod's 9, in an End capped at 30",
                    player,
                    &[area(end, (9, 9), 2, Loaded)],
                    R::TooManyChunks {
                        cap: ChunkCap::LoadedPerLevel,
                        level: end,
                        asked: 58,
                        limit: 30,
                    },
                ),
                (
                    "25 Ticking chunks in a Nether capped at 20",
                    player,
                    &[area(nether, (0, 0), 2, Ticking)],
                    R::TooManyChunks {
                        cap: ChunkCap::TickingPerLevel,
                        level: nether,
                        asked: 25,
                        limit: 20,
                    },
                ),
            ];
            let caps = game.lan.as_ref().unwrap().hub.viewed.caps;
            for (why, who, areas, refused) in cases {
                let hub = &mut game.lan.as_mut().unwrap().hub;
                hub.viewed.caps = caps;
                if why.contains("capped at 30") {
                    hub.viewed.caps.loaded_per_level = 30;
                }
                if why.contains("capped at 20") {
                    hub.viewed.caps.ticking_per_level = 20;
                }
                assert_eq!(who.set_chunk_areas(&mut game, areas), Err(refused), "{why}");
                assert_eq!(held(&game, "portal"), portal_held, "{why} keeps the areas");
                assert_eq!(
                    held(&game, "other"),
                    other_held,
                    "{why} keeps the other mod's"
                );
            }
            game.lan.as_mut().unwrap().hub.viewed.caps = caps;
        }

        let (seen, _) = tick_until(&mut game, &rx, |s| s.chunks.len() >= 9);
        assert!(seen.opened, "the Nether was opened on the client");
        let square: Vec<(i32, i32)> = (39..=41)
            .flat_map(|x| (39..=41).map(move |z| (x, z)))
            .collect();
        assert_eq!(seen.chunks, square, "the area's chunks, wrapped");

        {
            let _call = in_mod("portal");
            assert_eq!(player.set_chunk_areas(&mut game, &[]), Ok(()));
        }
        assert!(held(&game, "portal").is_empty());
        assert_eq!(
            held(&game, "other"),
            other_held,
            "the other mod's area stays"
        );
        // The first pass after the withdrawal starts the linger.
        game.tick_server(false).unwrap();
        assert!(sent(&rx).forgotten.is_empty());
        let (gone, ticks) = tick_until(&mut game, &rx, |s| !s.forgotten.is_empty());
        assert_eq!(ticks, 52, "forgotten 52 ticks after the linger started");
        assert_eq!(gone.forgotten, square);
        assert!(gone.closed, "closed once nothing of it is left");
    }
}
