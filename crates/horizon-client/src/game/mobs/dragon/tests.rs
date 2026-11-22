use super::*;

/// Vanilla 26.3's `EnderDragon.findClosestNode` builds 24 nodes by feeding a
/// float angle to `Mth.cos`/`Mth.sin`, which index a 65,536-entry table of
/// f32 sines rather than calling `cosf`. Each pair is the `x`/`z` vanilla
/// floors out of that table; the comment on a line shows what
/// `f32::cos`/`f32::sin` gives there instead, one block off along an axis.
#[test]
fn dragon_path_nodes_match_the_vanilla_sine_table() {
    let expected = [
        (60, 0),  // cosf: (60, -1)
        (51, 30), // cosf: (51, 29)
        (29, 51),
        (0, 60),
        (-31, 51), // cosf: (-30, 51)
        (-52, 29), // cosf: (-52, 30)
        (-60, 0),
        (-52, -31),
        (-30, -52), // cosf: (-31, -52)
        (0, -60),   // cosf: (-1, -60)
        (29, -52),
        (51, -30),
        (40, 0), // cosf: (40, -1)
        (28, 28),
        (0, 40),
        (-29, 28),
        (-40, 0),
        (-29, -29),
        (0, -40), // cosf: (-1, -40)
        (28, -29),
        (20, 0), // cosf: (20, -1)
        (0, 20),
        (-20, 0),
        (0, -20), // cosf: (-1, -20)
    ];
    let got: Vec<(i32, i32)> = (0..24).map(dragon_node_column).collect();
    assert_eq!(got, expected);
}

#[test]
#[ignore = "daily"]
fn a_spawn_egg_dragon_joins_once_and_a_canceled_spawn_fails() {
    use mod_api::{EntityJoinLevelEvent, Flow, Hooks, ModEntry, ServerHost};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };

    struct Observer {
        joins: Arc<Mutex<Vec<(u32, bool)>>>,
        cancel: Arc<AtomicBool>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn entity_join_level(
            &self,
            _: &mut dyn ServerHost,
            event: &mut EntityJoinLevelEvent<'_>,
        ) -> Flow {
            if event.kind != "minecraft:ender_dragon" {
                return Flow::Continue;
            }
            self.joins
                .lock()
                .unwrap()
                .push((event.entity, event.loaded_from_disk));
            if self.cancel.load(Ordering::Relaxed) {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }
    let joins = Arc::new(Mutex::new(Vec::new()));
    let cancel = Arc::new(AtomicBool::new(false));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "dragon-joins",
        Box::new(Observer {
            joins: joins.clone(),
            cancel: cancel.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.dragon.discard();
            game.native_joins_skip();
            let at = [4.5, 200.0, 4.5];
            let id = game
                .spawn_mob_spawned(
                    "minecraft:ender_dragon",
                    at,
                    super::super::creatures::SpawnSite::Finalizing(
                        horizon_client::mobs::spawn_rules::Reason::SpawnItemUse,
                    ),
                )
                .unwrap();
            assert_eq!(game.dragon.as_ref().unwrap().id, id);
            assert_eq!(*joins.lock().unwrap(), vec![(id, false)]);
            game.native_joins_flush(false, 0);
            assert_eq!(joins.lock().unwrap().len(), 1, "a join is told only once");
            let dispenser =
                SpawnSite::Finalizing(horizon_client::mobs::spawn_rules::Reason::Dispenser);
            assert!(game.spawn_dragon_spawned(at, dispenser).is_none());
            assert_eq!(joins.lock().unwrap().len(), 1, "no second dragon joins");

            game.dragon.discard();
            cancel.store(true, Ordering::Relaxed);
            // The dispenser calls the shared constructor directly.
            assert!(game.spawn_dragon_spawned(at, dispenser).is_none());
            assert!(game.dragon.is_none(), "a refused dragon never stays loaded");
            game.native_joins_flush(false, 0);
            assert_eq!(*joins.lock().unwrap(), vec![(id, false), (id + 9, false)]);
        });
    });
}

#[test]
#[ignore = "daily"]
fn dragon_finalization_preserves_the_site_and_both_cancels() {
    use horizon_client::mobs::spawn_rules::Reason;
    use mod_api::{FinalizeSpawnEvent, Flow, Hooks, Host, ModEntry};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Clone, Copy, PartialEq)]
    enum Answer {
        Keep,
        Skip,
        Refuse,
    }
    struct Listener {
        answer: Answer,
        reason: Reason,
        calls: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn finalize_spawn(&self, _: &dyn Host, event: &mut FinalizeSpawnEvent<'_>) -> Flow {
            assert_eq!(event.kind, "minecraft:ender_dragon");
            assert_eq!(event.reason, self.reason);
            assert!(event.spawner.is_none());
            self.calls.fetch_add(1, Ordering::Relaxed);
            match self.answer {
                Answer::Keep => Flow::Continue,
                Answer::Skip => Flow::Cancel,
                Answer::Refuse => {
                    event.spawn_cancelled = true;
                    Flow::Continue
                }
            }
        }
    }
    super::super::native_entity_refs::tests::with_game(|game| {
        for reason in [Reason::SpawnItemUse, Reason::Dispenser] {
            for answer in [Answer::Keep, Answer::Skip, Answer::Refuse] {
                game.dragon.discard();
                game.world.random.set_seed(42);
                let calls = Arc::new(AtomicUsize::new(0));
                let mods: mod_api::Mods = vec![ModEntry::new(
                    "dragon-finalize",
                    Box::new(Listener {
                        answer,
                        reason,
                        calls: calls.clone(),
                    }),
                )]
                .into();
                let result = mod_platform::testing::with_mods(mods, || {
                    game.spawn_dragon_spawned([4.5, 200.0, 4.5], SpawnSite::Finalizing(reason))
                });
                assert_eq!(calls.load(Ordering::Relaxed), 1);
                assert_eq!(result.is_none(), answer == Answer::Refuse);
                assert_eq!(game.dragon.is_none(), answer == Answer::Refuse);
                // snapTo always draws yaw; only completed Mob.finalizeSpawn
                // draws the follow-range triangle and handedness roll.
                let mut expected = LegacyRandom::new(42);
                let yaw = horizon_client::camera::wrap_degrees(expected.next_float() * 360.0);
                if let Some(dragon) = game.dragon.as_ref() {
                    assert_eq!(dragon.yaw, yaw);
                    assert_eq!(dragon.previous_yaw, yaw);
                }
                if answer == Answer::Keep {
                    expected.triangle(0.0, 0.114_850_000_000_000_01);
                    expected.next_float();
                }
                assert_eq!(game.world.random.next_long(), expected.next_long());
            }
        }
    });
}

