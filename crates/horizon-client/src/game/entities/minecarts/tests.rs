// Test module extracted without changing its logical path.
/// `NewMinecartBehavior` caps the first track step with
/// `AbstractMinecart.getMaxSpeed`, which `MinecartFurnace` halves, and
/// `comeOffTrack` clamps with it too.
#[test]
#[ignore = "daily"]
fn a_furnace_cart_is_capped_at_half_the_rule_speed_under_the_new_physics() {
    super::super::native_entity_refs::tests::with_game(|game| {
        let mut features = game.level.enabled_features.to_vec();
        features.push("minecraft:minecart_improvements".into());
        game.level.enabled_features = features.into();
        game.ui.rules =
            horizon_client::gamerules::GameRules::with_features(&game.level.enabled_features);
        let rule = f64::from(
            game.ui
                .rules
                .int_at(horizon_client::gamerules::vanilla::MAX_MINECART_SPEED),
        ) / 20.0;
        assert_eq!(rule, 0.4);
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
        );
        let rail = game
            .pack
            .assets
            .registry
            .default_state("minecraft:rail")
            .unwrap();
        for z in 0..16 {
            game.world.set(0, 64, z, rail).unwrap();
        }
        let speed_after = |game: &mut Game<'_>, kind: CartKind, on_rail: bool| {
            let at = if on_rail {
                Vec3::new(0.5, 64.0625, 2.5)
            } else {
                Vec3::new(8.5, 80.0, 8.5)
            };
            let mut cart = Minecart::new(1, kind, at);
            cart.velocity = Vec3::new(0.0, 0.0, 1.0);
            cart.first_tick = false;
            game.tick_cart(&mut cart, [0.0; 2], 0, &mut CartIndex::default());
            cart.velocity.horizontal_distance()
        };
        // On a rail, the first iteration's cap.
        let furnace = speed_after(game, CartKind::Furnace, true);
        let plain = speed_after(game, CartKind::Rideable, true);
        assert!(furnace <= 0.2 + 1e-9, "{furnace}");
        assert!(plain > 0.2 + 1e-9 && plain <= 0.4 + 1e-9, "{plain}");
        // Off the rails, comeOffTrack's clamp (and the same cut in water).
        let furnace = speed_after(game, CartKind::Furnace, false);
        assert!(furnace <= 0.2 + 1e-9, "{furnace}");
        let plain = speed_after(game, CartKind::Rideable, false);
        assert!(plain > 0.2 + 1e-9, "{plain}");
    });
}

