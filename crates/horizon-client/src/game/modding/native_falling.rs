//! Falling blocks for native server mods (docs/modding/src/how-to/native-mods.md, "Falling
//! blocks"): a mod throws a block as a `FallingBlockEntity` that flies, lands
//! and places itself (or drops as an item) as sand does. It acts where the
//! world is simulated, in the level in place, which it names by its
//! `dimension` (`minecraft:overworld`) as `native_set_block` does.
use super::{FallingBlock, Game};
use horizon_core::{geometry::Vec3, position::BlockPos};
use horizon_world::registry::BlockStateId;

/// The most falling blocks a level holds before `spawn_falling_block`
/// refuses more: a mod's debris shouldn't take the server's tick with it.
pub(super) const NATIVE_FALLING_MAX: usize = 1024;
/// The largest speed a thrown block starts with, in blocks a tick on each
/// axis (`Entity.setDeltaMovement` takes any; a packet's speed is cut to 3.9).
const NATIVE_FALLING_MAX_SPEED: f64 = 3.9;

/// `velocity` with each axis cut to the largest start speed; none for a
/// value that isn't finite.
fn start_velocity(velocity: [f64; 3]) -> Option<Vec3> {
    velocity.iter().all(|v| v.is_finite()).then(|| {
        let [x, y, z] =
            velocity.map(|v| v.clamp(-NATIVE_FALLING_MAX_SPEED, NATIVE_FALLING_MAX_SPEED));
        Vec3::new(x, y, z)
    })
}

