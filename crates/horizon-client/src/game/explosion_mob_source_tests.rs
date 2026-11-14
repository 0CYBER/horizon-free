//! `Explosion.getIndirectSourceEntity` for a blast a mob caused: a mob-owned
//! primed TNT spares vehicles without `mobGriefing`, and a mob's own blast
//! hands its kind and UUID to the TNT it primes.
use super::*;
use crate::game::boats::Boat;
use crate::game::native_machines::tests::server_game;
use horizon_client::gamerules::Value;
use horizon_client::mobs::{Mob, MobEvent, Species};
use horizon_world::nbt::Tag;

/// Blasts a boat 7.7 blocks away with `owner` as the indirect source, and
/// returns what the boat took: its damage and its push.
fn blast_boat(
    game: &mut Game<'_>,
    owner: Option<(u32, Text)>,
    player: bool,
    mob_griefing: bool,
) -> (f32, Vec3) {
    game.boats.clear();
    game.boats
        .push(Boat::new(1, "oak".into(), Vec3::new(16.2, 64.0, 8.5), 0.0));
    assert!(game.ui.rules.set("mob_griefing", Value::Bool(mob_griefing)));
    game.combat.blast_owner = owner;
    game.combat.blast_owner_player = player;
    game.server_explode([8.5, 64.0, 8.5], 4.0, BlockInteraction::Destroy, false)
        .unwrap();
    assert_eq!(game.boats.len(), 1, "the boat survives the blast");
    let boat = &game.boats[0];
    (boat.damage, boat.velocity)
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_mob_owned_blast_spares_vehicles_only_without_mob_griefing() {
    let mut game = server_game();
    game.build_registries();
    let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
    game.mobs
        .push(Mob::new(7, kind, Vec3::new(40.0, 64.0, 40.0), 0.0));
    let pig = || (7, Text::translate("entity.minecraft.pig", &[]));
    let steve = (1, Text::literal("Steve"));
    // VehicleEntity.ignoreExplosion: the indirect source is a mob and
    // mobGriefing is off, so the boat is neither hurt nor pushed.
    let (damage, velocity) = blast_boat(&mut game, Some(pig()), false, false);
    assert_eq!(damage, 0.0, "a mob's blast spares the boat");
    assert_eq!(velocity, Vec3::ZERO, "and does not push it");
    // The same blast with mobGriefing on still hits it.
    let (damage, velocity) = blast_boat(&mut game, Some(pig()), false, true);
    assert!(damage > 0.0, "mobGriefing on lets the boat be hurt");
    assert_ne!(velocity, Vec3::ZERO, "and pushed");
    // A player-owned TNT is not a mob: mobGriefing off does not spare it.
    let (damage, velocity) = blast_boat(&mut game, Some(steve), true, false);
    assert!(damage > 0.0, "a player's blast still hurts the boat");
    assert_ne!(velocity, Vec3::ZERO, "and pushes it");
    // PrimedTnt.getOwner no longer resolves a removed mob.
    game.mobs[0].removed = true;
    let (damage, _) = blast_boat(&mut game, Some(pig()), false, false);
    assert!(damage > 0.0, "a removed owner no longer spares the boat");
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_creeper_blast_primes_tnt_with_the_creeper_as_its_owner() {
    let mut game = server_game();
    game.build_registries();
    let creeper = 77;
    let kind = game
        .res
        .mob_types()
        .get("minecraft:creeper")
        .unwrap()
        .clone();
    game.mobs
        .push(Mob::new(creeper, kind, Vec3::new(8.5, 64.0, 8.5), 0.0));
    let uuid = game.uuid_for(creeper);
    let tnt = game
        .pack
        .assets
        .registry
        .default_state("minecraft:tnt")
        .unwrap();
    game.world.set(9, 64, 8, tnt).unwrap();
    let name = Text::translate("entity.minecraft.creeper", &[]);
    game.handle_mob_events(vec![
        MobEvent::ExplosionSource {
            mob: creeper,
            charged: false,
            name,
            in_water: false,
            kind: "minecraft:creeper".into(),
        },
        MobEvent::Explode([8.5, 64.0, 8.5], 3.0, Species::Creeper, false),
    ])
    .unwrap();
    assert_eq!(game.tnts.len(), 1, "the blast primed the TNT block");
    assert_eq!(
        game.tnts[0].owner,
        Some(Who::Mob(creeper)),
        "the chain TNT remembers the creeper"
    );
    // PrimedTnt.addAdditionalSaveData writes the owner's UUID.
    let saved = game.tnt_nbt(0);
    assert_eq!(saved.get("owner"), Some(&Tag::IntArray(uuid.to_vec())));
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_projectiles_owner_primes_tnt_and_a_gone_owner_does_not() {
    let mut game = server_game();
    game.build_registries();
    let ghast = 91;
    let kind = game.res.mob_types().get("minecraft:ghast").unwrap().clone();
    game.mobs
        .push(Mob::new(ghast, kind, Vec3::new(40.0, 64.0, 40.0), 0.0));
    let uuid = game.uuid_for(ghast);
    let tnt = game
        .pack
        .assets
        .registry
        .default_state("minecraft:tnt")
        .unwrap();
    // `LargeFireball.onHit` and `WitherSkull.onHit` name the projectile's owner
    // as the blast's indirect source (`Explosion.getIndirectSourceEntity`).
    game.combat.explosion_owner = Some(ghast);
    game.world.set(9, 64, 8, tnt).unwrap();
    game.server_explode([8.5, 64.0, 8.5], 3.0, BlockInteraction::Destroy, false)
        .unwrap();
    assert_eq!(game.tnts.len(), 1, "the blast primed the TNT block");
    assert_eq!(
        game.tnts[0].owner,
        Some(Who::Mob(ghast)),
        "the chain TNT remembers the fireball's owner"
    );
    assert_eq!(
        game.tnt_nbt(0).get("owner"),
        Some(&Tag::IntArray(uuid.to_vec()))
    );
    // `Projectile.getOwner` is an `EntityReference`: a removed owner does not
    // resolve, so the TNT it primes is left without one.
    game.tnts.clear();
    game.mobs[0].removed = true;
    game.world.set(9, 64, 8, tnt).unwrap();
    game.server_explode([8.5, 64.0, 8.5], 3.0, BlockInteraction::Destroy, false)
        .unwrap();
    assert_eq!(game.tnts.len(), 1, "the blast primed the TNT block again");
    assert_eq!(game.tnts[0].owner, None, "a gone owner primes no one");
    game.combat.explosion_owner = None;
}