/// The cart index hands a moved cart every cart a scan finds in its box,
/// as the carts move one by one, after one is gone, and after a command
/// (which forgets the index) put another cart in a slot.
#[test]
fn cart_index_finds_what_a_scan_finds() {
    use super::{CartIndex, CartKind, Minecart};
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = |m: f64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % 1_000_000) as f64 / 1_000_000.0 * m
    };
    // Clustered, so most carts have neighbours.
    let mut carts: Vec<Minecart> = (0..300)
        .map(|id| Minecart::new(id, CartKind::Chest, Vec3::new(next(40.0), 64.0, next(40.0))))
        .collect();
    let mut index = CartIndex::default();
    let mut near = Vec::new();
    let check = |index: &mut CartIndex, carts: &[Minecart], i: usize, near: &mut Vec<u32>| {
        let area = carts[i].bounds().inflate(Vec3::new(0.2, 0.0, 0.2));
        let scan: Vec<u32> = (0..carts.len() as u32)
            .filter(|&j| area.intersects(carts[j as usize].bounds()))
            .collect();
        if index.carts_near(carts, i, area, near) {
            let found: Vec<u32> = near
                .iter()
                .copied()
                .filter(|&j| area.intersects(carts[j as usize].bounds()))
                .collect();
            assert_eq!(found, scan, "cart {i}");
        }
    };
    for round in 0..3 {
        for i in 0..carts.len() {
            // Each cart moves up to two blocks before its push.
            carts[i].position.x += next(4.0) - 2.0;
            carts[i].position.z += next(4.0) - 2.0;
            check(&mut index, &carts, i, &mut near);
            index.moved(i, carts[i].bounds());
            if i == 100 && round == 1 {
                carts.remove(50);
            }
            if i == 200 && round == 2 {
                let at = carts[0].position;
                carts[150] = Minecart::new(999, CartKind::Rideable, at);
                index.forget_carts();
            }
            if i >= carts.len() - 1 {
                break;
            }
        }
    }
    assert!(index.carts.is_some(), "the index was used");
}
/// The cart index hands a cart every mob a scan finds in its box, and
/// knows whether a mob rides it, however the mob list changed since.
#[test]
#[ignore = "daily"]
fn cart_index_finds_the_mobs_a_scan_finds() {
    use super::{CartIndex, CartKind, Minecart};
    use horizon_client::mobs::Mob;
    crate::game::native_entity_refs::tests::with_game(|game| {
        let kinds: Vec<_> = ["minecraft:zombie", "minecraft:chicken", "minecraft:ravager"]
            .iter()
            .map(|k| game.res.mob_types().get(*k).cloned().unwrap())
            .collect();
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = |m: f64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 1_000_000) as f64 / 1_000_000.0 * m
        };
        for id in 0..200u32 {
            let at = Vec3::new(next(40.0), 64.0, next(40.0));
            let kind = kinds[id as usize % kinds.len()].clone();
            game.mobs.push(Mob::new(id + 1, kind, at, 0.0));
        }
        let carts: Vec<Minecart> = (0..60)
            .map(|id| {
                Minecart::new(
                    1000 + id,
                    CartKind::Rideable,
                    Vec3::new(next(40.0), 64.0, next(40.0)),
                )
            })
            .collect();
        let mut index = CartIndex::default();
        let mut near = Vec::new();
        let mut check = |game: &Game<'_>, what: &str| {
            for c in &carts {
                let area = c.bounds().inflate(Vec3::new(0.2, 0.0, 0.2));
                let scan: Vec<u32> = (0..game.mobs.len() as u32)
                    .filter(|&j| area.intersects(game.mobs[j as usize].bounds()))
                    .collect();
                let ridden = game.mobs.iter().any(|m| m.extra.mount == Some(c.id));
                let mounted = index.mobs_near(&game.mobs, c.id, area, &mut near);
                assert_eq!(mounted, Some(ridden), "{what}: cart {}", c.id);
                near.retain(|&j| area.intersects(game.mobs[j as usize].bounds()));
                assert_eq!(near, scan, "{what}: cart {}", c.id);
            }
        };
        check(game, "as placed");
        game.mobs[10].position = carts[3].position;
        game.mobs[11].extra.mount = Some(carts[4].id);
        check(game, "one moved, one boarded");
        let kind = kinds[0].clone();
        game.mobs
            .insert(0, Mob::new(500, kind, carts[5].position, 0.0));
        game.mobs.retain(|m| m.id % 7 != 0);
        check(game, "added and removed");
    });
}
#[test]
fn mob_walking_into_a_cart_pushes_it_by_a_twentieth() {
    // Issue #6776: a villager at x=0.13 next to a cart at x=0.5 east-west.
    let (xa, za) =
        super::cart_push_from(Vec3::new(0.13, 0.0, 0.0), Vec3::new(0.5, 0.0, 0.0)).unwrap();
    assert!((xa + 0.05).abs() < 1e-6 && za == 0.0, "{xa} {za}");
    assert!(super::cart_push_from(Vec3::ZERO, Vec3::ZERO).is_none());
}
/// Issue #6431: a blast too weak to push a cart past 40 damage still
/// primes a TNT cart.
#[test]
fn a_blast_ignites_a_tnt_cart_however_little_damage_it_does() {
    assert!(cart_is_destroyed(false, 10.0, true));
    assert!(!cart_is_destroyed(false, 10.0, false));
    assert!(cart_is_destroyed(false, 40.5, false));
    assert!(!cart_is_destroyed(false, 40.0, false));
}
/// The collision detonation of a TNT minecart needs both the collision and
/// a cart that was still moving: a nudge below a tenth of a block a tick
/// does not set it off, and a fast cart on an open track does not either.
#[test]
fn a_tnt_cart_detonates_only_when_a_collision_stops_a_moving_one() {
    assert!(tnt_ignites_by_collision(true, 0.01));
    assert!(tnt_ignites_by_collision(true, 0.36), "three tenths a tick");
    assert!(!tnt_ignites_by_collision(true, 0.0), "stopped at the wall");
    assert!(!tnt_ignites_by_collision(true, 0.009), "a nudge");
    assert!(!tnt_ignites_by_collision(false, 0.36), "nothing in the way");
}
use super::*;

