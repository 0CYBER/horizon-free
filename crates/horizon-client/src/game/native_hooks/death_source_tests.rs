//! Prepared-resource regressions through real projectile collision, mob AI and server death.
#![cfg(test)]
use crate::game::Game;
use crate::game::api_values::TryIntoApi;
use crate::game::native_entity_refs::tests::{bind_local_player, with_game};
use horizon_client::{
    chunk::Chunk,
    items::ItemStack,
    mobs::{Mob, Who},
    motion::PlayerMotion,
    projectiles::{Projectile, ProjectileKind},
};
use horizon_core::{geometry::Vec3, nbt::Tag, position::ChunkPos};
use mod_api::{
    ArmorHurtEvent, Flow, Hooks, HurtSource, LevelRandomHost, LivingDeathEvent,
    LivingIncomingDamageEvent, ModEntry, ServerHost,
};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
struct Hit {
    entity: u32,
    kind: String,
    damage_type: String,
    causing: i32,
    direct: i32,
    position: Option<[f64; 3]>,
    weapon: Option<mod_api::ItemStack>,
}
impl Hit {
    fn new(
        entity: u32,
        kind: &str,
        source: &HurtSource<'_>,
        weapon: Option<&mod_api::ItemStack>,
    ) -> Self {
        Self {
            entity,
            kind: kind.into(),
            damage_type: source.damage_type.into(),
            causing: source.causing,
            direct: source.direct,
            position: source.position,
            weapon: weapon.cloned(),
        }
    }
}

#[derive(Default)]
struct Seen {
    incoming: Vec<Hit>,
    deaths: Vec<Hit>,
    armor: Vec<Hit>,
    kinds: Vec<(String, String)>,
}
struct Probe {
    seen: Arc<Mutex<Seen>>,
    cancel: bool,
    heal: bool,
}
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn living_incoming_damage(
        &self,
        _: &mut dyn LevelRandomHost,
        event: &mut LivingIncomingDamageEvent<'_>,
    ) -> Flow {
        self.seen.lock().unwrap().incoming.push(Hit::new(
            event.entity as u32,
            event.kind,
            event.source,
            event.weapon,
        ));
        Flow::Continue
    }
    fn living_death(&self, host: &mut dyn ServerHost, event: &mut LivingDeathEvent<'_>) -> Flow {
        self.seen.lock().unwrap().deaths.push(Hit::new(
            event.entity,
            event.kind,
            event.source,
            event.weapon,
        ));
        let entity = host.entity(event.entity).expect("dying mob still resolves");
        let kind = entity.kind(host).expect("dying entity has a type").id(host);
        self.seen
            .lock()
            .unwrap()
            .kinds
            .push((event.kind.to_owned(), kind));
        if self.heal {
            let entity = host
                .entity(event.entity)
                .expect("dying entity still resolves");
            let living = host.living(entity).expect("dying body is living");
            assert!(living.set_health(host, 5.0));
            assert_eq!(living.health(host), Some(5.0));
        }
        if self.cancel && self.seen.lock().unwrap().deaths.len() == 1 {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
    fn armor_hurt(&self, _: &mut dyn ServerHost, event: &mut ArmorHurtEvent<'_>) -> Flow {
        assert!(!event.entries.is_empty());
        assert!(event.entries.iter().any(|entry| entry.damage > 0.0));
        self.seen.lock().unwrap().armor.push(Hit::new(
            event.entity,
            event.kind,
            event.source,
            None,
        ));
        Flow::Continue
    }
}

fn observed(cancel: bool, heal: bool, check: impl FnOnce(&mut Game<'_>, &Arc<Mutex<Seen>>)) {
    crate::entry::install_client_hooks();
    let seen = Arc::new(Mutex::new(Seen::default()));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "death-source",
            Box::new(Probe {
                seen: seen.clone(),
                cancel,
                heal,
            }),
        )]
        .into(),
        || {
            with_game(|game| {
                game.build_registries();
                game.load_server_loot();
                game.set_allow_commands(true);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.block_ticks
                    .add_empty_chunk(ChunkPos::new(0, 0))
                    .unwrap();
                assert!(game.run_command("fill 0 79 0 15 79 15 stone"));
                check(game, &seen);
            })
        },
    );
}

