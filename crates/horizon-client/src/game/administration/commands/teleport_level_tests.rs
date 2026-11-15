//! A mod's `teleport_to_level` into another level, kind by kind
//! (`Entity.teleportCrossDimension`): what each kind keeps, and a kind it cannot move.
use crate::game::native_entity_refs::tests::with_game;
use crate::game::{Dim, Game};
use horizon_core::geometry::Vec3;

/// A mod's `teleport_to_level` into another level carries an item and an arrow there as
/// vanilla's `Entity.teleportCrossDimension` does: recreated with their stack, thrower, owner
/// and motion (`Relative.DELTA`), gone from the source, and the old handle refused as a
/// removed entity's is (`Entity.teleport` returns null when `isRemoved`).
#[test]
#[ignore = "daily"]
fn a_mod_sends_an_item_and_an_arrow_to_another_level_with_their_motion() {
    with_game(|game| {
        game.native_lifecycle.world_open_ready().unwrap();
        game.summon_with_nbt(
            "minecraft:item",
            [0.5, 80.0, 0.5],
            r#"{Item:{id:"minecraft:diamond",count:7},Thrower:[I;1,2,3,4],Motion:[0.1d,0.2d,-0.3d]}"#,
        )
        .unwrap();
        game.summon_with_nbt(
            "minecraft:arrow",
            [2.5, 80.0, 0.5],
            "{Owner:[I;5,6,7,8],Motion:[0.5d,0.0d,0.25d]}",
        )
        .unwrap();
        game.lan_assign_ids();
        let thrower = Some([1, 2, 3, 4]);
        let owner = Some([5, 6, 7, 8]);
        let item = game
            .dropped
            .iter()
            .find(|e| e.thrower == thrower)
            .unwrap()
            .id;
        let arrow = game
            .creatures
            .projectiles
            .iter()
            .find(|p| p.owner_uuid == owner)
            .unwrap()
            .id;
        let (item, arrow) = (game.entity(item).unwrap(), game.entity(arrow).unwrap());
        let nether = "minecraft:the_nether";
        assert!(game.entity_teleport_to_level(item, nether, [3.5, 70.0, -2.5], None));
        assert!(game.entity_teleport_to_level(arrow, nether, [4.5, 71.0, -2.5], None));
        assert_eq!(game.dim, Dim::Overworld);
        assert!(!game.dropped.iter().any(|e| e.thrower == thrower));
        assert!(
            !game
                .creatures
                .projectiles
                .iter()
                .any(|p| p.owner_uuid == owner)
        );
        assert!(!game.entity_teleport_to_level(item, nether, [0.5, 70.0, 0.5], None));

        game.enter_level(Dim::Nether).unwrap();
        let e = game.dropped.iter().find(|e| e.thrower == thrower).unwrap();
        assert_eq!((&*e.stack.item, e.stack.count), ("minecraft:diamond", 7));
        assert_eq!(e.position, Vec3::new(3.5, 70.0, -2.5));
        assert_eq!(e.velocity, Vec3::new(0.1, 0.2, -0.3));
        let p = game
            .creatures
            .projectiles
            .iter()
            .find(|p| p.owner_uuid == owner)
            .unwrap();
        assert_eq!(p.position, Vec3::new(4.5, 71.0, -2.5));
        assert_eq!(p.velocity, Vec3::new(0.5, 0.0, 0.25));
    });
}

/// A mod's minecart keeps its motion into another level as `PositionMoveRotation.of(this)`
/// has it: `OldMinecartBehavior.getKnownMovement`, 0.4 at most along x and z.
#[test]
#[ignore = "daily"]
fn a_mod_sends_a_minecart_to_another_level_with_its_known_movement() {
    with_game(|game| {
        game.native_lifecycle.world_open_ready().unwrap();
        game.summon_with_nbt(
            "minecraft:minecart",
            [0.5, 80.0, 0.5],
            "{Motion:[0.3d,0.1d,-0.6d]}",
        )
        .unwrap();
        game.lan_assign_ids();
        let cart = game.entity(game.carts[0].id).unwrap();
        assert!(game.entity_teleport_to_level(
            cart,
            "minecraft:the_nether",
            [3.5, 70.0, -2.5],
            None
        ));
        assert!(game.carts.is_empty());
        game.enter_level(Dim::Nether).unwrap();
        assert_eq!(game.carts.len(), 1);
        assert_eq!(game.carts[0].velocity, Vec3::new(0.3, 0.1, -0.4));
    });
}

