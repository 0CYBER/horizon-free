//! `Player.attack`'s admission of a hit (`baseDamage > 0.0F || magicBoost > 0.0F`), the
//! attacker's `ATTACK_KNOCKBACK` in `LivingEntity.getKnockback`, and a creative player's
//! blow through `Entity.isInvulnerableToBase`'s `!source.isCreativePlayer()`.
use super::super::*;
use horizon_client::chunk::Chunk;
use horizon_client::motion::PlayerMotion;
use std::sync::Arc;

/// A world with one player standing at (0.5, 64, 0.5) and nothing else.
fn game(creative: bool) -> Game<'static> {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res: &'static _ = Box::leak(Box::new(loaded.resources));
    let mut game = Game::new(
        res,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.to_path_buf(),
        )),
        None,
    )
    .unwrap();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.creative = creative;
    game.mode = if creative {
        horizon_client::command::args::GameMode::Creative
    } else {
        horizon_client::command::args::GameMode::Survival
    };
    // A full attack charge (`getAttackStrengthScale(0.5)` = 1).
    game.attack_ticks = 20;
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game
}

/// A base value of an attribute of the player, as `/attribute ... base set` writes it.
fn set_base(player: &mut PlayerMotion, name: &str, value: f64) {
    let key = player.attributes.key(name).expect("attribute key");
    let handle = player
        .attributes
        .get_instance(Some(key))
        .unwrap()
        .expect("attribute instance");
    player
        .attributes
        .edit(&handle, |v, observer| v.set_base_value(value, observer))
        .unwrap();
}

/// A pig at (0.5, 64, 2.5), 2 blocks south of the player, with `extra` NBT.
fn summon_pig(game: &mut Game<'_>, extra: &str) {
    assert!(game.run_command(&format!(
        "summon minecraft:pig 0.5 64.0 2.5 {{NoAI:1b,NoGravity:1b,Silent:1b,PersistenceRequired:1b{extra}}}"
    )));
    assert_eq!(game.mobs.len(), 1, "the pig was summoned");
}

/// Look at the center of the first mob, as the attack fixture does.
fn aim_at_mob(game: &mut Game<'_>) {
    let center = game.mobs[0].center();
    let eye = game.eye_position().expect("player");
    let (dx, dy, dz) = (center[0] - eye[0], center[1] - eye[1], center[2] - eye[2]);
    game.yaw = (-dx).atan2(dz).to_degrees() as f32;
    game.pitch = (-dy).atan2((dx * dx + dz * dz).sqrt()).to_degrees() as f32;
}

/// `/attribute @s minecraft:attack_damage base set 0`: `Player.attack` computes no damage,
/// so `baseDamage > 0.0F || magicBoost > 0.0F` is false and the whole hurt tail is skipped:
/// no hurt (so no anger and no hurt sound), no `causeExtraKnockback`, no
/// `damageStatsAndHearts`, no `setLastHurtMob` and no `causeFoodExhaustion`. `Player.onAttack`
/// still starts the attack strength again.
#[test]
#[ignore = "daily"]
fn a_hit_that_deals_no_damage_has_no_knockback_anger_or_exhaustion() {
    let mut game = game(false);
    summon_pig(&mut game, "");
    set_base(
        game.player.as_mut().unwrap(),
        "minecraft:attack_damage",
        0.0,
    );
    aim_at_mob(&mut game);
    assert!(game.attack(), "the attack reached the pig");
    let pig = &game.mobs[0];
    assert_eq!(pig.health, 10.0, "the pig took no damage");
    assert_eq!(
        (pig.velocity.x, pig.velocity.z),
        (0.0, 0.0),
        "the pig was knocked back by a hit that deals no damage"
    );
    assert_eq!(pig.last_hurt, 0.0, "the hit started a hurt cooldown");
    assert_eq!(pig.invulnerable, 0, "the hit started a hurt cooldown");
    assert_eq!(pig.ai.last_hurt_by_player, 0, "the pig was angered");
    assert!(
        game.combat.last_hurt_mob.is_none(),
        "setLastHurtMob ran for a hit that dealt no damage"
    );
    let player = game.player.as_ref().unwrap();
    assert_eq!(
        player.health.food.exhaustion, 0.0,
        "causeFoodExhaustion(0.1F) ran for a hit that dealt no damage"
    );
    assert_eq!(
        game.attack_ticks, 0,
        "Player.onAttack did not start the attack strength again"
    );
}

/// `LivingEntity.getKnockback` halves the attacker's `ATTACK_KNOCKBACK` attribute:
/// `causeExtraKnockback` adds it on top of the 0.4 `dealDefaultKnockback` already applied,
/// halving what the default left.
#[test]
#[ignore = "daily"]
fn the_melee_knockback_scales_with_the_attackers_attack_knockback() {
    let mut game = game(false);
    summon_pig(&mut game, "");
    // A held item, so `getKnockback` runs `EnchantmentHelper.modifyKnockback` over the
    // attribute value (no Knockback enchantment: it returns that value unchanged).
    game.inventory[game.selected] = Some(ItemStack::new("minecraft:wooden_shovel", 1));
    aim_at_mob(&mut game);
    // A fully charged hit at 0 ATTACK_KNOCKBACK: only the hit's own 0.4 knockback.
    assert!(game.attack(), "the attack reached the pig");
    let plain = game.mobs[0].velocity.z;
    assert!(
        (plain - 0.4).abs() < 1e-6,
        "a plain hit's knockback: {plain}, expected 0.4"
    );
    // `/attribute @s minecraft:attack_knockback base set 5`: getKnockback is 5 / 2, and
    // causeExtraKnockback halves the 0.4 the hit left before adding it.
    set_base(
        game.player.as_mut().unwrap(),
        "minecraft:attack_knockback",
        5.0,
    );
    game.mobs[0].velocity = Vec3::ZERO;
    game.mobs[0].invulnerable = 0;
    game.mobs[0].last_hurt = 0.0;
    game.attack_ticks = 20;
    assert!(game.attack(), "the second attack reached the pig");
    let boosted = game.mobs[0].velocity.z;
    assert!(
        (boosted - 2.7).abs() < 1e-6,
        "the knockback of ATTACK_KNOCKBACK 5: {boosted}, expected 2.7"
    );
}

/// `Entity.isInvulnerableToBase`: `Invulnerable` refuses a hit unless the source bypasses
/// it or `source.isCreativePlayer()` — a creative player's melee still damages the mob.
#[test]
#[ignore = "daily"]
fn a_creative_player_hurts_an_invulnerable_mob() {
    let mut game = game(false);
    summon_pig(&mut game, ",Invulnerable:1b");
    assert!(game.mobs[0].invulnerable_flag, "the pig is Invulnerable");
    aim_at_mob(&mut game);
    assert!(game.attack(), "the attack reached the pig");
    assert_eq!(
        game.mobs[0].health, 10.0,
        "a survival player's hit on an Invulnerable pig"
    );
    // The same hit from a creative player: `source.isCreativePlayer()`.
    game.creative = true;
    game.mode = horizon_client::command::args::GameMode::Creative;
    game.attack_ticks = 20;
    assert!(game.attack(), "the second attack reached the pig");
    assert_eq!(
        game.mobs[0].health, 9.0,
        "a creative player's hit on an Invulnerable pig"
    );
}