/// Issue #8624: chest and hopper carts keep 0.995 of their speed when
/// empty (0.98 when full), not the plain cart's 0.96, so powered rails
/// carry them much further.
#[test]
fn container_carts_keep_more_speed_than_the_plain_cart() {
    assert!((container_keep(0, false, false) - 0.995).abs() < 1e-6);
    assert!((container_keep(15, false, false) - 0.98).abs() < 1e-6);
    assert!((container_keep(5, true, false) - 0.98).abs() < 1e-6);
    assert!((container_keep(0, false, true) - 0.995 * 0.95).abs() < 1e-6);
}

#[test]
fn a_tnt_cart_lets_the_held_item_be_used() {
    // Entity.interact passes: a crossbow aimed at it shoots.
    assert!(CartKind::Tnt.interact_passes(false, false, true));
    assert!(CartKind::CommandBlock.interact_passes(false, false, false));
    assert!(!CartKind::CommandBlock.interact_passes(false, false, true));
    assert!(!CartKind::Rideable.interact_passes(false, false, false));
    assert!(CartKind::Rideable.interact_passes(true, false, false));
    assert!(CartKind::Rideable.interact_passes(false, true, false));
    for kind in [CartKind::Chest, CartKind::Hopper, CartKind::Furnace] {
        assert!(!kind.interact_passes(true, true, false));
    }
}

use horizon_core::nbt::Tag;

fn saved_cart(kind: CartKind, extra: &[(&str, Tag)]) -> Tag {
    use horizon_core::nbt::JavaString;
    let mut tag = Minecart::new(1, kind, Vec3::ZERO).to_nbt([0; 4], None);
    if let Tag::Compound(map) = &mut tag {
        for (k, v) in extra {
            map.insert(JavaString::from(*k), v.clone());
        }
    }
    tag
}

/// AbstractMinecart.read/addAdditionalSaveData: `DisplayState` and
/// `DisplayOffset` survive a save, the offset only off its default.
#[test]
fn custom_display_state_and_offset_roundtrip() {
    // BlockState.CODEC for a non-default state: {Name, Properties}.
    let state = Tag::Compound(
        [
            ("Name".into(), Tag::String("minecraft:furnace".into())),
            (
                "Properties".into(),
                Tag::Compound(
                    [("lit".into(), Tag::String("true".into()))]
                        .into_iter()
                        .collect(),
                ),
            ),
        ]
        .into_iter()
        .collect(),
    );
    let saved = saved_cart(
        CartKind::Rideable,
        &[
            ("DisplayState", state.clone()),
            ("DisplayOffset", Tag::Int(10)),
        ],
    );
    let cart = Minecart::from_nbt(&saved, 1, CartKind::Rideable).unwrap();
    assert_eq!(cart.custom_display, Some(state.clone()));
    assert_eq!(display_offset(&cart), 10.0);
    let out = cart.to_nbt([0; 4], None);
    assert_eq!(out.get("DisplayState"), Some(&state));
    assert_eq!(out.get("DisplayOffset"), Some(&Tag::Int(10)));
}