#[test]
#[ignore = "daily"]
fn dragon_summon_command_finalizes_only_without_nbt() {
    use horizon_client::mobs::spawn_rules::Reason;
    use mod_api::{
        EntityJoinLevelEvent, FinalizeSpawnEvent, Flow, Hooks, Host, ModEntry, ServerHost,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Clone, Copy, PartialEq)]
    enum Answer {
        Keep,
        Skip,
        Refuse,
        RefuseJoin,
    }
    struct Listener {
        answer: Answer,
        reason: Reason,
        calls: Arc<AtomicUsize>,
        joins: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn entity_join_level(
            &self,
            _: &mut dyn ServerHost,
            event: &mut EntityJoinLevelEvent<'_>,
        ) -> Flow {
            assert_eq!(event.kind, "minecraft:ender_dragon");
            assert!(!event.loaded_from_disk);
            self.joins.fetch_add(1, Ordering::Relaxed);
            if self.answer == Answer::RefuseJoin {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
        fn finalize_spawn(&self, _: &dyn Host, event: &mut FinalizeSpawnEvent<'_>) -> Flow {
            assert_eq!(event.kind, "minecraft:ender_dragon");
            assert_eq!(event.reason, self.reason);
            assert!(event.spawner.is_none());
            self.calls.fetch_add(1, Ordering::Relaxed);
            match self.answer {
                Answer::Keep | Answer::RefuseJoin => Flow::Continue,
                Answer::Skip => Flow::Cancel,
                Answer::Refuse => {
                    event.spawn_cancelled = true;
                    Flow::Continue
                }
            }
        }
    }
    super::super::native_entity_refs::tests::with_game(|game| {
        for nbt in [None, Some("{}"), Some("{Silent:1b}")] {
            let reason = Reason::Command;
            for answer in [
                Answer::Keep,
                Answer::Skip,
                Answer::Refuse,
                Answer::RefuseJoin,
            ] {
                game.dragon.discard();
                game.world.random.set_seed(42);
                let calls = Arc::new(AtomicUsize::new(0));
                let joins = Arc::new(AtomicUsize::new(0));
                let mods: mod_api::Mods = vec![ModEntry::new(
                    "dragon-finalize",
                    Box::new(Listener {
                        answer,
                        reason,
                        calls: calls.clone(),
                        joins: joins.clone(),
                    }),
                )]
                .into();
                let result = mod_platform::testing::with_mods(mods, || {
                    game.build_registries();
                    game.set_allow_commands(true);
                    let suffix = nbt.map(|tag| format!(" {tag}")).unwrap_or_default();
                    game.run_command(&format!(
                        "summon minecraft:ender_dragon 4.5 200 4.5{suffix}"
                    ))
                });
                assert_eq!(calls.load(Ordering::Relaxed), usize::from(nbt.is_none()));
                let refused_finalization = nbt.is_none() && answer == Answer::Refuse;
                assert_eq!(
                    joins.load(Ordering::Relaxed),
                    usize::from(!refused_finalization)
                );
                let refused = refused_finalization || answer == Answer::RefuseJoin;
                assert_eq!(result, !refused);
                assert_eq!(game.dragon.is_none(), refused);
                if !refused {
                    assert_eq!(game.dragon_silent(), nbt == Some("{Silent:1b}"));
                    let dragon = game.dragon.as_ref().unwrap();
                    assert_eq!(dragon.position, Vec3::new(4.5, 200.0, 4.5));
                    assert_eq!(dragon.yaw, 0.0);
                    assert_eq!(dragon.previous_yaw, 0.0);
                }
                // Command construction consumes no placement yaw. Mob.finalizeSpawn
                // draws follow range and handedness unless skipped. The NBT
                // loader seeds the dragon's own random without drawing from the level RNG.
                let mut expected = LegacyRandom::new(42);
                if nbt.is_none() && matches!(answer, Answer::Keep | Answer::RefuseJoin) {
                    expected.triangle(0.0, 0.114_850_000_000_000_01);
                    expected.next_float();
                }
                assert_eq!(game.world.random.next_long(), expected.next_long());
            }
        }
    });
}

#[test]
fn a_dragon_save_keeps_equipment_only_while_something_is_equipped() {
    use horizon_client::items::ItemStack;
    use horizon_items::equipment::{BODY, SADDLE};
    let mut d = Dragon::new(Vec3::ZERO, 0.0, 0);
    // An old save has no `equipment` key: the dragon loads empty and saves none.
    let old = d.to_nbt([0; 4], None);
    assert!(old.get("equipment").is_none());
    assert!(Dragon::from_nbt(&old, 0).unwrap().equipment.is_empty());
    // Body and saddle round-trip with their name and damage.
    let mut body = ItemStack::new("minecraft:diamond_horse_armor", 1);
    body.damage = 6;
    body.data_mut().custom_name = Some(Tag::String("Plate".into()));
    d.equipment[BODY] = Some(body);
    d.equipment[SADDLE] = Some(ItemStack::new("minecraft:saddle", 1));
    let saved = d.to_nbt([0; 4], None);
    assert_eq!(Dragon::from_nbt(&saved, 0).unwrap().equipment, d.equipment);
    // Saving over a raw save that holds a stale key removes it once emptied.
    d.equipment.take_all().for_each(drop);
    assert!(d.to_nbt([0; 4], Some(&saved)).get("equipment").is_none());
}

fn player_box(feet: [f64; 3]) -> (Aabb, Vec3) {
    (
        Aabb::new(
            Vec3::new(feet[0] - 0.3, feet[1], feet[2] - 0.3),
            Vec3::new(feet[0] + 0.3, feet[1] + 1.8, feet[2] + 0.3),
        ),
        Vec3::new(feet[0], feet[1], feet[2]),
    )
}

/// `Entity.load` and `EndCrystal.readAdditionalSaveData` read their booleans with
/// `getBooleanOr`: any number, `ShowBottom` true when absent.
#[test]
fn a_crystal_reads_its_flags_from_any_number() {
    let crystal = |fields: Vec<(&str, Tag)>| {
        let pos = [0.5f64, 64.0, 0.5].map(|v| Tag::Double(v.to_bits()));
        let mut all = vec![("Pos", Tag::List(pos.to_vec()))];
        all.extend(fields);
        Crystal::from_nbt(&Tag::compound(all), 0).unwrap()
    };
    let c = crystal(vec![
        ("Invulnerable", Tag::Byte(2)),
        ("ShowBottom", Tag::Int(0)),
    ]);
    assert!(c.invulnerable && !c.show_bottom);
    let c = crystal(Vec::new());
    assert!(!c.invulnerable && c.show_bottom);
}

/// Under `HORIZON_STILL` a crystal's spin and bob start from its position, whichever order
/// the crystals load in; without it each takes the next draw, as vanilla's do.
#[test]
fn a_still_crystal_starts_from_its_position_in_any_load_order() {
    let spikes = [Vec3::new(42.5, 77.0, 0.5), Vec3::new(-33.5, 95.0, 24.5)];
    let starts = |order: [usize; 2], still: bool| {
        let mut random = LegacyRandom::new(5);
        let mut times = [0; 2];
        for i in order {
            let drawn = random.next_int_bound(100_000).unwrap();
            times[i] = Crystal::start_time(spikes[i], drawn, still);
        }
        times
    };
    let still = starts([0, 1], true);
    assert_eq!(still, starts([1, 0], true));
    assert_ne!(still[0], still[1], "each crystal keeps its own phase");
    let first = LegacyRandom::new(5).next_int_bound(100_000).unwrap();
    assert_eq!(starts([0, 1], false)[0], first);
    assert_eq!(
        starts([1, 0], false)[1],
        first,
        "the first loaded takes the first draw"
    );
}
/// `Mth.wrapDegrees`: `% 360` then one step of 360, exact for a negative angle
/// (a remainder made positive and back again loses the float's low bits).
#[test]
fn wrap_is_mth_wrap_degrees() {
    for (d, wrapped) in [
        (-10.123456_f32, -10.123456_f32),
        (190.0, -170.0),
        (-190.0, 170.0),
        (540.25, 180.25 - 360.0),
        (-0.0001, -0.0001),
    ] {
        assert_eq!(wrap(d), wrapped, "{d}");
    }
    assert_eq!(wrap64(-10.123456789), -10.123456789);
}
/// `EnderDragon.tickDeath` roars (`globalLevelEvent(1028)`) only when
/// `!isSilent()`; `Silent` is saved only when set, as a byte.
#[test]
fn a_dragon_is_silent_only_with_a_set_silent_byte() {
    use horizon_core::nbt::{Compound, Tag};
    let with = |v: Option<i8>| {
        let mut map = Compound::new();
        if let Some(b) = v {
            map.insert("Silent".into(), Tag::Byte(b));
        }
        Tag::Compound(map)
    };
    assert!(!super::saved_silent(None));
    assert!(!super::saved_silent(Some(&with(None))));
    assert!(!super::saved_silent(Some(&with(Some(0)))));
    assert!(super::saved_silent(Some(&with(Some(1)))));
}

#[test]
fn breath_lifetime_excludes_the_wait() {
    assert!(!breath_expired(219, 200));
    assert!(breath_expired(220, 200));
    assert!(!breath_expired(619, 600));
    assert!(breath_expired(620, 600));
}

#[test]
fn breath_victims_cadence() {
    let (a, b) = (FightWho::Host, FightWho::Guest(1));
    let mut v = Vec::new();
    assert!(breath_victims(15, &mut v, [a]).is_empty());
    assert_eq!(breath_victims(20, &mut v, [a]), vec![a]);
    assert!(breath_victims(25, &mut v, [a]).is_empty());
    // A newcomer is hit at the next 5-tick check, not the next 20.
    assert_eq!(breath_victims(30, &mut v, [a, b]), vec![b]);
    assert!(breath_victims(35, &mut v, [a, b]).is_empty());
    assert_eq!(breath_victims(40, &mut v, [a, b]), vec![a]);
    assert_eq!(breath_victims(50, &mut v, [a, b]), vec![b]);
}

#[test]
fn instant_damage_at_cloud_scale() {
    assert_eq!(harm_amount(0), 3.0);
    assert_eq!(harm_amount(1), 6.0);
}

#[test]
fn cloud_box_is_half_a_block_tall_from_its_floor() {
    let at = [0.0, 64.0, 0.0];
    let hit = |feet: [f64; 3], r: f32| {
        let (b, p) = player_box(feet);
        cloud_hits(at, r, b, p)
    };
    assert!(hit([0.0, 64.0, 0.0], 3.0));
    // Feet above the 0.5 box: the old |dy| < 1 test hit here.
    assert!(!hit([0.0, 64.6, 0.0], 3.0));
    // Feet below, head reaching into the box.
    assert!(hit([0.0, 63.0, 0.0], 3.0));
    // Box overlaps but the centre is outside the radius.
    assert!(!hit([3.2, 64.0, 0.0], 3.0));
    assert!(hit([3.0, 64.0, 0.0], 3.0));
}

/// `DragonFireball.onHit` creates the cloud at the fireball's hit position.
/// The shared projectile engine stores the box bottom there, so no -0.5.
#[test]
#[ignore = "daily"]
fn a_fireballs_breath_cloud_starts_at_its_hit_position() {
    super::super::native_entity_refs::tests::with_game(|game| {
        let at = Vec3::new(0.5, 120.0, 0.5);
        game.dragon_fireball_on_hit(at);
        let cloud = game.breath_clouds.last().expect("one breath cloud");
        assert_eq!(cloud.at, [at.x, at.y, at.z]);
    });
}

#[test]
fn fireball_cloud_snaps_to_a_player_within_four() {
    let (b, p) = player_box([2.0, 70.0, 1.0]);
    assert_eq!(
        fireball_cloud_origin([0.0, 70.0, 0.0], &[(b, p)]),
        [2.0, 70.0, 1.0]
    );
    let (b, p) = player_box([4.5, 70.0, 0.0]);
    assert_eq!(
        fireball_cloud_origin([0.0, 70.0, 0.0], &[(b, p)]),
        [0.0, 70.0, 0.0]
    );
}

#[test]
fn dying_dragons_keep_their_flight_pose_between_frames() {
    let mut dragon = Dragon::new(Vec3::new(0.0, 80.0, 0.0), 0.0, 432);
    dragon.history.fill((70.0, 170.0));
    dragon.position.y = 90.0;
    dragon.yaw = -170.0;
    dragon.record();
    assert_eq!(dragon.sample_lerp(0, 0.5), (80.0, 180.0));
    dragon.health = 0.0;
    for partial in [0.0, 0.25, 0.5, 0.75, 1.0] {
        assert_eq!(dragon.sample_lerp(0, partial), (70.0, 170.0));
    }
}

/// Driven by `tools/test_dragon_render_parity.py`; compares the actual
/// ray vertices with EnderDragonRenderer's captured VertexConsumer output.
#[test]
#[ignore]
fn dragon_render_parity_fixture() {
    let path = std::env::var("HORIZON_DRAGON_RENDER_OUTPUT").expect("harness output path");
    let mut cases = Vec::new();
    for t in [0.0, 0.05, 0.25, 0.5, 0.8, 0.9, 0.95, 1.0] {
        let mut vertices = Vec::new();
        dragon_rays(
            glam::Mat4::IDENTITY,
            t,
            horizon_client::hud::Rect {
                u0: 0.0,
                v0: 0.0,
                u1: 1.0,
                v1: 1.0,
                w: 1,
                h: 1,
            },
            &mut vertices,
        );
        // The fourth quad vertex only pads a triangle for our index buffer.
        let triangles: Vec<_> = vertices
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|q| &q[..3])
            .map(|v| serde_json::json!([v.pos[0], v.pos[1], v.pos[2], v.color]))
            .collect();
        cases.push(triangles);
    }
    std::fs::write(path, serde_json::to_vec(&cases).unwrap()).unwrap();
}

