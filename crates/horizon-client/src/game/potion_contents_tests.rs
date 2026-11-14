//! `AbstractThrownPotion.onHit` reads the stack's whole `potion_contents`: the
//! potion's effects and the stack's `custom_effects`, the colour they mix to
//! (or `custom_color`) and its `potion_duration_scale`.
use super::*;
use crate::game::native_machines::tests::server_game;
use horizon_client::motion::PlayerMotion;

/// A server's game with a player standing at the origin's block.
fn player_game() -> Game<'static> {
    let mut game = server_game();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    // A fixed name: no generated horizon-player.txt in the working directory.
    game.options.player_name = Some("PotionContents".into());
    game.build_registries();
    game.set_allow_commands(true);
    game
}

/// A stack with `custom_effects` of `(effect, duration, amplifier)` on top of
/// whatever potion it carries, as `PotionContents.customEffects`.
fn with_custom_effects(mut stack: ItemStack, effects: &[(&str, i32, i32)]) -> ItemStack {
    let list = effects
        .iter()
        .map(|(id, duration, amp)| {
            Tag::compound([
                ("id", Tag::String(format!("minecraft:{id}").as_str().into())),
                ("duration", Tag::Int(*duration)),
                ("amplifier", Tag::Int(*amp)),
            ])
        })
        .collect();
    stack
        .data_mut()
        .potion_extra
        .push(("custom_effects".into(), Tag::List(list)));
    stack
}

/// `ThrownSplashPotion.onHitAsPotion` over `PotionContents.getAllEffects` and
/// `PotionContents.getColor`: a swiftness potion carrying a custom poison
/// effect lands both, each for `(int)(scale * d * durationScale + 0.5)` of its
/// ticks, and the splash's hundred spell particles are drawn in the contents'
/// `custom_color`.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_splash_potion_applies_its_custom_effects_and_duration_scale() {
    let mut game = player_game();
    let mut stack = with_custom_effects(
        ItemStack::new("minecraft:splash_potion", 1).with_potion("minecraft:swiftness"),
        &[("poison", 900, 0)],
    );
    stack.data_mut().set_other(
        "minecraft:potion_duration_scale",
        Some(Tag::Float(0.125f32.to_bits())),
    );
    stack
        .data_mut()
        .potion_extra
        .push(("custom_color".into(), Tag::Int(0x0020_4080)));
    game.particles.requested = Some(Vec::new());
    let at = Vec3::new(0.5, 64.0, 0.5);
    game.splash(
        &stack,
        at,
        None,
        None,
        0,
        &PotionOwner::default(),
        true,
        Vec3::ZERO,
    );
    let effects = &game.potions.effects;
    // The potion's 3600 ticks and the custom effect's 900, both at 0.125.
    assert_eq!(
        effects.get("speed").map(|e| e.duration),
        Some(450),
        "the potion's own effect, scaled"
    );
    assert_eq!(
        effects.get("poison").map(|e| e.duration),
        Some(113),
        "the custom effect, scaled"
    );
    let spell: Vec<[f32; 4]> = game
        .particles
        .requested
        .as_ref()
        .unwrap()
        .iter()
        .filter(|(o, ..)| o.ty == ParticleType::Effect)
        .map(|(o, ..)| o.color)
        .collect();
    assert_eq!(spell.len(), 100, "the splash's spell particles");
    for c in spell {
        // 0x204080, brightened by each particle's own 0.75 to 1.
        assert!(
            (c[0] * 2.0 - c[1]).abs() < 1.0e-4 && (c[1] * 2.0 - c[2]).abs() < 1.0e-4,
            "{c:?}"
        );
    }
}

