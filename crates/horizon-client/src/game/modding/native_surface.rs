// game-module-visibility: pub(crate)
//! The `friction` Part method (`IBlockExtension.getFriction`). The client library asks through
//! `native_client_hooks`; only a state whose block has a subscribing part is converted.
use super::api_values::{TryIntoApi, block_state};
use super::{Game, boats::Boat};
use horizon_core::position::BlockPos;
use horizon_world::{native_load_context::LoopContext, registry::BlockStateId};

/// The game's side of `horizon_client::motion::native_friction`.
pub(crate) fn friction(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    pos: BlockPos,
    state: BlockStateId,
    friction: &mut f32,
) {
    if !block_state::block_listens(mod_api::HookId::Friction, state) {
        return;
    }
    let Ok(state) = state.try_into_api() else {
        return;
    };
    mod_api::dispatch_friction(
        &horizon_client_render::api_context::LoopReader(context).context(),
        entity,
        kind,
        pos,
        state,
        friction,
    );
}

impl Game<'_> {
    /// The friction of `state` at `pos` under the boat `b` (`AbstractBoat.getGroundFriction`,
    /// `Leashable.angularFriction`), from `own`, the block's value. A boat's type is a built
    /// name, so it is built only when the block has a part to answer.
    pub(super) fn boat_friction(
        &self,
        b: &Boat,
        pos: BlockPos,
        state: BlockStateId,
        own: f32,
    ) -> f32 {
        if !block_state::block_listens(mod_api::HookId::Friction, state) {
            return own;
        }
        let mut friction = own;
        let kind = b.entity_type();
        horizon_client::motion::native_friction(
            &self.loop_context,
            b.id,
            &kind,
            pos,
            state,
            &mut friction,
        );
        friction
    }
}

#[cfg(test)]
mod tests {
    use super::super::Game;
    use super::super::native_entity_refs::tests::with_game;
    use horizon_client::entities::{ArmorStand, StandMedium};
    use horizon_client::motion::{MotionWorld, PlayerMotion};
    use horizon_core::{geometry::Vec3, position::BlockPos};
    use horizon_world::living_air::{LivingAirWorld, travel_in_air};
    use std::sync::atomic::Ordering;

    fn item_and_orb(game: &Game<'_>) -> (f64, f64) {
        use horizon_client::entities::{ExperienceOrb, ItemEntity};
        let at = Vec3::new(5.5, 65.0, 5.5);
        let mut item = ItemEntity {
            attachment_lifetime: Default::default(),
            rotation: [0.0; 2],
            join: Default::default(),
            health: 5,
            fire_ticks: 0,
            id: 8,
            stack: horizon_client::items::ItemStack {
                item: "minecraft:stone".into(),
                count: 1,
                damage: 0,
                extra: None,
            },
            thrower: None,
            target: None,
            position: at,
            previous: at,
            velocity: Vec3::new(0.2, -0.04, 0.0),
            on_ground: true,
            fall_distance: 0.0,
            age: 0,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 0,
            needs_sync: false,
            stuck: Vec3::ZERO,
            was_touching_water: false,
            no_gravity: false,
            vertical_collision: false,
            horizontal_collision: false,
            random: horizon_core::random::EntityRandom::own(horizon_core::random::LegacyRandom::new(0)),
        };
        let step = item.tick_move(
            &game.world,
            game.res.facts(),
            game.collision,
            false,
            (0.0, 0.0),
            &[],
            None,
            None,
            &game.loop_context,
        );
        assert!(item.on_ground);
        assert!(item.tick_settle(
            &game.world,
            game.res.facts(),
            false,
            step,
            &game.loop_context
        ));
        let mut orb = ExperienceOrb::new(9, at, Vec3::new(0.2, 0.0, 0.0), 1);
        orb.on_ground = true;
        let (alive, _) = orb.tick_among(
            &game.world,
            game.res.facts(),
            game.collision,
            None,
            false,
            &mut || 0.0,
            None,
            &game.motion.fluids,
            &game.loop_context,
            &mut |_, _| {},
            &[],
        );
        assert!(alive);
        assert!(orb.on_ground);
        (item.velocity.x, orb.velocity.x)
    }