#[test]
fn the_dying_model_reads_the_mask_where_it_reads_the_skin() {
    // The skin in the atlas's top-left quarter, the mask in its bottom-right one.
    let mut quad = [Vertex {
        pos: [0.0; 3],
        uv: [0.25, 0.125],
        layer: 0,
        color: 0,
        light: 0,
        normal: [0; 4],
        glint_uv: [0.0; 2],
    }];
    dissolve_mask(&mut quad, [0.0, 0.0, 0.5, 0.5], [0.5, 0.5, 1.0, 1.0]);
    assert_eq!(quad[0].glint_uv, [0.75, 0.625]);
    // ARGB.white(1 - deathTime / 200): floor(127.5), and past 200 the int shift wraps.
    assert_eq!(dissolve_color(100.0), 0x7FFF_FFFF);
    assert_eq!(dissolve_color(200.5), 0xFFFF_FFFF);
}

#[test]
fn death_rays_sample_inside_the_white_sprite() {
    // A four-texel white sprite surrounded by transparent atlas pixels.
    // A border UV can round outside it during perspective interpolation.
    let size = 1024.0;
    let white = horizon_client::hud::Rect {
        u0: 20.0 / size,
        v0: 60.0 / size,
        u1: 24.0 / size,
        v1: 64.0 / size,
        w: 4,
        h: 4,
    };
    for t in [0.25, 0.8, 0.95, 1.0] {
        let mut vertices = Vec::new();
        dragon_rays(glam::Mat4::IDENTITY, t, white, &mut vertices);
        assert!(!vertices.is_empty());
        for v in &vertices {
            assert!(
                v.uv[0] > white.u0 + 0.5 / size && v.uv[0] < white.u1 - 0.5 / size,
                "death-ray U touches the atlas border: {}",
                v.uv[0]
            );
            assert!(
                v.uv[1] > white.v0 + 0.5 / size && v.uv[1] < white.v1 - 0.5 / size,
                "death-ray V touches the atlas border: {}",
                v.uv[1]
            );
        }
    }
}