fn player(game: &mut Game<'_>) {
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 80.0, 4.5)).unwrap());
    bind_local_player(game, 11, 123);
    game.set_mode_id(0);
    game.next_entity = 100;
}

fn mob(game: &mut Game<'_>, id: u32, name: &str, position: Vec3) {
    let kind = game.res.mob_types().get(name).unwrap().clone();
    let mut mob = Mob::new(id, kind, position, 0.0);
    mob.persistent = true;
    mob.no_ai = true;
    mob.on_ground = true;
    game.mobs.push(mob);
}

fn bow() -> ItemStack {
    let mut bow = ItemStack::new("minecraft:bow", 1);
    bow.damage = 7;
    bow.data_mut().set_other(
        "minecraft:custom_data",
        Some(Tag::Compound(
            [("death_source_marker".into(), Tag::Int(3414))].into(),
        )),
    );
    bow.data_mut().set_other(
        "minecraft:custom_name",
        Some(Tag::String("Killing bow".into())),
    );
    bow
}

/// Reuse BowItem.releaseUsing, then make its collision deterministic (not its random spread).
fn player_arrow(game: &mut Game<'_>, weapon: &ItemStack) -> u32 {
    game.inventory[game.selected] = Some(weapon.clone());
    game.inventory[9] = Some(ItemStack::new("minecraft:arrow", 8));
    game.release_bow(weapon, 20);
    assert_eq!(
        game.creatures.projectiles.len(),
        1,
        "actual bow release spawned an arrow"
    );
    game.creatures.projectiles.update(0, |arrow| {
        arrow.position = Vec3::new(4.5, 80.5, 4.5);
        arrow.previous = arrow.position;
        arrow.velocity = Vec3::new(4.0, 0.0, 0.0);
        arrow.id
    })
}