    #[test]
    #[ignore = "daily"]
    fn a_block_part_answers_the_friction_an_entity_reads() {
        // The fixture's parts: glass is 0.98 to a player only, white wool 0.98 to every entity,
        // red wool NaN, blue wool 3.0, green wool infinity and black wool -0.5.
        let calls = mod_platform::__private::part_tests::friction_game_thread();
        // The client library asks the mods through the executable's callbacks.
        crate::entry::install_client_hooks();
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            let floor = BlockPos::new(5, 64, 5);
            // The player's ground travel reads the block through its motion world.
            let player = |game: &mut Game<'_>| {
                let mut player = PlayerMotion::new(game.motion, Vec3::new(5.5, 65.0, 5.5)).unwrap();
                player.set_walking_speed_base(0.1).unwrap();
                player.ground.on_ground = true;
                player.velocity = Vec3::new(0.2, 0.0, 0.0);
                let mut world = MotionWorld {
                    entities: &[],
                    player: &mut player,
                    scene: &mut game.world,
                    assets: &game.pack.assets,
                    collision_binding: game.collision,
                    resources: game.motion,
                    health: &game.health_resources,
                    tag_items: Some(game.res.items()),
                    loop_context: &game.loop_context,
                    damage_hooks: &game.damage_hooks,
                    block_registries: Some(&game.ui.registries),
                };
                let coefficient = LivingAirWorld::block_friction(&mut world, floor).unwrap();
                calls.store(0, Ordering::SeqCst);
                travel_in_air(
                    &mut world,
                    &mut horizon_world::move_gate::MoveGate::new(),
                    Vec3::new(1.0, 0.0, 0.0),
                )
                .unwrap();
                let reads = calls.load(Ordering::SeqCst);
                (
                    coefficient,
                    world.player.position.x - 5.5,
                    world.player.velocity.x,
                    reads,
                )
            };
            // An armor stand moving 0.2 along x on the ground, after one travel: 0.2 × friction × 0.91.
            let stand = |game: &Game<'_>| {
                let mut stand = ArmorStand::new(7, Vec3::new(5.5, 65.0, 5.5), 0.0);
                stand.on_ground = true;
                stand.velocity = Vec3::new(0.2, 0.0, 0.0);
                let mut scene = horizon_client::collisions::SceneCollisions {
                    world: &game.world,
                    binding: game.collision,
                    subject: None,
                };
                let step = stand
                    .tick_move(
                        &mut scene,
                        game.res.facts(),
                        &game.motion.climbing,
                        false,
                        None,
                        &[],
                        &game.loop_context,
                    )
                    .unwrap();
                stand.tick_settle(StandMedium::Air, step);
                stand.velocity.x
            };
            let registry = game.pack.assets.registry;
            for (block, player_friction, stand_x, item_friction) in [
                // No part: the block's own value, for every entity.
                ("minecraft:stone", 0.6, 0.10920001, 0.6),
                ("minecraft:ice", 0.98, 0.17836001, 0.98),
                // A part answering for the player only.
                ("minecraft:glass", 0.98, 0.10920001, 0.6),
                // A part answering for every entity.
                ("minecraft:white_wool", 0.98, 0.17836001, 0.98),
                // Answers out of range are dropped for the block's own.
                ("minecraft:red_wool", 0.6, 0.10920001, 0.6),
                ("minecraft:blue_wool", 0.6, 0.10920001, 0.6),
                ("minecraft:green_wool", 0.6, 0.10920001, 0.6),
                ("minecraft:black_wool", 0.6, 0.10920001, 0.6),
            ] {
                let state = registry.default_state(block).unwrap();
                game.world.set(floor.x, floor.y, floor.z, state).unwrap();
                let (coefficient, distance, velocity, reads) = player(game);
                assert_eq!(coefficient, player_friction, "{block}");
                let (speed, dragged) = if player_friction > 0.9 {
                    (0.2229493, 0.1988262)
                } else {
                    (0.3, 0.1638)
                };
                assert!((distance - speed).abs() < 1.0e-6, "{block}: {distance}");
                assert!((velocity - dragged).abs() < 1.0e-6, "{block}: {velocity}");
                assert_eq!(
                    reads,
                    usize::from(!matches!(block, "minecraft:stone" | "minecraft:ice")),
                    "one friction answer per travel tick: {block}"
                );
                assert!((stand(game) - stand_x).abs() < 1.0e-7, "{block}");
                let (item_x, orb_x) = item_and_orb(game);
                let expected = if item_friction > 0.9 {
                    0.19208001
                } else {
                    0.11760001
                };
                assert!((item_x - expected).abs() < 1.0e-7, "item {block}: {item_x}");
                assert!((orb_x - expected).abs() < 1.0e-7, "orb {block}: {orb_x}");
            }
        });
    }
    #[test]
    #[ignore = "daily"]
    fn mob_travel_and_both_leash_sites_use_the_parts_answer_once() {
        use super::super::boats::Boat;
        use horizon_client::mobs::{Mob, leash};
        let calls = mod_platform::__private::part_tests::friction_game_thread();
        crate::entry::install_client_hooks();
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            let at = Vec3::new(5.5, 65.0, 5.5);
            let horse = game.res.mob_types().get("minecraft:horse").unwrap().clone();
            let ghast = game.res.mob_types().get("minecraft:ghast").unwrap().clone();
            game.mobs.push(Mob::new(99, ghast, at, 0.0));
            let mut damage = horizon_client::health::TestLevel::new(0);
            for (block, drag, spin, listens) in [
                ("minecraft:stone", 0.10920001, 1.0920001, 0),
                ("minecraft:white_wool", 0.17836001, 1.7836001, 1),
            ] {
                let state = game.pack.assets.registry.default_state(block).unwrap();
                game.world.set(5, 64, 5, state).unwrap();
                let snapshot: Vec<_> = game
                    .mobs
                    .iter()
                    .map(|m| m.seen(None, game.items()))
                    .collect();
                {
                    let world = game.mob_world_base(&snapshot);
                    let mut mob = Mob::new(10, horse.clone(), at, 0.0);
                    mob.on_ground = true;
                    mob.velocity = Vec3::new(0.2, 0.0, 0.0);
                    calls.store(0, Ordering::SeqCst);
                    mob.travel(&world, &mut damage.level(), Vec3::ZERO, &mut Vec::new());
                    assert!(
                        (mob.velocity.x - drag).abs() < 1.0e-7,
                        "mob {block}: {}",
                        mob.velocity.x
                    );
                    assert_eq!(
                        calls.load(Ordering::SeqCst),
                        listens,
                        "one answer per mob travel tick"
                    );
                    mob.position = at;
                    mob.on_ground = true;
                    mob.velocity = Vec3::ZERO;
                    mob.extra.leash = Some(leash::Holder::Mob(99));
                    mob.extra.leash_spin = 2.0;
                    calls.store(0, Ordering::SeqCst);
                    leash::tick(&mut mob, &world, &mut Vec::new());
                    assert!(
                        (mob.extra.leash_spin - spin).abs() < 1.0e-6,
                        "mob lead {block}: {}",
                        mob.extra.leash_spin
                    );
                    assert_eq!(calls.load(Ordering::SeqCst), listens);
                }
                let mut boat = Boat::new(11, "oak".into(), at, 0.0);
                boat.on_ground = true;
                boat.leash_spin = 2.0;
                boat.leash = Some(leash::Holder::Mob(99));
                calls.store(0, Ordering::SeqCst);
                game.tick_boat_leash(&mut boat);
                assert!(
                    (boat.leash_spin - spin).abs() < 1.0e-6,
                    "boat lead {block}: {}",
                    boat.leash_spin
                );
                assert_eq!(calls.load(Ordering::SeqCst), listens);
            }
        });
    }
}