/// The damage type of the hits below, which count.
const HIT: Option<HurtSource> = Some(HurtSource {
    damage_type: "minecraft:player_attack",
    causing: 0,
    direct: 0,
    position: None,
});
fn dragon(phase: Phase) -> Dragon {
    let mut d = Dragon::new(Vec3::new(0.0, 70.0, 0.0), 0.0, 1);
    d.phase = phase;
    d
}

/// `EnderDragon.readAdditionalSaveData` reads `sitting_damage_received`
/// (`getFloatOr`, 0 when absent); the writer already saved it. Horizon
/// only wrote it, so a perched dragon that took a quarter of its health
/// but was saved before taking off came back with the damage forgotten.
#[test]
fn a_perched_dragons_damage_survives_the_chunk_save() {
    let mut d = dragon(Phase::Flaming { ticks: 0 });
    d.sitting_damage = 12.5;
    let tag = d.to_nbt([1, 2, 3, 4], None);
    let back = Dragon::from_nbt(&tag, 0).unwrap();
    assert_eq!(back.sitting_damage, 12.5);
    // No key at all reads as no damage accumulated.
    let mut bare = tag.clone();
    if let horizon_core::nbt::Tag::Compound(fields) = &mut bare {
        fields.remove(&"sitting_damage_received".into());
    }
    assert_eq!(Dragon::from_nbt(&bare, 0).unwrap().sitting_damage, 0.0);
}
fn flying() -> Phase {
    Phase::Holding {
        path: None,
        target: None,
        clockwise: false,
    }
}

/// `DragonLandingPhase.doClientTick`: eight puffs a tick from the head,
/// along `getHeadLookVector(1)` (a quarter turn of it is thrown away in
/// vanilla) plus the dragon's own motion.
#[test]
fn a_landing_dragon_breathes_eight_puffs_from_its_head() {
    let look = [0.0, -0.6, 0.8];
    let head = [10.0, 70.5, -3.0];
    let motion = [0.25, -0.05, 0.5];
    let mut random = LegacyRandom::new(7);
    let puffs = breath_puffs(
        &Phase::Landing { target: None },
        look,
        head,
        motion,
        &mut random,
    );
    assert_eq!(puffs.len(), 8);
    let speed = [
        -look[0] * f64::from(0.08_f32) + motion[0],
        -look[1] * f64::from(0.3_f32) + motion[1],
        -look[2] * f64::from(0.08_f32) + motion[2],
    ];
    for (at, v) in puffs {
        assert_eq!(v, speed);
        // The head's centre, jittered by `nextGaussian() / 2`.
        for a in 0..3 {
            assert!(
                (at[a] - head[a]).abs() < 2.0,
                "puff {} off the head: {at:?}",
                a
            );
        }
    }
}

/// `DragonSittingFlamingPhase.doClientTick`: on the even ticks of the first
/// ten, eight starts each breathe a fan of six puffs down the look line.
#[test]
fn a_flaming_dragon_breathes_a_fan_on_every_second_tick() {
    let look = [0.6, -0.0, 0.8];
    let head = [0.0, 65.5, 4.0];
    for (tick, want) in [(1, 0), (2, 48), (3, 0), (4, 48), (8, 48), (9, 0), (10, 0)] {
        let mut random = LegacyRandom::new(11);
        let puffs = breath_puffs(
            &Phase::Flaming { ticks: tick },
            look,
            head,
            [0.0; 3],
            &mut random,
        );
        assert_eq!(puffs.len(), want, "flame tick {tick}");
    }
    let mut random = LegacyRandom::new(11);
    let puffs = breath_puffs(
        &Phase::Flaming { ticks: 2 },
        look,
        head,
        [0.0; 3],
        &mut random,
    );
    for (i, (at, v)) in puffs.iter().enumerate() {
        let j = (i % 6) as f64;
        // Six puffs leave from each of the eight starts.
        assert_eq!(at, &puffs[i - i % 6].0);
        assert_eq!(
            *v,
            [
                -look[0] * f64::from(0.08_f32) * j,
                -look[1] * f64::from(0.6_f32),
                -look[2] * f64::from(0.08_f32) * j,
            ]
        );
    }
}

