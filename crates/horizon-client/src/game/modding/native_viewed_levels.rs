//! A client mod's reads of the levels the server opened for viewing (#3981, gap 10), as Immersive
//! Portals' `ClientWorldLoader.getWorld(dim)` hands out the far `ClientLevel`: on a client,
//! `host.level(dim)` also names each level the server keeps open on `horizon:levels`, and
//! `block_state`, `clip`, `collision_boxes` and `has_collisions` read its world (`viewed_worlds`),
//! as do the reads built on `level_block_state`: `block`, `block_property` and the both-sided
//! fluid reads (`fluid_shape`, `fluid_height`, `fluid_height_for_camera`,
//! `fluid_can_be_replaced_with`). A read never writes and never loads a column: a column the
//! world does not hold is None. The handle answers None from the server's close of the level,
//! and from a Login, a Respawn or a new configuration phase, which drop every viewed world. A
//! `LevelRef` names a level and the client level's epoch, not one open of the view: a close and
//! a later open within the same epoch make the old handle read again, while a Login, a
//! reconfiguration or a Respawn into another level starts a new epoch, so the old handle stays
//! stale and a mod asks `host.level` again (a same-level Respawn keeps the epoch). Writes,
//! server-only reads, the other reads and the entity reads (until step 9d) refuse it.
use super::Game;
use horizon_client::world::World;
use mod_api::{DimensionKey, LevelRef};