/// `/data merge entity` on a cart: `EntityDataAccessor.setData` loads the
/// merged compound into the live cart (`Entity.load`). Its display state,
/// offset and name read back; its id, kind and unsaved state stay.
#[test]
fn data_merge_loads_into_a_live_cart() {
    let mut cart = Minecart::new(7, CartKind::Chest, Vec3::new(1.5, 64.0, 2.5));
    cart.hurt_time = 5;
    let old = cart.to_nbt([0; 4], None);
    let (patch, _) = horizon_core::nbt::parse_snbt(
            r#"{DisplayState:"minecraft:furnace",DisplayOffset:3,CustomName:"Cart",Pos:[1.5d,70.0d,2.5d]}"#,
        )
        .unwrap();
    let merged = horizon_client::data_cmd::merged(&old, &patch);
    cart.load(&merged).unwrap();
    assert_eq!(
        (cart.id, cart.kind, cart.hurt_time),
        (7, CartKind::Chest, 5)
    );
    assert_eq!(cart.position, Vec3::new(1.5, 70.0, 2.5));
    assert_eq!(display_offset(&cart), 3.0);
    let out = cart.to_nbt([0; 4], Some(&merged));
    assert_eq!(out.get("DisplayState"), patch.get("DisplayState"));
    assert_eq!(out.get("DisplayOffset"), Some(&Tag::Int(3)));
    assert_eq!(out.get("CustomName"), Some(&Tag::String("Cart".into())));
    // A compound without a position loads nothing.
    let mut no_pos = merged.clone();
    if let Tag::Compound(map) = &mut no_pos {
        map.remove(&"Pos".into());
        map.insert("DisplayOffset".into(), Tag::Int(9));
    }
    assert!(cart.load(&no_pos).is_none());
    assert_eq!(display_offset(&cart), 3.0);
}