/// No other phase has a `doClientTick` that breathes.
#[test]
fn only_the_landing_and_flaming_phases_breathe() {
    let mut random = LegacyRandom::new(3);
    for phase in [
        flying(),
        Phase::Scanning { time: 5 },
        Phase::Hover { target: None },
        Phase::Takeoff {
            first: false,
            path: None,
            target: None,
        },
        Phase::Attacking { ticks: 3 },
        Phase::Dying {
            target: None,
            time: 0,
        },
    ] {
        assert!(
            breath_puffs(&phase, [0.0, -1.0, 0.0], [0.0; 3], [0.0; 3], &mut random).is_empty(),
            "{phase:?}"
        );
    }
}

#[test]
fn a_perched_dragon_shrugs_arrows_off() {
    let mut test_level = horizon_client::health::TestLevel::new(0);
    let level = &mut test_level.level();
    // AbstractDragonSittingPhase.onHurt: an arrow does nothing, melee does.
    let mut d = dragon(Phase::Scanning { time: 0 });
    assert_eq!(d.hurt(level, 0, 9.0, HIT, true), Hurt::Shrugged);
    assert_eq!(d.health, MAX_HEALTH);
    assert_eq!(
        d.hurt(level, 0, 9.0, HIT, false),
        Hurt::Landed { full: true }
    );
    assert_eq!(d.health, MAX_HEALTH - 9.0);
    // In flight an arrow lands.
    let mut d = dragon(flying());
    assert_eq!(
        d.hurt(level, 0, 9.0, HIT, true),
        Hurt::Landed { full: true }
    );
    assert_eq!(d.health, MAX_HEALTH - 9.0);
}

/// `EnderDragon.knockback` calls `LivingEntity.knockback` only when the
/// phase is not sitting. `dealDefaultKnockback` uses `0.4F` away from the
/// attacker's feet. Resistance is 0 and `onGround` stays false
/// (`noPhysics`), so from velocity `(1, -0.3, 0.5)` and a source 3 by 4
/// blocks away the push is `(0.5 - 0.6p, -0.3, 0.25 - 0.8p)` with
/// `p = (double) 0.4F`. A perched hit, including one that makes it take
/// off, leaves the velocity alone. A hit inside the damage cooldown is
/// not a full hit, so it does not push either.
#[test]
fn a_melee_hit_knocks_a_flying_dragon_and_not_a_perched_one() {
    let mut test_level = horizon_client::health::TestLevel::new(0);
    let level = &mut test_level.level();
    let from = Vec3::new(3.0, 70.0, 4.0);
    let velocity = Vec3::new(1.0, -0.3, 0.5);
    let power = f64::from(0.4_f32);
    let pushed = Vec3::new(0.5 - 0.6 * power, -0.3, 0.25 - 0.8 * power);

    let mut perched = dragon(Phase::Scanning { time: 0 });
    perched.velocity = velocity;
    assert_eq!(
        perched.hurt_from(level, 0, 9.0, HIT, false, Some(from), false),
        Hurt::Landed { full: true }
    );
    assert_eq!(perched.velocity, velocity);

    let mut taking_off = dragon(Phase::Flaming { ticks: 0 });
    taking_off.velocity = velocity;
    taking_off.hurt_from(level, 0, 60.0, HIT, false, Some(from), false);
    assert!(matches!(taking_off.phase, Phase::Takeoff { .. }));
    assert_eq!(taking_off.velocity, velocity);

    let mut flying = dragon(flying());
    flying.velocity = velocity;
    assert_eq!(
        flying.hurt_from(level, 0, 9.0, HIT, false, Some(from), false),
        Hurt::Landed { full: true }
    );
    assert_eq!(flying.velocity, pushed);
    assert_eq!(
        flying.hurt_from(level, 0, 20.0, HIT, false, Some(from), false),
        Hurt::Landed { full: false }
    );
    assert_eq!(flying.velocity, pushed);
}

#[test]
fn a_part_refuses_fire() {
    let mut test_level = horizon_client::health::TestLevel::new(0);
    let level = &mut test_level.level();
    // EnderDragonPart.hurtServer: the dragon is fire immune, so a
    // fireball's hit is refused on the head and on the other parts.
    let large = ProjectileKind::LargeFireball { power: 1 };
    for part in [0, 2] {
        let mut d = dragon(flying());
        let hurt = d.hurt_part(level, part, 6.0, HIT, DamageKind::Fireball, Some(&large));
        assert_eq!(hurt, Hurt::Refused);
        assert_eq!(d.health, MAX_HEALTH);
    }
    let small = ProjectileKind::SmallFireball;
    let mut d = dragon(flying());
    let hurt = d.hurt_part(level, 0, 5.0, HIT, DamageKind::Fireball, Some(&small));
    assert_eq!(hurt, Hurt::Refused);
    assert_eq!(d.health, MAX_HEALTH);
}

#[test]
fn a_perched_dragon_shrugs_a_players_wind_charge_off() {
    let mut test_level = horizon_client::health::TestLevel::new(0);
    let level = &mut test_level.level();
    // AbstractDragonSittingPhase.onHurt: a direct WindCharge does
    // nothing, a breeze's BreezeWindCharge is not one.
    let charge = ProjectileKind::WindCharge;
    let mut d = dragon(Phase::Scanning { time: 0 });
    let hurt = d.hurt_part(level, 0, 1.0, HIT, DamageKind::WindCharge, Some(&charge));
    assert_eq!(hurt, Hurt::Shrugged);
    assert_eq!(d.health, MAX_HEALTH);
    let breeze = ProjectileKind::BreezeWindCharge;
    let hurt = d.hurt_part(level, 0, 1.0, HIT, DamageKind::WindCharge, Some(&breeze));
    assert_eq!(hurt, Hurt::Landed { full: true });
    assert_eq!(d.health, MAX_HEALTH - 1.0);
    // In flight the player's charge lands too.
    let mut d = dragon(flying());
    let hurt = d.hurt_part(level, 0, 1.0, HIT, DamageKind::WindCharge, Some(&charge));
    assert_eq!(hurt, Hurt::Landed { full: true });
    assert_eq!(d.health, MAX_HEALTH - 1.0);
}