#[allow(dead_code)]
impl Game<'_> {
    /// Throws a falling block of `state` from `pos` (its bottom centre) at
    /// `velocity` (blocks a tick) in the level in place, `dimension`: it
    /// falls, lands and places itself as sand does, or drops as an item
    /// where it can't. The block at `pos` is left as it is: take it away first
    /// with `native_set_block` to make a block fall out of the world. False,
    /// with nothing thrown, on a client's copy of the world, for another
    /// dimension, an air or unknown state, a chunk that isn't loaded, a value
    /// that isn't finite, or with 1024 falling blocks in the level already.
    /// Each axis of `velocity` is cut to 3.9.
    pub(super) fn spawn_falling_block(
        &mut self,
        dimension: &str,
        state: BlockStateId,
        pos: [f64; 3],
        velocity: [f64; 3],
        falling: mod_api::Falling,
    ) -> bool {
        if self.entity_ids_exhausted() {
            return false;
        }
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
            || self.pack.assets.registry.state(state).is_none()
            || self.res.facts().is_air(state)
            || self.falling.len() >= NATIVE_FALLING_MAX
            || !pos.iter().all(|v| v.is_finite() && v.abs() <= 30_000_000.0)
        {
            return false;
        }
        let Some(velocity) = start_velocity(velocity) else {
            return false;
        };
        let cell = BlockPos::new(
            pos[0].floor() as i32,
            pos[1].floor() as i32,
            pos[2].floor() as i32,
        );
        if !self.world.chunks.contains_key(&(cell.x >> 4, cell.z >> 4)) {
            return false;
        }
        let mut damage =
            horizon_client::falling_block::Damage::falling(&self.res.facts().get(state).name);
        if falling.hurts {
            damage.hurt_entities = true;
            damage.per_distance = 2.0;
            damage.max = 40;
        }
        let at = Vec3::new(pos[0], pos[1], pos[2]);
        self.next_entity += 1;
        self.falling.push(FallingBlock {
            attachment_lifetime: Default::default(),
            rotation: [0.0, 0.0],
            join: Default::default(),
            id: self.next_entity,
            state,
            start_pos: cell,
            position: at,
            previous: at,
            velocity,
            age: 0,
            no_gravity: false,
            on_ground: false,
            drop_item: falling.drop_item,
            fall_distance: 0.0,
            stuck: Vec3::ZERO,
            damage,
            saved: None,
            portal: Default::default(),
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use horizon_mod_runtime::attachments::{
        AttachmentType, Attachments, HasAttachmentLifetime, Holder, index::AttachmentIndex,
    };
    use std::sync::{Arc, Mutex};

    static VALUE: AttachmentType<i32> = AttachmentType {
        namespace: "test",
        name: "falling_lifetime",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn falling_tick_preserves_its_owner_on_success_and_collision_error() {
        bound_falling_case(false, |game, _| {
            assert!(game.spawn_remote_projectile(
                "minecraft:falling_block",
                &horizon_protocol::packets::play::AddEntity {
                    id: 72,
                    uuid: 72,
                    kind: 0,
                    position: [4.5, 70.0, 0.5],
                    velocity: [0.0; 3],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: game.falling[0].state.0 as i32
                },
                (0.0, 0.0),
            ));
            let expected: Vec<_> = game.falling.iter().map(|f| (f.id, f.join)).collect();
            game.tick_one_falling(0, &[]).unwrap();
            assert_eq!(
                game.falling
                    .iter()
                    .map(|f| (f.id, f.join))
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(game.falling[0].age, 1);
            assert!(game.falling[0].position.y < 70.0);
            game.tick_one_falling(1, &[]).unwrap();
            assert_eq!(
                game.falling
                    .iter()
                    .map(|f| (f.id, f.join))
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!([game.falling[0].age, game.falling[1].age], [1, 1]);
            // A corrupt resident palette makes the real collision lookup fail.
            Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap()).set(0, 69, 0, 65_535);
            let position = game.falling[0].position;
            assert!(game.tick_one_falling(0, &[]).is_err());
            assert_eq!(
                game.falling
                    .iter()
                    .map(|f| (f.id, f.join))
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(game.falling[0].position, position);
            assert_eq!(game.falling[0].age, 1);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn falling_client_tick_commits_state_without_detaching_its_owner() {
        bound_falling_case(true, |game, _| {
            game.remote_projectile_data(
                71,
                &[(
                    8,
                    10,
                    horizon_protocol::entity_data::Value::Pos([8, 90, -4]),
                )],
            );
            assert_eq!(game.falling[0].start_pos, BlockPos::new(8, 90, -4));
            for age in 1..=3 {
                let previous = game.falling[0].position;
                game.tick_remote_projectiles(|_, at| at);
                assert_eq!(game.falling[0].age, age);
                assert_eq!(game.falling[0].previous, previous);
                assert!(game.falling[0].position.y < previous.y);
                assert!(game.falling[0].velocity.y < 0.0);
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn falling_parked_swap_preserves_state_and_owner_until_discard() {
        bound_falling_case(false, |game, index| {
            let home = game.dim;
            game.tick_one_falling(0, &[]).unwrap();
            let position = game.falling[0].position;
            game.enter_level(super::super::Dim::Nether).unwrap();
            assert!(game.falling.is_empty());
            assert_eq!(
                index
                    .lock()
                    .unwrap()
                    .snapshot(&("test".into(), "falling_lifetime".into()))
                    .len(),
                1
            );
            game.enter_level(home).unwrap();
            assert_eq!(game.falling.len(), 1);
            assert_eq!(game.falling[0].id, 71);
            assert_eq!(game.falling[0].age, 1);
            assert_eq!(game.falling[0].position, position);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn falling_lifecycle_commands_unload_park_and_remote() {
        use super::super::{
            commands::Target,
            entity::taken_pass_tests::{LifecycleFixture, RemoteLifecycle, SlotIdentities},
        };
        let mut identities = SlotIdentities::default();
        LifecycleFixture {
            seed: |game, entries| {
                game.falling.clear();
                let state = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:sand")
                    .unwrap();
                for &(id, at) in entries {
                    assert!(game.spawn_remote_projectile(
                        "minecraft:falling_block",
                        &horizon_protocol::packets::play::AddEntity {
                            id: id as i32,
                            uuid: u128::from(id),
                            kind: 0,
                            position: at,
                            velocity: [0.0; 3],
                            pitch: 0,
                            yaw: 0,
                            head_yaw: 0,
                            data: state.0 as i32
                        },
                        (0.0, 0.0),
                    ));
                }
            },
            ids: |game| game.falling.iter().map(|f| f.id).collect(),
            target: Target::Falling,
            unloads_with_chunk: true,
            remote: Some(RemoteLifecycle {
                prepare: |game| game.clear_remote_projectiles(),
                apply: |game, remove, _| game.remove_remote_projectile(remove),
            }),
        }
        .run_with(|game| {
            identities.observe(
                &mut game.falling,
                game.native_lifecycle.epoch().unwrap(),
                |f| f.id,
            );
        });
    }

    fn bound_falling_case(
        client: bool,
        exercise: impl FnOnce(&mut Game<'_>, &Arc<Mutex<AttachmentIndex<mod_api::EntityRef>>>),
    ) {
        let test = |game: &mut Game<'_>| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let state = game
                .pack
                .assets
                .registry
                .default_state("minecraft:sand")
                .unwrap();
            let at = Vec3::new(0.5, 70.0, 0.5);
            game.falling.push(FallingBlock {
                on_ground: false,
                attachment_lifetime: Default::default(),
                rotation: [0.0; 2],
                join: Default::default(),
                id: 71,
                state,
                start_pos: BlockPos::new(0, 70, 0),
                position: at,
                previous: at,
                velocity: Vec3::ZERO,
                age: 0,
                no_gravity: false,
                portal: Default::default(),
                drop_item: false,
                fall_distance: 0.0,
                stuck: Vec3::ZERO,
                damage: Default::default(),
                saved: None,
            });
            let id = game.falling[0].id;
            let holder = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), id);
            let index = Arc::new(Mutex::new(AttachmentIndex::default()));
            let mut values = Attachments::default();
            VALUE.set(&mut values, Holder::Entity(id), 10).unwrap();
            let original = game.falling.update(0, |f| {
                f.attachment_lifetime().bind(&mut values, &index, holder);
                f.attachment_lifetime().registration().unwrap()
            });
            let key = ("test".into(), "falling_lifetime".into());
            let before = index.lock().unwrap().snapshot(&key);
            assert_eq!(before.len(), 1);
            exercise(game, &index);
            let current = game
                .falling
                .update(0, |f| f.attachment_lifetime().registration().unwrap());
            assert!(original.same_registration(&current));
            assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
            assert_eq!(VALUE.get(&mut values, Holder::Entity(id)), Ok(10));
            // Exercise the production clear before any fixture cleanup can mask it.
            game.clear_remote_projectiles();
            assert!(game.falling.is_empty());
            assert!(index.lock().unwrap().snapshot(&key).is_empty());
            assert_eq!(before[0].entry.read(&VALUE), Ok(&10));
        };
        if client {
            super::super::native_entity_refs::tests::with_client_game(test);
        } else {
            super::super::native_entity_refs::tests::with_game(test);
        }
    }

    #[test]
    fn a_throw_is_cut_to_the_largest_speed_and_refuses_what_isnt_finite() {
        assert_eq!(
            start_velocity([0.5, 9.0, -9.0]),
            Some(Vec3::new(
                0.5,
                NATIVE_FALLING_MAX_SPEED,
                -NATIVE_FALLING_MAX_SPEED
            ))
        );
        assert_eq!(start_velocity([0.0, f64::NAN, 0.0]), None);
        assert_eq!(start_velocity([f64::INFINITY, 0.0, 0.0]), None);
    }
}