impl Game<'_> {
    /// On a client, the key of the level `dimension` names: the one it shows, or one the server
    /// opened for viewing while that view is open. None before login.
    pub(super) fn client_level_key(&self, dimension: &str) -> Option<DimensionKey> {
        let remote = self.remote.as_ref().filter(|r| r.logged_in)?;
        if remote.level == dimension {
            return Some(remote.level_key);
        }
        remote.viewed.key_of(dimension)
    }
    /// On a client, the world a handle names now: the player's own, or a viewed level's while
    /// the server keeps it open. None before login, for a stale handle and for any other level.
    pub(super) fn client_view_world(&self, level: LevelRef) -> Option<&World> {
        let remote = self.remote.as_ref().filter(|r| r.logged_in)?;
        if self.native_lifecycle.epoch() != Some(level.epoch()) {
            return None;
        }
        if level.dimension() == remote.level_key {
            return Some(&self.world);
        }
        remote.viewed.world_of(level.dimension())
    }
    /// On a client, the handle the player's own level answers to now (its key and the client
    /// level's epoch): what a world view naming a level is compared with, resolved with the
    /// same rules as `client_view_world`. None before login.
    fn client_own_level(&self) -> Option<LevelRef> {
        let remote = self.remote.as_ref().filter(|r| r.logged_in)?;
        Some(LevelRef::new(
            self.native_lifecycle.epoch()?,
            remote.level_key,
        ))
    }
    /// The frusta of the frame's camera `[fov_degrees, yaw, pitch]` and its world views, which are
    /// asked for here: each view's level handle is resolved once, first, against the player's own.
    pub(in crate::game) fn world_view_frustum(
        &mut self,
        camera: glam::DVec3,
        [fov_degrees, yaw, pitch]: [f32; 3],
        bob: glam::Mat4,
        depth_far: f32,
    ) -> Option<horizon_client::gfx::JoinedFrusta> {
        let own_level = self.client_own_level();
        let renderer = self.renderer.as_mut()?;
        renderer.set_world_view_own_level(own_level);
        Some(renderer.camera_frustum(camera, fov_degrees, yaw, pitch, bob, depth_far))
    }
    /// The world `block_state` and `clip` read: on a server an open level, in place or parked
    /// (`native_level_world`), on a client `client_view_world`.
    pub(super) fn native_level_view_world(&self, level: LevelRef) -> Option<&World> {
        if self.is_logical_server() {
            self.native_level_world(level)
        } else {
            self.client_view_world(level)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::game::{
        Game,
        api_values::TryIntoApi,
        viewed_worlds::tests::{client, close, column, layered, open, run, spawn},
    };
    use horizon_protocol::packets::play::{self, Clientbound as C};
    use horizon_world::registry::BlockStateId;
    use mod_api::{
        BlockFace, BlockState, BlockUpdateFlags, ClipBlock, ClipFluid, Host, LevelRef,
        LevelWriteError,
    };

    const NETHER: &str = "minecraft:the_nether";
    /// The distinguishing block of 9c's fixture: air in the viewed Nether, stone in the
    /// player's own Overworld.
    const POCKET: [i32; 3] = [3, 70, 5];

    fn set(pos: [i32; 3], state: u32) -> C {
        C::BlockUpdate(play::BlockUpdate {
            pos,
            state: state as i32,
        })
    }

    /// A client in the Overworld whose own column (0, 0) is air with stone at `POCKET`, and
    /// which views the Nether (level 1): its column (0, 0) stone below y 80 and air above, with
    /// air at `POCKET`.
    fn viewing(game: &mut Game) -> (LevelRef, LevelRef) {
        game.handle_packet(layered(0, 0, 0)).unwrap();
        game.handle_packet(set(POCKET, 1)).unwrap();
        game.handle_packet(open(1, NETHER)).unwrap();
        game.handle_packet(run(1, &[layered(0, 0, 9), set(POCKET, 0)]))
            .unwrap();
        let own = game.level("minecraft:overworld").unwrap();
        let viewed = game.level(NETHER).unwrap();
        (own, viewed)
    }

    /// The block at `pos` through the handle, as its state id: 0 air, 1 stone.
    fn state(host: &dyn Host, level: LevelRef, pos: [i32; 3]) -> Option<u32> {
        let read = level.block_state(host, pos)?;
        Some(
            [0, 1]
                .into_iter()
                .find(|&id| api(id) == read)
                .expect("air or stone"),
        )
    }

    fn api(id: u32) -> BlockState {
        BlockStateId(id).try_into_api().unwrap()
    }

    /// The viewed level's handle reads the viewed world's block where the player's own level
    /// holds another: air against stone at the same position.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_viewed_block_state_differs_from_the_players_own() {
        let mut resources = None;
        let mut game = client(&mut resources);
        let (own, viewed) = viewing(&mut game);
        assert_ne!(own, viewed);
        assert_eq!(state(&game, viewed, POCKET), Some(0));
        assert_eq!(state(&game, own, POCKET), Some(1));
        assert_eq!(state(&game, viewed, [3, 71, 5]), Some(1));
        assert_eq!(state(&game, viewed, [3, 80, 5]), Some(0));
        // A level the server did not open has no handle.
        assert_eq!(game.level("minecraft:the_end"), None);
    }

    /// A ray down onto the viewed level's stone hits its top, while the same ray in the
    /// player's own level, all air there, misses.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn clip_hits_the_viewed_block_while_the_players_own_ray_misses() {
        let mut resources = None;
        let mut game = client(&mut resources);
        let (own, viewed) = viewing(&mut game);
        let (from, to) = ([8.5, 90.5, 8.5], [8.5, 75.5, 8.5]);
        for mode in [ClipBlock::Collider, ClipBlock::Outline] {
            let hit = viewed.clip(&game, from, to, mode, ClipFluid::None).unwrap();
            assert!(!hit.miss);
            assert_eq!(hit.block, [8, 79, 8]);
            assert_eq!(hit.face, BlockFace::Up);
            assert_eq!(hit.location, [8.5, 80.0, 8.5]);
            assert!(!hit.inside && !hit.world_border);
            let miss = own.clip(&game, from, to, mode, ClipFluid::None).unwrap();
            assert!(miss.miss);
        }
    }

    /// The viewed level's collision boxes are its own: none in the pocket the player's own
    /// level fills with stone, the stone's box where the player's level is air.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn collision_boxes_read_the_viewed_world() {
        let mut resources = None;
        let mut game = client(&mut resources);
        let (own, viewed) = viewing(&mut game);
        let pocket = [3.0, 70.0, 5.0, 4.0, 71.0, 6.0];
        assert_eq!(viewed.collision_boxes(&game, pocket), Some(vec![]));
        assert_eq!(viewed.has_collisions(&game, pocket), Some(false));
        assert_eq!(own.collision_boxes(&game, pocket), Some(vec![pocket]));
        assert_eq!(own.has_collisions(&game, pocket), Some(true));
        let top = [8.0, 79.0, 8.0, 9.0, 80.0, 9.0];
        assert_eq!(viewed.collision_boxes(&game, top), Some(vec![top]));
        assert_eq!(viewed.has_collisions(&game, top), Some(true));
        assert_eq!(own.collision_boxes(&game, top), Some(vec![]));
        assert_eq!(own.has_collisions(&game, top), Some(false));
    }

    /// A column the viewed world does not hold answers None, and reading it loads nothing.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_column_the_viewed_world_does_not_hold_is_none() {
        let mut resources = None;
        let mut game = client(&mut resources);
        let (_, viewed) = viewing(&mut game);
        assert_eq!(state(&game, viewed, [20, 70, 5]), None);
        // Through the air above the stone: a miss inside column 0, None once into column 1.
        let ray = |to: f64| {
            viewed.clip(
                &game,
                [8.5, 85.5, 8.5],
                [to, 85.5, 8.5],
                ClipBlock::Collider,
                ClipFluid::None,
            )
        };
        assert!(ray(15.5).unwrap().miss);
        assert_eq!(ray(24.5), None);
        let inside = [8.0, 85.0, 8.0, 9.0, 86.0, 9.0];
        assert_eq!(viewed.collision_boxes(&game, inside), Some(vec![]));
        let across = [15.0, 85.0, 8.0, 17.0, 86.0, 9.0];
        assert_eq!(viewed.collision_boxes(&game, across), None);
        assert_eq!(viewed.has_collisions(&game, across), None);
        let world = &game.remote.as_ref().unwrap().viewed.get(1).unwrap().world;
        assert_eq!(world.chunks.len(), 1);
    }

    /// Writes, server-only reads and the entity reads refuse a viewed level, which keeps its
    /// block.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn writes_and_entity_reads_refuse_a_viewed_level() {
        let mut resources = None;
        let mut game = client(&mut resources);
        let (_, viewed) = viewing(&mut game);
        assert_eq!(
            viewed.set_block(&mut game, POCKET, api(1), BlockUpdateFlags::CLIENTS),
            Err(LevelWriteError::Unavailable)
        );
        assert_eq!(state(&game, viewed, POCKET), Some(0));
        assert_eq!(viewed.fluid_state(&game, POCKET), None);
        assert_eq!(viewed.entities_in(&game, [0.0; 3], [16.0; 3]), None);
    }

    /// The server's close, and a Respawn, end the handle: it and `host.level` answer None until
    /// the server opens the level again.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_close_and_a_respawn_end_the_viewed_handle() {
        let mut resources = None;
        let mut game = client(&mut resources);
        let (_, viewed) = viewing(&mut game);
        game.handle_packet(close(1)).unwrap();
        assert_eq!(game.level(NETHER), None);
        assert_eq!(state(&game, viewed, [3, 71, 5]), None);
        assert_eq!(
            viewed.collision_boxes(&game, [8.0, 79.0, 8.0, 9.0, 80.0, 9.0]),
            None
        );
        game.handle_packet(open(1, NETHER)).unwrap();
        game.handle_packet(run(1, &[column(0, 0)])).unwrap();
        assert_eq!(state(&game, viewed, [3, 71, 5]), Some(1));
        game.handle_packet(C::Respawn(play::Respawn {
            spawn: spawn("minecraft:overworld"),
            keep: 0,
        }))
        .unwrap();
        assert_eq!(game.level(NETHER), None);
        assert_eq!(state(&game, viewed, [3, 71, 5]), None);
        let ray = viewed.clip(
            &game,
            [8.5, 90.5, 8.5],
            [8.5, 75.5, 8.5],
            ClipBlock::Collider,
            ClipFluid::None,
        );
        assert_eq!(ray, None);
        // The same-level Respawn retains the epoch, but not the previous world's blocks.
        game.handle_packet(open(1, NETHER)).unwrap();
        game.handle_packet(run(1, &[layered(0, 0, 0)])).unwrap();
        assert_eq!(state(&game, viewed, [3, 71, 5]), Some(0));

        game.handle_packet(C::Respawn(play::Respawn {
            spawn: spawn("minecraft:the_end"),
            keep: 0,
        }))
        .unwrap();
        game.handle_packet(open(1, NETHER)).unwrap();
        game.handle_packet(run(1, &[column(0, 0)])).unwrap();
        assert_eq!(state(&game, viewed, [3, 71, 5]), None);
        let reopened = game.level(NETHER).unwrap();
        assert_eq!(state(&game, reopened, [3, 71, 5]), Some(1));
    }
}