#[test]
fn only_a_player_owned_wither_skull_takes_the_dragons_health() {
    let mut test_level = horizon_client::health::TestLevel::new(0);
    let level = &mut test_level.level();
    // WitherSkull.onHitEntity: EnderDragon.hurt takes the wither's skull
    // (not a player, not an explosion) but takes no health, even perched.
    let skull = ProjectileKind::WitherSkull { dangerous: false };
    let wither = super::super::wither::skull_source(Some(Who::Mob(7)), [0.0; 3]);
    assert_eq!((wither.amount, wither.kind), (8.0, DamageKind::WitherSkull));
    let mut d = dragon(Phase::Scanning { time: 0 });
    let hurt = d.hurt_part(level, 2, wither.amount, None, wither.kind, Some(&skull));
    assert_eq!(hurt, Hurt::Landed { full: false });
    assert_eq!(d.health, MAX_HEALTH);
    // A player's skull: 8 on the head, 8 / 4 + 1 on the body.
    let player = super::super::wither::skull_source(Some(Who::Player), [0.0; 3]);
    let mut d = dragon(Phase::Scanning { time: 0 });
    let hurt = d.hurt_part(level, 0, player.amount, HIT, player.kind, Some(&skull));
    assert_eq!(hurt, Hurt::Landed { full: true });
    assert_eq!(d.health, MAX_HEALTH - 8.0);
    let mut d = dragon(flying());
    d.hurt_part(level, 2, player.amount, HIT, player.kind, Some(&skull));
    assert_eq!(d.health, MAX_HEALTH - 3.0);
}

#[test]
fn the_cooldown_lets_only_the_excess_through() {
    let mut test_level = horizon_client::health::TestLevel::new(0);
    let level = &mut test_level.level();
    // LivingEntity.hurtServer: damageCooldownTime > 10 keeps lastHurt.
    let mut d = dragon(flying());
    assert_eq!(
        d.hurt(level, 0, 6.0, HIT, false),
        Hurt::Landed { full: true }
    );
    assert_eq!(
        d.hurt(level, 0, 4.0, HIT, false),
        Hurt::Landed { full: false }
    );
    assert_eq!(d.health, MAX_HEALTH - 6.0);
    assert_eq!(
        d.hurt(level, 0, 10.0, HIT, false),
        Hurt::Landed { full: false }
    );
    assert_eq!(d.health, MAX_HEALTH - 10.0);
    // Ten ticks on, the window is over: a full hit again.
    d.invulnerable = 10;
    assert_eq!(
        d.hurt(level, 0, 3.0, HIT, false),
        Hurt::Landed { full: true }
    );
    assert_eq!(d.health, MAX_HEALTH - 13.0);
}

#[test]
fn parts_and_sources() {
    let mut test_level = horizon_client::health::TestLevel::new(0);
    let level = &mut test_level.level();
    // Neither head nor neck: a quarter plus one.
    let mut d = dragon(flying());
    assert_eq!(
        d.hurt(level, 2, 20.0, HIT, false),
        Hurt::Landed { full: true }
    );
    assert_eq!(d.health, MAX_HEALTH - 6.0);
    // Not a player's and not an explosion: true, but no damage.
    let mut d = dragon(flying());
    assert_eq!(
        d.hurt(level, 0, 20.0, None, false),
        Hurt::Landed { full: false }
    );
    assert_eq!(d.health, MAX_HEALTH);
    // Too little after the part's cut, and nothing while dying.
    assert_eq!(d.hurt(level, 3, 0.001, HIT, false), Hurt::Refused);
    d.phase = Phase::Dying {
        target: None,
        time: 0,
    };
    assert_eq!(d.hurt(level, 0, 20.0, HIT, false), Hurt::Refused);
}

/// The dragon hits the hook of these tests saw: entity, damage type,
/// causing and direct entities, amount.
type Seen = (String, i32, i32, f32);
static SEEN: std::sync::Mutex<Vec<(i32, Seen)>> = std::sync::Mutex::new(Vec::new());
fn seen(entity: i32) -> Vec<Seen> {
    let seen = SEEN.lock().unwrap();
    seen.iter()
        .filter(|s| s.0 == entity)
        .map(|s| s.1.clone())
        .collect()
}
/// Sets the native damage hook of these tests: it records the dragon
/// hits of entities 9001 to 9005, cancels 9001's, doubles 9002's, makes
/// 9003's -5 and 9004's NaN. The other tests' dragons are entity 0.
fn hooks() -> horizon_client::health::DamageHooks {
    use horizon_client::health::{DamageContainer, DamageHooks, DamageLevel, Hurting, Victim};
    fn hook(
        _: DamageLevel<'_>,
        hurting: Hurting<'_>,
        _: Victim,
        container: &mut DamageContainer,
    ) -> bool {
        let Hurting::Dragon(entity @ 9001..=9005, s) = hurting else {
            return false;
        };
        let amount = container.new_damage();
        let hit = (s.damage_type.to_owned(), s.causing, s.direct, amount);
        SEEN.lock().unwrap().push((entity, hit));
        match entity {
            9002 => container.set_new_damage(amount * 2.0),
            9003 => container.set_new_damage(-5.0),
            9004 => container.set_new_damage(f32::NAN),
            _ => {}
        }
        entity == 9001
    }
    DamageHooks {
        incoming: Some(hook),
        ..DamageHooks::NONE
    }
}
#[test]
fn base_check_only_receives_qualifying_dragon_hits() {
    let mut d = dragon(flying());
    assert!(d.reaches_base_check(0, 1.0, true, true));
    assert!(!d.reaches_base_check(0, 1.0, false, false));
    assert!(!d.reaches_base_check(BODY, 0.001, true, false));
    d.set_phase(Phase::Dying {
        target: None,
        time: 0,
    });
    assert!(!d.reaches_base_check(0, 1.0, true, false));
}

#[test]
#[ignore = "daily"]
fn carry_dragon_dispatches_only_after_phase_and_source_guards() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Probe(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Probe {
        fn entity_invulnerability_check(
            &self,
            _: mod_api::LevelSide<'_>,
            _: &mut mod_api::EntityInvulnerabilityCheckEvent<'_>,
        ) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let heard = Arc::new(AtomicUsize::new(0));
    let mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe(heard.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.dragon.replace(dragon(Phase::Scanning { time: 0 }));
            let arrow = ProjectileKind::Trident;
            let hit = |game: &mut Game<'_>, counts, direct| {
                game.hurt_dragon_part_at(
                    0,
                    2.0,
                    counts,
                    None,
                    DamageKind::Generic,
                    DragonHitOrigin {
                        direct,
                        entity_id: None,
                        from: None,
                        invulnerability_checked: false,
                    },
                )
            };
            assert_eq!(hit(game, true, Some(&arrow)), Hurt::Shrugged);
            assert_eq!(heard.load(Ordering::SeqCst), 0);
            game.dragon.replace(dragon(flying()));
            let id = game.dragon.as_ref().unwrap().id;
            assert!(game.command_damage_dragon(
                id,
                2.0,
                "minecraft:generic",
                DamageKind::Generic,
                (None, None, None)
            ));
            assert_eq!(heard.load(Ordering::SeqCst), 0);
            assert!(matches!(hit(game, false, None), Hurt::Landed { .. }));
            assert_eq!(heard.load(Ordering::SeqCst), 0);
            hit(game, true, None);
            assert_eq!(heard.load(Ordering::SeqCst), 1);
        });
    });
}