/// `ThrownLingeringPotion.onHitAsPotion` with `AreaEffectCloud.applyComponentsFromItemStack`:
/// contents with only custom effects still make a cloud, in their colour, and
/// it sprays them at the stack's `potion_duration_scale` (1.0 here, over the
/// lingering potion's own 0.25).
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_lingering_potion_of_custom_effects_makes_its_cloud() {
    let mut game = player_game();
    let mut stack = with_custom_effects(
        ItemStack::new("minecraft:lingering_potion", 1),
        &[("poison", 900, 0)],
    );
    stack
        .data_mut()
        .potion_extra
        .push(("custom_color".into(), Tag::Int(0x0012_3456)));
    stack.data_mut().set_other(
        "minecraft:potion_duration_scale",
        Some(Tag::Float(1.0f32.to_bits())),
    );
    game.splash(
        &stack,
        Vec3::new(0.5, 64.0, 0.5),
        None,
        None,
        0,
        &PotionOwner::default(),
        true,
        Vec3::ZERO,
    );
    assert_eq!(
        game.breath_clouds.len(),
        1,
        "a cloud for custom-only contents"
    );
    let cloud = game.breath_clouds[0].potion.as_ref().unwrap();
    assert_eq!(cloud.effects, vec![("poison", 900, 0)]);
    assert_eq!(cloud.settings.duration_scale, 1.0);
    let particle = cloud.particle();
    assert_eq!(particle.ty, ParticleType::EntityEffect);
    assert_eq!(
        particle.color,
        [
            0x12 as f32 / 255.0,
            0x34 as f32 / 255.0,
            0x56 as f32 / 255.0,
            1.0
        ],
        "the cloud's particles are drawn in the contents' colour"
    );
}

/// `AbstractThrownPotion.affectEntitiesAround` with `Axolotl.rehydrate`:
/// water refills an axolotl's air by 1800 once, not once per copy of the
/// `#rehydrates_axolotls` check.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_water_potion_refills_an_axolotl_once() {
    let mut game = player_game();
    assert!(game.run_command(
        "summon minecraft:axolotl 0.5 64 0.5 {NoAI:1b,PersistenceRequired:1b,Silent:1b}"
    ));
    game.mobs[0].air = 0;
    let water = ItemStack::new("minecraft:splash_potion", 1).with_potion("minecraft:water");
    game.splash(
        &water,
        Vec3::new(0.5, 64.0, 0.5),
        None,
        None,
        0,
        &PotionOwner::default(),
        true,
        Vec3::ZERO,
    );
    assert_eq!(game.mobs[0].air, 1800, "one rehydrate, capped at the max");
}

/// `PotionContents.is(PotionTags.DOUSES_FIRE)` needs `isPotionWithoutCustomEffects`:
/// water carrying a custom effect leaves a lit campfire alone, while plain
/// water still puts it out.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_water_potion_with_custom_effects_does_not_douse_fire() {
    let mut game = server_game();
    let pos = [2, 4, 3];
    let campfire = game
        .pack
        .assets
        .registry
        .default_state("minecraft:campfire")
        .unwrap();
    game.world.set(pos[0], pos[1], pos[2], campfire).unwrap();
    let lit = |game: &Game<'_>| {
        game.pack
            .assets
            .registry
            .state(game.world.block(pos[0], pos[1], pos[2]))
            .and_then(|s| s.properties.get("lit"))
            .is_some_and(|v| v == "true")
    };
    assert!(lit(&game), "the campfire starts lit");
    // The potion hits the top of the block under the campfire, so the cell
    // `AbstractThrownPotion.douseFire` looks at first is the campfire's.
    let hit = Some(([2, 3, 3], Direction::Up));
    let at = Vec3::new(2.5, 3.5, 3.5);
    let throw = |game: &mut Game<'_>, stack: &ItemStack| {
        game.splash(
            stack,
            at,
            None,
            hit,
            0,
            &PotionOwner::default(),
            true,
            Vec3::ZERO,
        )
    };
    let water = ItemStack::new("minecraft:splash_potion", 1).with_potion("minecraft:water");
    throw(
        &mut game,
        &with_custom_effects(water.clone(), &[("slowness", 1800, 0)]),
    );
    assert!(
        lit(&game),
        "water with custom effects is not in #douses_fire"
    );
    throw(&mut game, &water);
    assert!(!lit(&game), "plain water puts the campfire out");
}