/// A merge of an unrelated key loads the cart's own saved state back
/// unchanged: a furnace cart keeps its fuel and push, a TNT cart its fuse,
/// a hopper cart its Enabled flag.
#[test]
fn data_merge_of_a_name_keeps_the_carts_state() {
    let mut furnace = Minecart::new(1, CartKind::Furnace, Vec3::ZERO);
    (furnace.fuel, furnace.push) = (1200, Vec3::new(0.5, 0.0, -0.25));
    let mut tnt = Minecart::new(2, CartKind::Tnt, Vec3::ZERO);
    tnt.fuse = 40;
    let mut hopper = Minecart::new(3, CartKind::Hopper, Vec3::ZERO);
    hopper.enabled = false;
    let (patch, _) = horizon_core::nbt::parse_snbt(r#"{CustomName:"x"}"#).unwrap();
    for cart in [furnace, tnt, hopper] {
        let mut merged_cart = cart.clone();
        let merged = horizon_client::data_cmd::merged(&cart.to_nbt([0; 4], None), &patch);
        merged_cart.load(&merged).unwrap();
        assert_eq!(
            (
                merged_cart.fuel,
                merged_cart.push,
                merged_cart.fuse,
                merged_cart.enabled
            ),
            (cart.fuel, cart.push, cart.fuse, cart.enabled),
            "{:?}",
            cart.kind
        );
    }
}

#[test]
fn air_display_state_shows_nothing() {
    let (air, stone) = (BlockStateId(0), BlockStateId(1));
    assert_eq!(shown_unless_air(Some(air), |s| s == air), None);
    assert_eq!(shown_unless_air(Some(stone), |s| s == air), Some(stone));
    assert_eq!(shown_unless_air(None, |s| s == air), None);
}

#[test]
fn an_unreadable_display_state_is_dropped_and_not_saved() {
    let bad = Tag::String("mod:gone".into());
    let saved = saved_cart(CartKind::Rideable, &[("DisplayState", bad)]);
    let mut cart = Minecart::from_nbt(&saved, 1, CartKind::Rideable).unwrap();
    forget_unreadable_display(&mut cart, |_| false);
    assert!(cart.custom_display.is_none());
    assert!(
        cart.to_nbt([0; 4], Some(&saved))
            .get("DisplayState")
            .is_none()
    );
    // A readable one is kept.
    let mut cart = Minecart::from_nbt(&saved, 1, CartKind::Rideable).unwrap();
    forget_unreadable_display(&mut cart, |_| true);
    assert!(cart.custom_display.is_some());
}

#[test]
fn display_offset_is_not_saved_at_its_default() {
    // A hopper cart's default is 1, a chest cart's 8, any other kind's 6.
    for (kind, default) in [
        (CartKind::Hopper, 1),
        (CartKind::Chest, 8),
        (CartKind::Rideable, 6),
    ] {
        let saved = saved_cart(kind, &[("DisplayOffset", Tag::Int(default))]);
        let cart = Minecart::from_nbt(&saved, 1, kind).unwrap();
        assert_eq!(display_offset(&cart), f64::from(default));
        let out = cart.to_nbt([0; 4], Some(&saved));
        assert!(out.get("DisplayOffset").is_none(), "{kind:?}");
        assert!(out.get("DisplayState").is_none(), "{kind:?}");
    }
    // A hopper cart saved with 6 keeps it: that is not its default.
    let saved = saved_cart(CartKind::Hopper, &[("DisplayOffset", Tag::Int(6))]);
    let cart = Minecart::from_nbt(&saved, 1, CartKind::Hopper).unwrap();
    assert_eq!(
        cart.to_nbt([0; 4], None).get("DisplayOffset"),
        Some(&Tag::Int(6))
    );
    // And a chest cart saved with 6, the base default, keeps it too.
    let saved = saved_cart(CartKind::Chest, &[("DisplayOffset", Tag::Int(6))]);
    let cart = Minecart::from_nbt(&saved, 1, CartKind::Chest).unwrap();
    assert_eq!(display_offset(&cart), 6.0);
    assert_eq!(
        cart.to_nbt([0; 4], None).get("DisplayOffset"),
        Some(&Tag::Int(6))
    );
}

#[test]
fn display_offset_matches_vanilla_for_each_cart_kind() {
    // MinecartChest and MinecartHopper override getDefaultDisplayOffset;
    // the rest keep AbstractMinecart's 6.
    assert_eq!(default_display_offset(CartKind::Chest), 8.0);
    assert_eq!(default_display_offset(CartKind::Hopper), 1.0);
    assert_eq!(default_display_offset(CartKind::Furnace), 6.0);
    assert_eq!(default_display_offset(CartKind::Tnt), 6.0);
    assert_eq!(default_display_offset(CartKind::CommandBlock), 6.0);
    assert_eq!(default_display_offset(CartKind::Spawner), 6.0);
    assert_eq!(default_display_offset(CartKind::Rideable), 6.0);
    // A cart without a saved offset draws at its kind's default.
    let chest = Minecart::new(1, CartKind::Chest, Vec3::ZERO);
    assert_eq!(display_offset(&chest), 8.0);
}

/// `MinecartSpawner`: its `BaseSpawner` is saved and loaded with the cart (the delay, the
/// ranges and the `SpawnData`), it drops and is picked as a plain minecart, and it shows a
/// spawner block.
#[test]
fn a_spawner_cart_keeps_its_spawner_and_drops_a_minecart() {
    use horizon_core::nbt::Tag;
    let saved = Tag::compound([
        ("id", Tag::String("minecraft:spawner_minecart".into())),
        (
            "Pos",
            Tag::List(
                [0.5f64, 40.5, 0.5]
                    .map(|v| Tag::Double(v.to_bits()))
                    .to_vec(),
            ),
        ),
        ("Delay", Tag::Short(37)),
        ("MaxSpawnDelay", Tag::Short(120)),
        ("SpawnRange", Tag::Short(2)),
        (
            "SpawnData",
            Tag::compound([(
                "entity",
                Tag::compound([("id", Tag::String("minecraft:zombie".into()))]),
            )]),
        ),
    ]);
    let mut cart = Minecart::from_nbt(&saved, 1, CartKind::Spawner).unwrap();
    let spawner = cart.spawner.as_mut().expect("a spawner cart has a spawner");
    assert_eq!(
        (
            spawner.spawn_delay,
            spawner.max_spawn_delay,
            spawner.spawn_range
        ),
        (37, 120, 2)
    );
    // No raw tag to copy from: what comes out is the cart's own save.
    spawner.spawn_delay = 41;
    let out = cart.to_nbt([0; 4], None);
    assert_eq!(out.get("Delay"), Some(&Tag::Short(41)));
    assert_eq!(out.get("MaxSpawnDelay"), Some(&Tag::Short(120)));
    assert_eq!(
        out.get("SpawnData")
            .and_then(|d| d.get("entity"))
            .and_then(|e| e.get("id")),
        Some(&Tag::String("minecraft:zombie".into()))
    );
    assert_eq!(CartKind::Spawner.item(), "minecraft:minecart");
    assert_eq!(
        CartKind::from_id("minecraft:spawner_minecart"),
        Some(CartKind::Spawner)
    );
    // A summoned one has a spawner of its own.
    assert!(
        Minecart::new(2, CartKind::Spawner, Vec3::ZERO)
            .spawner
            .is_some()
    );
}

#[test]
fn a_cow_boards_an_empty_cart_and_an_iron_golem_does_not() {
    assert!(pickup_instead_of_push(Species::Cow, false, false));
    assert!(can_ride_cart(Species::Cow, 0));
    assert!(!pickup_instead_of_push(Species::IronGolem, false, false));
    assert!(!pickup_instead_of_push(Species::Cow, false, true));
    assert!(!pickup_instead_of_push(Species::Cow, true, false));
    assert!(!can_ride_cart(Species::Cow, 60));
    assert!(!can_ride_cart(Species::Warden, 0));
}

/// `WitherBoss.canRide` returns false, so a moving cart cannot scoop the boss up.
#[test]
fn a_wither_never_rides_a_cart() {
    assert!(!can_ride_cart(Species::Wither, 0));
}

#[test]
fn a_cart_seat_matches_the_player_offset() {
    let at = Vec3::new(1.0, 10.0, 2.0);
    assert!((cart_seat(at, PASSENGER_Y).y - 10.1875).abs() < 1e-12);
    assert!((cart_seat(at, SEAT).y - (10.0 + SEAT)).abs() < 1e-12);
    assert_eq!(cart_seat(at, 0.0).x, 1.0);
    assert_eq!(cart_seat(at, 0.0).z, 2.0);
}

#[test]
fn a_cart_saves_as_never_ticked_until_it_ticks() {
    use horizon_core::nbt::Tag;
    let generated = Tag::compound([
        ("id", Tag::String("minecraft:chest_minecart".into())),
        (
            "Pos",
            Tag::List(
                [0.5f64, 40.5, 0.5]
                    .map(|v| Tag::Double(v.to_bits()))
                    .to_vec(),
            ),
        ),
        ("HasTicked", Tag::Byte(1)),
        ("Fire", Tag::Short(0)),
    ]);
    let mut cart = Minecart::from_nbt(&generated, 1, CartKind::Chest).unwrap();
    let saved = cart.to_nbt([0; 4], Some(&generated));
    assert_eq!(saved.get("HasTicked"), Some(&Tag::Byte(1)));
    assert_eq!(saved.get("Fire"), Some(&Tag::Short(0)));
    cart.first_tick = false;
    let saved = cart.to_nbt([0; 4], Some(&generated));
    assert_eq!(saved.get("HasTicked"), Some(&Tag::Byte(0)));
    assert_eq!(saved.get("Fire"), Some(&Tag::Short(0)));
}

#[test]
fn a_command_cart_saves_its_base_command_block() {
    use horizon_core::nbt::Tag;
    let saved = Tag::compound([
        (
            "Pos",
            Tag::List(
                [0.5f64, 40.5, 0.5]
                    .map(|v| Tag::Double(v.to_bits()))
                    .to_vec(),
            ),
        ),
        ("Command", Tag::String("say hi".into())),
        ("SuccessCount", Tag::Int(3)),
        ("TrackOutput", Tag::Byte(1)),
        ("LastOutput", Tag::String("[12:00:00] hi".into())),
        ("LastExecution", Tag::Long(42)),
    ]);
    let cart = Minecart::from_nbt(&saved, 1, CartKind::CommandBlock).unwrap();
    assert_eq!(cart.command.command, "say hi");
    assert_eq!(cart.command.success_count, 0);
    // readAdditionalSaveData syncs both values.
    assert_eq!(cart.synced_command, "say hi");
    assert_eq!(cart.synced_output, "[12:00:00] hi");
    let out = cart.to_nbt([0; 4], Some(&saved));
    assert_eq!(out.get("SuccessCount"), Some(&Tag::Int(0)));
    for key in ["Command", "TrackOutput", "LastOutput", "LastExecution"] {
        assert_eq!(out.get(key), saved.get(key), "{key}");
    }
    assert_eq!(out.get("UpdateLastExecution"), Some(&Tag::Byte(1)));
    // A block-only flag never reaches the entity.
    assert!(out.get("powered").is_none() && out.get("auto").is_none());
    // Untracked output drops the saved last output.
    let mut quiet = cart.clone();
    quiet.command.track_output = false;
    let out = quiet.to_nbt([0; 4], Some(&saved));
    assert!(out.get("LastOutput").is_none());
    assert_eq!(out.get("TrackOutput"), Some(&Tag::Byte(0)));
}

#[test]
fn furnace_push_keeps_its_length_under_gravity() {
    // Gravity gives the movement a vertical part. The push turns onto the
    // movement at the same 3D length every tick, as vanilla's does.
    let movement = Vec3::new(0.2, -0.04, 0.0);
    let mut push = Vec3::new(0.5, 0.0, 0.0);
    for _ in 0..1000 {
        push = push_along(push, movement);
    }
    assert!((push.length() - 0.5).abs() < 1e-12);
    assert!((push.x - 0.5 * 0.2 / movement.length()).abs() < 1e-12);
}

#[test]
fn a_furnace_saves_its_push_without_y() {
    use horizon_core::nbt::Tag;
    let generated = Tag::compound([
        ("id", Tag::String("minecraft:furnace_minecart".into())),
        (
            "Pos",
            Tag::List(
                [0.5f64, 40.5, 0.5]
                    .map(|v| Tag::Double(v.to_bits()))
                    .to_vec(),
            ),
        ),
    ]);
    let mut cart = Minecart::from_nbt(&generated, 1, CartKind::Furnace).unwrap();
    cart.push = Vec3::new(0.5, 0.7, 0.25);
    let saved = cart.to_nbt([0; 4], Some(&generated));
    assert_eq!(saved.get("PushX"), Some(&Tag::Double(0.5f64.to_bits())));
    assert_eq!(saved.get("PushZ"), Some(&Tag::Double(0.25f64.to_bits())));
    let loaded = Minecart::from_nbt(&saved, 1, CartKind::Furnace).unwrap();
    assert_eq!(loaded.push, Vec3::new(0.5, 0.0, 0.25));
}

/// A shulker beside the rail is picked up although `Mob::pushable` leaves it out.
#[test]
fn a_cart_picks_up_a_shulker_but_not_a_dead_one() {
    assert!(offered_pickup(false, Species::Shulker, true));
    assert!(!offered_pickup(false, Species::Shulker, false));
    assert!(!offered_pickup(false, Species::Bat, true));
    assert!(offered_pickup(true, Species::Cow, true));
    assert!(pickup_instead_of_push(Species::Shulker, false, false));
    assert!(can_ride_cart(Species::Shulker, 0));
}

/// A shulker already riding a boat is no pickup candidate and `Shulker.push` is a no-op.
#[test]
fn a_mounted_shulker_does_not_push_a_cart() {
    assert!(!pickup_instead_of_push(Species::Shulker, true, false));
    assert!(!takes_push_from_cart(Species::Shulker));
    assert!(takes_push_from_cart(Species::Cow));
}

/// A pass over loose carts, some of them TNT carts that blow up in the middle of it (the list shrinks
/// under the pass), plays the same with the boxes taken at the start of each tick as without them.
#[test]
#[ignore = "daily"]
fn a_pass_with_carts_blowing_up_plays_the_same_with_and_without_the_boxes_taken_at_the_start() {
    use super::near_carts::tests_support::{NEAR_CARTS_ANSWERS, NEAR_CARTS_OFF};
    // Opt in to the seed uniquifier only in a fresh child, as the other seeded tests do.
    if crate::game::native_entity_refs::tests::run_fresh_cases(
        "game::minecarts::tests::a_pass_with_carts_blowing_up_plays_the_same_with_and_without_the_boxes_taken_at_the_start",
        "HORIZON_TEST_SEEDS",
        &["1"],
    ) {
        return;
    }
    type Snapshot = Vec<(u32, [f64; 3], [f64; 3])>;
    fn play(off: bool) -> (Vec<Snapshot>, u32) {
        let mut ticks = Vec::new();
        let mut answers = 0;
        super::super::native_entity_refs::tests::with_game(|game| {
            horizon_client::random_source::set_thread_uniquifier(777).unwrap();
            NEAR_CARTS_OFF.set(off);
            NEAR_CARTS_ANSWERS.set(0);
            game.build_registries();
            game.set_allow_commands(true);
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            game.block_ticks
                .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
                .unwrap();
            assert!(game.run_command("forceload add 0 0"));
            for k in 0..40u32 {
                let at = horizon_core::geometry::Vec3::new(
                    2.0 + f64::from(k % 8) * 0.9,
                    80.0 + f64::from(k / 8) * 1.3,
                    2.0 + f64::from((k * 5) % 7) * 0.8,
                );
                let tnt = k % 9 == 4;
                let mut cart = super::Minecart::new(
                    k + 1,
                    if tnt {
                        super::CartKind::Tnt
                    } else {
                        super::CartKind::Rideable
                    },
                    at,
                );
                if tnt {
                    cart.fuse = 4 + (k % 5) as i32;
                }
                game.carts.push(cart);
            }
            for _ in 0..30 {
                game.tick_carts();
                ticks.push(
                    game.carts
                        .iter()
                        .map(|c| {
                            (
                                c.id,
                                [c.position.x, c.position.y, c.position.z],
                                [c.velocity.x, c.velocity.y, c.velocity.z],
                            )
                        })
                        .collect(),
                );
            }
            answers = NEAR_CARTS_ANSWERS.get();
        });
        NEAR_CARTS_OFF.set(false);
        (ticks, answers)
    }
    let (without, none) = play(true);
    let (with, taken) = play(false);
    assert_eq!(none, 0);
    assert!(
        taken > 100,
        "the boxes taken at the start answered {taken} times"
    );
    // The pass really changed under itself: carts went, and the rest moved.
    assert!(with.first().unwrap().len() > with.last().unwrap().len());
    assert_ne!(with.first(), with.last());
    assert_eq!(without, with);
}