fn dragon_as(entity: u32) -> Dragon {
    let mut d = dragon(flying());
    d.id = entity;
    d
}

#[test]
fn dragon_invulnerability_hook_blocks_the_matching_damage_type() {
    use horizon_client::health::{DamageHooks, DamageLevel, Hurting};
    fn hook(_: DamageLevel<'_>, hurting: Hurting<'_>, vanilla: bool) -> bool {
        match hurting {
            Hurting::Dragon(9100, source) => source.damage_type == "minecraft:arrow",
            _ => vanilla,
        }
    }
    let hooks = DamageHooks {
        invulnerable: Some(hook),
        ..DamageHooks::NONE
    };
    let mut test_level = horizon_client::health::TestLevel::with_hooks(0, hooks);
    let level = &mut test_level.level();
    let mut d = dragon_as(9100);
    let arrow = HurtSource {
        damage_type: "minecraft:arrow",
        ..HIT.unwrap()
    };
    let generic = HurtSource {
        damage_type: "minecraft:generic",
        ..HIT.unwrap()
    };
    assert_eq!(
        d.hurt(level, 0, 4.0, Some(arrow), false),
        Hurt::Landed { full: false }
    );
    assert_eq!(d.health, MAX_HEALTH);
    assert_eq!(
        d.hurt(level, 0, 4.0, Some(generic), false),
        Hurt::Landed { full: true }
    );
    assert_eq!(d.health, MAX_HEALTH - 4.0);
}

#[test]
fn dragon_part_respects_a_hook_override_of_fire_immunity() {
    let mut test_level = horizon_client::health::TestLevel::new(0);
    let level = &mut test_level.level();
    let mut d = dragon(flying());
    assert_eq!(
        d.hurt_part_from(level, 0, 4.0, HIT, DamageKind::OnFire, None, None, false),
        Hurt::Refused
    );
    assert_eq!(
        d.hurt_part_from(level, 0, 4.0, HIT, DamageKind::OnFire, None, None, true),
        Hurt::Landed { full: true }
    );
    assert_eq!(d.health, MAX_HEALTH - 4.0);
}

#[test]
fn the_native_damage_hook_changes_or_cancels_a_dragons_hit() {
    let mut test_level = horizon_client::health::TestLevel::with_hooks(0, hooks());
    let level = &mut test_level.level();
    // EnderDragon.hurt returns true whatever reallyHurt did. A canceled
    // hit takes no health, starts no cooldown and isn't full: what
    // hurt_dragon_part reads to credit the player and play the hurt sound.
    let mut d = dragon_as(9001);
    let arrow = HurtSource {
        damage_type: "minecraft:arrow",
        causing: 12,
        direct: 0,
        position: None,
    };
    let hurt = d.hurt(level, 0, 9.0, Some(arrow), false);
    assert_eq!(hurt, Hurt::Landed { full: false });
    assert_eq!(seen(9001), [("minecraft:arrow".to_owned(), 12, 0, 9.0)]);
    assert_eq!(d.health, MAX_HEALTH);
    assert_eq!((d.invulnerable, d.last_hurt, d.hurt_time), (0, 0.0, 0));
    // The hook sees the amount after the part's cut (20 / 4 + 1).
    let mut d = dragon_as(9002);
    assert_eq!(
        d.hurt(level, 2, 20.0, HIT, false),
        Hurt::Landed { full: true }
    );
    assert_eq!(
        seen(9002),
        [("minecraft:player_attack".to_owned(), 0, 0, 6.0)]
    );
    assert_eq!(d.health, MAX_HEALTH - 12.0);
    // Neither a hit that doesn't count nor one cut below 0.01 reaches
    // reallyHurt.
    let mut d = dragon_as(9005);
    assert_eq!(
        d.hurt(level, 0, 9.0, None, false),
        Hurt::Landed { full: false }
    );
    assert_eq!(d.hurt(level, 3, 0.001, HIT, false), Hurt::Refused);
    assert!(seen(9005).is_empty());
    // LivingEntity.hurtServer: a negative amount hurts by 0, a NaN by
    // Float.MAX_VALUE (the killing blow: the dying flight).
    let mut d = dragon_as(9003);
    assert_eq!(
        d.hurt(level, 0, 9.0, HIT, false),
        Hurt::Landed { full: true }
    );
    assert_eq!(d.health, MAX_HEALTH);
    let mut d = dragon_as(9004);
    assert_eq!(
        d.hurt(level, 0, 9.0, HIT, false),
        Hurt::Landed { full: true }
    );
    assert_eq!(d.health, 1.0);
    assert!(matches!(d.phase, Phase::Dying { .. }));
}

#[test]
fn a_hit_names_the_player_behind_it() {
    let source = |kind, by, direct| {
        let s = super::hurt_source(kind, by, direct);
        (s.damage_type, s.causing, s.direct)
    };
    let melee = source(DamageKind::PlayerAttack, Some(12), None);
    assert_eq!(melee, ("minecraft:player_attack", 12, 12));
    let mut arrow = Projectile::new(ProjectileKind::Trident, Vec3::ZERO, None);
    arrow.id = 37;
    let shot = source(DamageKind::Arrow, Some(12), Some(arrow.id as i32));
    assert_eq!(shot, ("minecraft:arrow", 12, 37));
    let skull = source(DamageKind::WitherSkull, Some(12), Some(arrow.id as i32));
    assert_eq!(skull, ("minecraft:wither_skull", 12, 37));
    // DamageSources.magic(): an ownerless skull names no direct entity.
    let magic = source(DamageKind::Magic, None, Some(arrow.id as i32));
    assert_eq!(magic, ("minecraft:magic", 0, 0));
    let blast = source(DamageKind::Explosion, Some(12), None);
    assert_eq!(blast, ("minecraft:player_explosion", 12, 0));
    let blast = source(DamageKind::Explosion, None, None);
    assert_eq!(blast, ("minecraft:explosion", 0, 0));
}