/// Summons `kind` with `nbt` at 0.5 80 0.5 in the Overworld, where `id` finds it, and sends
/// it to 3.5 70 -2.5 in the Nether by a mod's `teleport_to_level`: the call is true and the
/// Overworld holds it no more. The Nether is left in place.
fn sent_to_the_nether(
    game: &mut Game<'_>,
    kind: &str,
    nbt: &str,
    id: impl Fn(&Game<'_>) -> Option<u32>,
) {
    game.native_lifecycle.world_open_ready().unwrap();
    game.summon_with_nbt(kind, [0.5, 80.0, 0.5], nbt).unwrap();
    game.lan_assign_ids();
    let entity = game.entity(id(game).expect("summoned")).unwrap();
    assert!(game.entity_teleport_to_level(entity, "minecraft:the_nether", [3.5, 70.0, -2.5], None));
    assert!(id(game).is_none(), "the Overworld still holds it");
    game.enter_level(Dim::Nether).unwrap();
}

/// A primed TNT is recreated in the other level with its fuse and motion
/// (`PrimedTnt.restoreFrom`, `Relative.DELTA`).
#[test]
#[ignore = "daily"]
fn a_mod_sends_a_primed_tnt_to_another_level_with_its_fuse_and_motion() {
    with_game(|game| {
        sent_to_the_nether(
            game,
            "minecraft:tnt",
            "{fuse:200s,Motion:[0.1d,0.2d,-0.3d]}",
            |g| g.tnts.first().map(|t| t.id),
        );
        assert_eq!(game.tnts.len(), 1);
        let tnt = &game.tnts[0];
        assert_eq!(tnt.fuse, 200);
        assert_eq!(tnt.position, Vec3::new(3.5, 70.0, -2.5));
        assert_eq!(tnt.velocity, Vec3::new(0.1, 0.2, -0.3));
    });
}

/// An experience orb is recreated in the other level with its value, count and motion.
#[test]
#[ignore = "daily"]
fn a_mod_sends_an_experience_orb_to_another_level_with_its_value_and_motion() {
    with_game(|game| {
        sent_to_the_nether(
            game,
            "minecraft:experience_orb",
            "{Value:7s,Count:2,Motion:[0.1d,0.2d,-0.3d]}",
            |g| g.orbs.first().map(|o| o.id),
        );
        assert_eq!(game.orbs.len(), 1);
        let orb = &game.orbs[0];
        assert_eq!((orb.value, orb.count), (7, 2));
        assert_eq!(orb.position, Vec3::new(3.5, 70.0, -2.5));
        assert_eq!(orb.velocity, Vec3::new(0.1, 0.2, -0.3));
    });
}

/// A firework rocket is recreated in the other level with its stack, owner and motion
/// (`Projectile.restoreFrom`).
#[test]
#[ignore = "daily"]
fn a_mod_sends_a_firework_to_another_level_with_its_owner_and_motion() {
    with_game(|game| {
        sent_to_the_nether(
            game,
            "minecraft:firework_rocket",
            r#"{LifeTime:40,Owner:[I;5,6,7,8],Motion:[0.1d,0.2d,-0.3d],FireworksItem:{id:"minecraft:firework_rocket",count:1}}"#,
            |g| g.rockets.first().map(|r| r.id),
        );
        assert_eq!(game.rockets.len(), 1);
        let rocket = &game.rockets[0];
        assert_eq!(&*rocket.item.item, "minecraft:firework_rocket");
        assert_eq!(rocket.owner_uuid, Some([5, 6, 7, 8]));
        assert_eq!(rocket.position, Vec3::new(3.5, 70.0, -2.5));
        assert_eq!(rocket.velocity, Vec3::new(0.1, 0.2, -0.3));
    });
}

/// A thrown trident is recreated in the other level with its stack and motion.
#[test]
#[ignore = "daily"]
fn a_mod_sends_a_trident_to_another_level_with_its_stack_and_motion() {
    with_game(|game| {
        sent_to_the_nether(
            game,
            "minecraft:trident",
            r#"{Motion:[0.1d,0.2d,-0.3d],item:{id:"minecraft:trident",count:1}}"#,
            |g| g.tridents.first().map(|t| t.id),
        );
        assert_eq!(game.tridents.len(), 1);
        let trident = &game.tridents[0];
        assert_eq!(&*trident.stack.item, "minecraft:trident");
        assert_eq!(trident.position, Vec3::new(3.5, 70.0, -2.5));
        assert_eq!(trident.velocity, Vec3::new(0.1, 0.2, -0.3));
    });
}

/// Armor stands use Entity.teleportCrossDimension, keeping their position on arrival.
#[test]
#[ignore = "daily"]
fn a_mod_sends_an_armor_stand_to_another_level() {
    with_game(|game| {
        game.native_lifecycle.world_open_ready().unwrap();
        game.summon_with_nbt("minecraft:armor_stand", [0.5, 80.0, 0.5], "{}")
            .unwrap();
        game.lan_assign_ids();
        let stand = game.entity(game.armor_stands[0].id).unwrap();
        assert!(game.entity_teleport_to_level(
            stand,
            "minecraft:the_nether",
            [3.5, 70.0, -2.5],
            None
        ));
        assert!(game.armor_stands.is_empty());
        game.enter_level(Dim::Nether).unwrap();
        assert_eq!(game.armor_stands.len(), 1);
        assert_eq!(game.armor_stands[0].position, Vec3::new(3.5, 70.0, -2.5));
    });
}

#[test]
#[ignore = "daily"]
fn portal_arrival_effects_mod_items_merge_immediately() {
    with_game(|game| {
        game.native_lifecycle.world_open_ready().unwrap();
        game.spawn_item_own(
            horizon_client::items::ItemStack::new("minecraft:diamond", 3),
            [0.5, 80.0, 0.5],
            10,
        );
        game.dropped[0].age = 7;
        game.dropped[0].velocity = Vec3::new(0.1, 0.2, -0.3);
        let item = game.entity(game.dropped[0].id).unwrap();
        game.enter_level(Dim::Nether).unwrap();
        game.spawn_item_own(
            horizon_client::items::ItemStack::new("minecraft:diamond", 5),
            [3.5, 70.0, -2.5],
            10,
        );
        game.dropped[0].age = 41;
        let waiting = game.dropped[0].id;
        game.enter_level(Dim::Overworld).unwrap();
        assert!(game.entity_teleport_to_level(
            item,
            "minecraft:the_nether",
            [3.5, 70.0, -2.5],
            None
        ));
        assert!(game.dropped.is_empty());
        game.enter_level(Dim::Nether).unwrap();
        assert_eq!(game.dropped.len(), 1);
        let survivor = &game.dropped[0];
        assert_eq!(survivor.id, waiting);
        assert_eq!(
            (survivor.stack.count, survivor.age, survivor.ticks),
            (8, 7, 0)
        );
    });
}

#[test]
#[ignore = "daily"]
fn portal_arrival_effects_explicit_tnt_teleports_protect_portals() {
    for command in [false, true] {
        with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            game.native_lifecycle.world_open_ready().unwrap();
            game.summon_with_nbt(
                "minecraft:tnt",
                [0.5, 80.0, 0.5],
                "{fuse:200s,NoGravity:1b}",
            )
            .unwrap();
            let old = game.tnts[0].id;
            assert!(!game.tnts[0].used_portal);
            if command {
                assert!(game.run_command(
                    "execute in minecraft:the_nether run tp @e[type=minecraft:tnt] 8.5 70 8.5"
                ));
            } else {
                let entity = game.entity(old).unwrap();
                assert!(game.entity_teleport_to_level(
                    entity,
                    "minecraft:the_nether",
                    [8.5, 70.0, 8.5],
                    None,
                ));
            }
            assert!(game.tnts.is_empty());
            game.enter_level(Dim::Nether).unwrap();
            assert_eq!(game.tnts.len(), 1);
            assert_ne!(game.tnts[0].id, old);
            assert!(game.tnts[0].used_portal);
            crate::game::nether_portal::arrival_effects_tests::assert_tnt_blast(game, true);
            game.summon_with_nbt(
                "minecraft:tnt",
                [8.5, 70.0, 8.5],
                "{fuse:200s,NoGravity:1b}",
            )
            .unwrap();
            assert!(!game.tnts[0].used_portal);
            crate::game::nether_portal::arrival_effects_tests::assert_tnt_blast(game, false);
        });
    }
}