fn finish_projectile_death(game: &mut Game<'_>, id: u32) {
    game.tick_projectiles().unwrap();
    assert!(game.mobs.iter().find(|mob| mob.id == id).unwrap().health <= 0.0);
    game.tick_creature_entities().unwrap();
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn player_bow_death_retains_the_incoming_source_and_full_weapon() {
    crate::game::in_own_process(
        "game::native_hooks::death_source_tests::player_bow_death_retains_the_incoming_source_and_full_weapon",
        || {
            observed(false, false, |game, seen| {
                player(game);
                mob(game, 41, "minecraft:zombie", Vec3::new(6.0, 80.0, 4.5));
                game.mobs[0].health = 1.0;
                let weapon = bow();
                let expected = weapon.try_into_api().unwrap();
                let arrow = player_arrow(game, &weapon);
                // Death must not reconstruct the bow from the shooter's current hand.
                game.inventory[game.selected] = Some(ItemStack::new("minecraft:stick", 1));
                finish_projectile_death(game, 41);
                let seen = seen.lock().unwrap();
                assert_eq!(seen.deaths.len(), 1);
                assert_eq!(seen.incoming.len(), 1);
                assert_eq!(seen.deaths[0], seen.incoming[0]);
                let hit = &seen.deaths[0];
                assert_eq!((hit.entity, hit.kind.as_str()), (41, "minecraft:zombie"));
                assert_eq!(
                    (hit.damage_type.as_str(), hit.causing, hit.direct),
                    ("minecraft:arrow", 11, arrow as i32)
                );
                assert!(
                    hit.position.is_some(),
                    "the arrow's original source position survives"
                );
                assert_eq!(
                    hit.weapon.as_ref(),
                    Some(&expected),
                    "all bow components and damage survive"
                );
                assert!(expected.components().is_some());
            })
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn skeleton_arrow_death_credits_the_skeleton_not_the_player() {
    crate::game::in_own_process(
        "game::native_hooks::death_source_tests::skeleton_arrow_death_credits_the_skeleton_not_the_player",
        || {
            observed(false, false, |game, seen| {
                player(game);
                game.player.as_mut().unwrap().position = Vec3::new(12.0, 80.0, 12.0);
                mob(game, 42, "minecraft:skeleton", Vec3::new(2.5, 80.0, 4.5));
                mob(game, 43, "minecraft:zombie", Vec3::new(6.0, 80.0, 4.5));
                game.mobs[1].health = 1.0;
                let weapon = bow();
                game.mobs[0].extra.set_main_hand_stack(Some(weapon.clone()));
                let mut shot = Projectile::new(
                    ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: None,
                    },
                    Vec3::new(4.5, 80.5, 4.5),
                    Some(Who::Mob(42)),
                );
                shot.left_owner = true;
                shot.velocity = Vec3::new(4.0, 0.0, 0.0);
                shot.arrow.as_mut().unwrap().fired_from_weapon = Some(weapon.clone());
                game.add_projectile(shot);
                let arrow = game.creatures.projectiles[0].id;
                finish_projectile_death(game, 43);
                let seen = seen.lock().unwrap();
                assert_eq!(seen.deaths.len(), 1);
                assert_eq!(seen.deaths[0], seen.incoming[0]);
                assert_eq!(
                    (seen.deaths[0].causing, seen.deaths[0].direct),
                    (42, arrow as i32)
                );
                assert_eq!(seen.deaths[0].weapon, Some(weapon.try_into_api().unwrap()));
            })
        },
    );
}

fn cancelled_mob(heal: bool) {
    observed(true, heal, |game, seen| {
        player(game);
        // Sheep's guaranteed mutton makes the absence of loot non-probabilistic.
        mob(game, 44, "minecraft:sheep", Vec3::new(6.0, 80.0, 4.5));
        game.mobs[0].health = 1.0;
        player_arrow(game, &bow());
        game.tick_projectiles().unwrap();
        game.tick_creature_entities().unwrap();
        assert_eq!(seen.lock().unwrap().deaths.len(), 1);
        assert_eq!(game.mobs[0].health, if heal { 5.0 } else { 0.0 });
        assert!(game.dropped.is_empty());
        let elapsed = game.mobs[0].death_time;
        if !heal {
            assert!(elapsed < 20);
            for _ in elapsed..19 {
                game.tick_creature_entities().unwrap();
                assert!(game.mobs.iter().any(|mob| mob.id == 44 && !mob.removed));
            }
            game.tick_creature_entities().unwrap();
            assert!(
                !game.mobs.iter().any(|mob| mob.id == 44 && !mob.removed),
                "body removed at tick 20"
            );
        } else {
            for _ in 0..25 {
                game.tick_creature_entities().unwrap();
            }
            let mob = game.mobs.iter().find(|mob| mob.id == 44).unwrap();
            assert_eq!((mob.health, mob.death_time, mob.removed), (5.0, 0, false));
        }
        for _ in 0..5 {
            game.tick_creature_entities().unwrap();
        }
        assert_eq!(seen.lock().unwrap().deaths.len(), 1, "no death redispatch");
        assert!(
            game.dropped.is_empty(),
            "a cancelled death never rolls loot"
        );
        assert!(game.orbs.is_empty());
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn cancelled_mob_death_preserves_listener_health_five_without_loot() {
    crate::game::in_own_process(
        "game::native_hooks::death_source_tests::cancelled_mob_death_preserves_listener_health_five_without_loot",
        || cancelled_mob(true),
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn cancelled_unhealed_mob_is_removed_at_twenty_ticks_without_loot_or_redispatch() {
    crate::game::in_own_process(
        "game::native_hooks::death_source_tests::cancelled_unhealed_mob_is_removed_at_twenty_ticks_without_loot_or_redispatch",
        || cancelled_mob(false),
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn zombie_ai_melee_armor_hurt_retains_the_real_source() {
    crate::game::in_own_process(
        "game::native_hooks::death_source_tests::zombie_ai_melee_armor_hurt_retains_the_real_source",
        || {
            observed(false, false, |game, seen| {
                player(game);
                Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap()).lit = true;
                game.world.day_time = 18_000; // An open-air zombie must not burn during the melee scene.
                assert!(game.run_command("difficulty normal"));
                assert!(game.run_command("forceload add 0 0"));
                game.update_chunk_tickets();
                assert!(game.tickets.levels().entity_ticking_at(5, 4));
                assert!(game.player.as_ref().unwrap().client_loaded);
                game.armor[1] = Some(ItemStack::new("minecraft:iron_chestplate", 1));
                game.apply_equipment();
                game.refresh_armor_protection();
                mob(game, 45, "minecraft:zombie", Vec3::new(5.0, 80.0, 4.5));
                game.mobs[0].no_ai = false;
                game.mobs[0].ai.speed = 0.2;
                game.mobs[0].ai.target = Some(Who::Player);
                for _ in 0..40 {
                    // MeleeAttackGoal.canUse admits a new attempt once game time reaches 20.
                    game.world.game_time += 1;
                    game.tick_creature_entities().unwrap();
                    if !seen.lock().unwrap().armor.is_empty() {
                        break;
                    }
                }
                let seen = seen.lock().unwrap();
                assert!(
                    !seen.armor.is_empty(),
                    "real zombie AI must land its melee hit"
                );
                let hit = &seen.armor[0];
                assert_eq!((hit.entity, hit.kind.as_str()), (11, "minecraft:player"));
                assert_eq!(
                    (hit.damage_type.as_str(), hit.causing, hit.direct),
                    ("minecraft:mob_attack", 45, 45)
                );
                let incoming = seen.incoming.iter().find(|hit| hit.entity == 11).unwrap();
                assert_eq!(
                    (hit.causing, hit.direct, hit.position),
                    (incoming.causing, incoming.direct, incoming.position)
                );
                assert!(game.armor[1].as_ref().unwrap().damage > 0);
            })
        },
    );
}

fn cancelled_player(heal: bool) {
    observed(true, heal, |game, seen| {
        crate::game::native_player_death::tests::add_guest(game);
        game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
            guest.health = 0.0;
            guest.loaded = true;
            guest.load_timeout = 0;
            guest.inventory[0] = Some(ItemStack::new("minecraft:diamond", 3));
            guest.vehicle = Some(99);
        });
        game.guest_die(0, 1);
        assert_eq!(seen.lock().unwrap().deaths.len(), 1);
        assert_eq!(
            game.lan.as_ref().unwrap().hub.guests[0].health,
            if heal { 5.0 } else { 0.0 }
        );
        let entity = game.lan.as_ref().unwrap().hub.guests[0].entity_id as u32;
        for tick in 1..=20 {
            // The ordinary server loop asks each tick; cancellation must not redispatch.
            game.guest_die(0, 1);
            if !heal {
                game.tick_guests();
            }
            if heal || tick < 20 {
                let host: &dyn mod_api::Host = game;
                assert!(
                    host.entity(entity).is_some(),
                    "body remains before tickDeath removal"
                );
            }
        }
        let guest = &game.lan.as_ref().unwrap().hub.guests[0];
        assert_eq!(guest.health, if heal { 5.0 } else { 0.0 });
        assert_eq!(
            guest.inventory[0],
            Some(ItemStack::new("minecraft:diamond", 3))
        );
        if !heal {
            assert_eq!(guest.death_time, 20);
            assert!(
                guest.vehicle.is_none(),
                "tickDeath removed the unhealed body from its vehicle"
            );
            let host: &dyn mod_api::Host = game;
            assert!(
                host.entity(guest.entity_id as u32).is_none(),
                "tickDeath removed the body from entity lookup"
            );
        } else {
            assert!(!guest.dead);
            assert_eq!(guest.death_time, 0);
        }
        assert_eq!(seen.lock().unwrap().deaths.len(), 1);
        assert!(game.dropped.is_empty());
        assert!(game.orbs.is_empty());
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and loopback sockets"]
fn cancelled_player_death_preserves_listener_health_five_and_inventory() {
    crate::game::in_own_process(
        "game::native_hooks::death_source_tests::cancelled_player_death_preserves_listener_health_five_and_inventory",
        || cancelled_player(true),
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and loopback sockets"]
fn cancelled_unhealed_player_ticks_away_without_drops_or_redispatch() {
    crate::game::in_own_process(
        "game::native_hooks::death_source_tests::cancelled_unhealed_player_ticks_away_without_drops_or_redispatch",
        || cancelled_player(false),
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and loopback sockets"]
fn guest_player_bow_death_keeps_the_connected_shooter_without_a_local_player() {
    crate::game::in_own_process(
        "game::native_hooks::death_source_tests::guest_player_bow_death_keeps_the_connected_shooter_without_a_local_player",
        || {
            observed(false, false, |game, seen| {
                game.player = None;
                crate::game::native_player_death::tests::add_guest(game);
                game.next_entity = 100;
                game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                    guest.position = [4.5, 80.0, 4.5];
                    guest.loaded = true;
                    guest.load_timeout = 0;
                });
                mob(game, 41, "minecraft:zombie", Vec3::new(6.0, 80.0, 4.5));
                game.mobs[0].health = 1.0;
                let weapon = bow();
                let expected = weapon.try_into_api().unwrap();
                let arrow = game
                    .as_guest(1, |game| player_arrow(game, &weapon))
                    .unwrap();
                assert!(
                    game.player.is_none(),
                    "the impact runs in the dedicated-server context"
                );
                game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                    guest.inventory[guest.selected] = Some(ItemStack::new("minecraft:stick", 1));
                });
                finish_projectile_death(game, 41);
                let seen = seen.lock().unwrap();
                assert_eq!(seen.deaths.len(), 1);
                assert_eq!(seen.incoming.len(), 1);
                assert_eq!(seen.deaths[0], seen.incoming[0]);
                let hit = &seen.deaths[0];
                assert_eq!((hit.causing, hit.direct), (20, arrow as i32));
                assert_eq!(hit.weapon.as_ref(), Some(&expected));
                assert!(hit.position.is_some());
            })
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn cancelled_body_reconstruction_keeps_the_hit_and_original_twenty_tick_deadline() {
    crate::game::in_own_process(
        "game::native_hooks::death_source_tests::cancelled_body_reconstruction_keeps_the_hit_and_original_twenty_tick_deadline",
        || {
            observed(true, false, |game, seen| {
                player(game);
                mob(game, 44, "minecraft:sheep", Vec3::new(6.0, 80.0, 4.5));
                game.mobs[0].health = 1.0;
                player_arrow(game, &bow());
                game.tick_projectiles().unwrap();
                game.tick_creature_entities().unwrap();
                while game.mob_object(44).unwrap().death_time < 19 {
                    game.tick_creature_entities().unwrap();
                }
                let original = game.mob_object(44).unwrap().accepted_hit.clone().unwrap();
                // This is the exact NBT + non-Clone runtime-token reconstruction
                // used by teleport_vehicle_across, not a copied in-memory Mob.
                let tag = game.vehicle_tree_nbt(44).unwrap();
                let state = game.capture_vehicle_tree_state(44).unwrap();
                assert!(game.mob_object(44).unwrap().accepted_hit.is_none());
                game.mobs.retain(|body| body.id != 44);
                game.detach_entity(44);
                game.native_leaves_flush();
                let restored = game.restore_vehicle_tree_state(tag, state).unwrap().root;
                game.native_joins_flush(false, 0);
                assert_ne!(
                    restored, 44,
                    "the destination reconstructs under a fresh ID"
                );
                let body = game.mob_object(restored).unwrap();
                assert_eq!(
                    body.death_time, 19,
                    "DeathTime survives the actual NBT codec"
                );
                assert_eq!(body.last_damage, original.committed);
                assert!(Arc::ptr_eq(body.accepted_hit.as_ref().unwrap(), &original));
                assert_eq!(seen.lock().unwrap().deaths.len(), 1);
                game.tick_creature_entities().unwrap();
                assert!(
                    game.mob_object(restored).is_none(),
                    "the next tick is the original twentieth tick"
                );
                assert_eq!(
                    seen.lock().unwrap().deaths.len(),
                    1,
                    "a transfer does not retry a cancelled die"
                );
                assert!(game.dropped.is_empty());
                assert!(game.orbs.is_empty());
            })
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn later_cancelled_mob_revival_is_transactional_and_a_second_kill_drops_loot() {
    crate::game::in_own_process(
        "game::native_hooks::death_source_tests::later_cancelled_mob_revival_is_transactional_and_a_second_kill_drops_loot",
        || {
            observed(true, false, |game, seen| {
                player(game);
                mob(game, 44, "minecraft:sheep", Vec3::new(6.0, 80.0, 4.5));
                game.mobs[0].health = 1.0;
                player_arrow(game, &bow());
                game.tick_projectiles().unwrap();
                game.tick_creature_entities().unwrap();
                let before = game.mob_object(44).unwrap().death_time;
                let living = {
                    let host: &mut dyn ServerHost = game;
                    host.living(host.entity(44).unwrap()).unwrap()
                };
                let abort = game.transaction_open();
                assert!(living.set_health(game, 5.0));
                assert_eq!(game.mob_object(44).unwrap().death_time, 0);
                game.transaction_close(&abort, false);
                let body = game.mob_object(44).unwrap();
                assert_eq!(
                    (body.health, body.death_time, body.death_canceled),
                    (0.0, before, true)
                );
                game.tick_creature_entities().unwrap();
                assert_eq!(
                    seen.lock().unwrap().deaths.len(),
                    1,
                    "abort restored the processed attempt"
                );
                assert!(game.dropped.is_empty());
                assert!(living.set_health(game, 5.0));
                let body = game.mob_object(44).unwrap();
                assert_eq!(
                    (body.health, body.death_time, body.death_canceled),
                    (5.0, 0, false)
                );
                assert!(game.run_command("kill @e[type=minecraft:sheep]"));
                assert_eq!(
                    seen.lock().unwrap().deaths.len(),
                    2,
                    "the genuine second death reaches its listener"
                );
                assert!(
                    game.dropped
                        .iter()
                        .any(|drop| &*drop.stack.item == "minecraft:mutton")
                );
            })
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn cancelled_corpse_ordinary_saved_load_does_not_retry_death_or_restart_its_clock() {
    crate::game::in_own_process(
        "game::native_hooks::death_source_tests::cancelled_corpse_ordinary_saved_load_does_not_retry_death_or_restart_its_clock",
        || {
            observed(true, false, |game, seen| {
                player(game);
                mob(game, 44, "minecraft:sheep", Vec3::new(6.0, 80.0, 4.5));
                game.mobs[0].health = 1.0;
                player_arrow(game, &bow());
                game.tick_projectiles().unwrap();
                game.tick_creature_entities().unwrap();
                while game.mob_object(44).unwrap().death_time < 19 {
                    game.tick_creature_entities().unwrap();
                }
                let saved = game.vehicle_tree_nbt(44).unwrap();
                game.mobs.retain(|body| body.id != 44);
                game.forget_entity(44);
                game.native_leaves_flush();
                // Ordinary disk reconstruction: deliberately no runtime transfer token.
                let restored = game.load_mob(saved, None).unwrap();
                game.native_joins_flush(false, 0);
                let body = game.mob_object(restored).unwrap();
                assert_eq!(
                    (body.health, body.death_time, body.death_canceled),
                    (0.0, 19, true)
                );
                game.tick_creature_entities().unwrap();
                assert!(
                    game.mob_object(restored).is_none(),
                    "the twentieth tick remains the original deadline"
                );
                assert_eq!(
                    seen.lock().unwrap().deaths.len(),
                    1,
                    "ordinary loading never retries the cancelled die"
                );
                assert!(game.dropped.is_empty());
                assert!(game.orbs.is_empty());
            })
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and loopback sockets"]
fn later_cancelled_player_revival_is_transactional_and_a_second_kill_drops_inventory() {
    crate::game::in_own_process(
        "game::native_hooks::death_source_tests::later_cancelled_player_revival_is_transactional_and_a_second_kill_drops_inventory",
        || {
            observed(true, false, |game, seen| {
                crate::game::native_player_death::tests::add_guest(game);
                game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                    guest.health = 0.0;
                    guest.loaded = true;
                    guest.load_timeout = 0;
                    guest.inventory[0] = Some(ItemStack::new("minecraft:diamond", 3));
                    guest.carried = Some(ItemStack::new("minecraft:diamond", 2));
                });
                game.guest_die(0, 1);
                game.tick_guests();
                let id = game.lan.as_ref().unwrap().hub.guests[0].entity_id as u32;
                let living = {
                    let host: &mut dyn ServerHost = game;
                    host.living(host.entity(id).unwrap()).unwrap()
                };
                let abort = game.transaction_open();
                assert!(living.set_health(game, 5.0));
                let guest = &game.lan.as_ref().unwrap().hub.guests[0];
                assert_eq!(
                    (guest.health, guest.death_time, guest.death_canceled),
                    (5.0, 0, false)
                );
                game.transaction_close(&abort, false);
                let guest = &game.lan.as_ref().unwrap().hub.guests[0];
                assert_eq!(
                    (guest.health, guest.death_time, guest.death_canceled),
                    (0.0, 1, true)
                );
                game.guest_die(0, 1);
                assert_eq!(
                    seen.lock().unwrap().deaths.len(),
                    1,
                    "abort restored the cancelled attempt"
                );
                assert!(game.dropped.is_empty());
                assert!(living.set_health(game, 5.0));
                let guest = &game.lan.as_ref().unwrap().hub.guests[0];
                assert_eq!(
                    (guest.health, guest.death_time, guest.death_canceled),
                    (5.0, 0, false)
                );
                assert!(!guest.motion.as_ref().unwrap().health.dead);
                assert!(game.run_command("kill @a"));
                game.guest_die(0, 1);
                assert_eq!(seen.lock().unwrap().deaths.len(), 2);
                assert!(game.lan.as_ref().unwrap().hub.guests[0].dead);
                // Inventory drops in die; the cursor belongs to tickDeath at the
                // ordinary twentieth tick, not to the earlier cancelled attempt.
                for tick in 1..20 {
                    game.world.game_time += 1;
                    game.tick_guests();
                    let guest = &game.lan.as_ref().unwrap().hub.guests[0];
                    assert_eq!(guest.death_time, tick);
                    assert!(guest.body_present());
                    assert_eq!(guest.carried, Some(ItemStack::new("minecraft:diamond", 2)));
                }
                game.world.game_time += 1;
                game.tick_guests();
                let guest = &game.lan.as_ref().unwrap().hub.guests[0];
                assert_eq!(guest.death_time, 20);
                assert!(!guest.body_present());
                assert!(guest.carried.is_none());
                assert_eq!(
                    game.dropped
                        .iter()
                        .filter(|drop| &*drop.stack.item == "minecraft:diamond")
                        .map(|drop| drop.stack.count)
                        .sum::<u32>(),
                    5
                );
            })
        },
    );
}

/// Hook payloads and entity handles preserve mod namespaces and qualify bare vanilla ids.
#[test]
#[ignore = "daily"]
fn mob_death_and_handle_preserve_namespaced_types() {
    crate::game::in_own_process(
        "game::native_hooks::death_source_tests::mob_death_and_handle_preserve_namespaced_types",
        || {
            observed(false, false, |game, seen| {
                let sheep = game.res.mob_types().get("minecraft:sheep").unwrap().clone();
                for (index, id) in ["vulture:vulture", "sheep", "minecraft:sheep"]
                    .into_iter()
                    .enumerate()
                {
                    let mut kind = (*sheep).clone();
                    kind.id = id.to_owned();
                    let mut mob = Mob::new(
                        201 + index as u32,
                        Arc::new(kind),
                        Vec3::new(4.5, 80.0, 4.5),
                        0.0,
                    );
                    mob.persistent = true;
                    mob.no_ai = true;
                    game.mobs.push(mob);
                }
                assert!(game.run_command("kill @e"));
                assert_eq!(
                    seen.lock().unwrap().kinds,
                    [
                        ("vulture:vulture".to_owned(), "vulture:vulture".to_owned()),
                        ("minecraft:sheep".to_owned(), "minecraft:sheep".to_owned()),
                        ("minecraft:sheep".to_owned(), "minecraft:sheep".to_owned()),
                    ]
                );
            })
        },
    );
}