#[test]
fn a_quarter_of_its_health_while_perched_makes_it_take_off() {
    let mut test_level = horizon_client::health::TestLevel::new(0);
    let level = &mut test_level.level();
    let mut d = dragon(Phase::Flaming { ticks: 0 });
    d.hurt(level, 0, 30.0, HIT, false);
    assert!(d.phase.sitting());
    d.invulnerable = 0;
    d.hurt(level, 0, 21.0, HIT, false);
    assert!(matches!(d.phase, Phase::Takeoff { .. }));
    assert_eq!(d.sitting_damage, 0.0);
}

#[test]
fn an_explosion_pushes_the_dragon_along_the_blast() {
    let mut d = dragon(flying());
    d.position = Vec3::new(0.0, 70.0, 0.0);
    d.velocity = Vec3::ZERO;
    // Power 5 right under the dragon: it is thrown up, away from the blast.
    let push = d.explosion_knockback([0.0, 66.0, 0.0], 5.0, 1.0).unwrap();
    assert!(push[1] > 0.0 && push[0] == 0.0 && push[2] == 0.0);
    // Out of reach or fully covered: no push.
    assert_eq!(d.explosion_knockback([0.0, 40.0, 0.0], 5.0, 1.0), None);
    assert_eq!(
        d.explosion_knockback([0.0, 66.0, 0.0], 5.0, 0.0),
        Some([0.0; 3])
    );
}

#[test]
fn explosions_hit_each_part_by_its_exposure() {
    let mut test_level = horizon_client::health::TestLevel::new(0);
    let level = &mut test_level.level();
    let mut d = dragon(flying());
    for (i, part) in d.parts.iter_mut().enumerate() {
        part.0 = Vec3::new(i as f64 * 4.0, 70.0, 0.0);
    }
    // A power-6 blast at the dragon's feet: diameter 12.
    let center = [0.0, 70.0, 0.0];
    let open = d.explosion_hits(center, 6.0, |_| 1.0);
    // The dragon itself (its body), then the parts within 12 blocks.
    let parts: Vec<usize> = open.iter().map(|h| h.0).collect();
    assert_eq!(parts, vec![2, 0, 1, 2, 3]);
    // At the centre: (1 + 1) / 2 * 7 * 12 + 1.
    assert_eq!(open[0].1, 85.0);
    // Behind cover (no ray reaches the box) only the 1 is left.
    let covered = d.explosion_hits(center, 6.0, |_| 0.0);
    assert!(covered.iter().all(|h| h.1 == 1.0));
    // Through the cooldown the strongest hit after the part's cut wins.
    for (part, amount) in open {
        d.hurt(level, part, amount, HIT, false);
    }
    assert_eq!(d.health, MAX_HEALTH - 85.0);
}

fn player(who: FightWho, x: f64, creative: bool, spectator: bool) -> FightPlayer {
    let position = Vec3::new(x, 64.0, 0.0);
    FightPlayer {
        who,
        position,
        bounds: Aabb::new(position, Vec3::new(x + 0.6, 65.8, 0.6)),
        alive: true,
        creative,
        spectator,
    }
}

#[test]
fn the_first_crystal_destroyed_in_a_tick_names_the_strafed_player() {
    // DragonHoldingPatternPhase.onCrystalDestroyed strafes at once, and
    // the strafing phase ignores the next crystal.
    let (host, guest) = (
        player(FightWho::Host, 1.0, false, false),
        player(FightWho::Guest(8), 2.0, false, false),
    );
    let mut d = dragon(flying());
    d.strafe_after_crystal(Some(&host));
    d.strafe_after_crystal(Some(&guest));
    assert_eq!(d.strafe_request, Some(FightWho::Host));
    // Nobody, or a player it can't attack, leaves the next crystal free.
    let creative = player(FightWho::Host, 1.0, true, false);
    let mut d = dragon(flying());
    d.strafe_after_crystal(None);
    d.strafe_after_crystal(Some(&creative));
    d.strafe_after_crystal(Some(&guest));
    assert_eq!(d.strafe_request, Some(FightWho::Guest(8)));
    // Only while it holds its pattern.
    let mut d = dragon(Phase::Scanning { time: 0 });
    d.strafe_after_crystal(Some(&guest));
    assert_eq!(d.strafe_request, None);
}

#[test]
fn the_dragon_targets_any_player_it_can_attack() {
    let at = Vec3::new(0.0, 64.0, 0.0);
    // A spectating host and a creative guest are never targets: the
    // survival guest further away is.
    let players = [
        player(FightWho::Host, 1.0, false, true),
        player(FightWho::Guest(7), 2.0, true, false),
        player(FightWho::Guest(8), 30.0, false, false),
    ];
    let t = nearest_enemy(&players, false, at, None, |_| true);
    assert_eq!(t.map(|p| p.who), Some(FightWho::Guest(8)));
    // Out of a 20-block range, or filtered out, or on Peaceful: none.
    assert!(nearest_enemy(&players, false, at, Some((at, 20.0)), |_| true).is_none());
    assert!(nearest_enemy(&players, false, at, None, |p| p.position.x < 10.0).is_none());
    assert!(nearest_enemy(&players, true, at, None, |_| true).is_none());
    // The nearest of several, measured from where it looks (the egg).
    let players = [
        player(FightWho::Host, 5.0, false, false),
        player(FightWho::Guest(8), -3.0, false, false),
    ];
    let t = nearest_enemy(&players, false, at, None, |_| true);
    assert_eq!(t.map(|p| p.who), Some(FightWho::Guest(8)));
    let t = nearest_enemy(&players, false, Vec3::new(9.0, 64.0, 0.0), None, |_| true);
    assert_eq!(t.map(|p| p.who), Some(FightWho::Host));
}
#[test]
fn a_landed_hurt_sets_the_dragon_hurt_flash_time() {
    let mut test_level = horizon_client::health::TestLevel::new(0);
    let level = &mut test_level.level();
    let mut d = dragon_as(42);
    assert_eq!(d.hurt_time, 0);
    let hurt = d.hurt(level, 0, 5.0, HIT, false);
    assert_eq!(hurt, Hurt::Landed { full: true });
    assert_eq!(d.hurt_time, 10);
}

#[test]
#[ignore = "daily"]
fn the_tracked_dragon_lists_its_data_defaults() {
    use horizon_protocol::entity_data::serializer;
    super::super::native_entity_refs::tests::with_game(|game| {
        let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
        dragon.id = 15;
        game.dragon.replace(dragon);
        let state = game.lan_dragon_state().unwrap();
        assert_eq!(
            state.defaults,
            vec![
                (9, serializer::FLOAT, Value::Float(1.0)),
                (15, serializer::BYTE, Value::Byte(0)),
                (16, serializer::INT, Value::Int(10)),
            ]
        );
    });
}
