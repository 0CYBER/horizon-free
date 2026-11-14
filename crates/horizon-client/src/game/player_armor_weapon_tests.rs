//! `CombatRules.getDamageAfterAbsorb` on a player victim: the attacker's weapon
//! thins the armour fraction (`EnchantmentHelper.modifyArmorEffectiveness`), so
//! a mob holding a Breach mace hits a player harder.
use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
use horizon_client::{items::ItemStack, mobs::Mob, motion::PlayerMotion};
use horizon_core::geometry::Vec3;

fn with_game(test: impl FnOnce(&mut Game<'_>)) {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        root.clone(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        true,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let mut g = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&root),
        None,
    )
    .unwrap();
    test(&mut g);
}

/// A mace with `breach` levels of Breach.
fn mace(breach: u32) -> ItemStack {
    let mut stack = ItemStack::new("minecraft:mace", 1);
    if breach > 0 {
        stack.enchant("minecraft:breach", breach);
    }
    stack
}

/// What one zombie blow of 5.0 takes off a player in full diamond armour, the
/// zombie's main hand as given.
fn taken(hand: Option<ItemStack>) -> f32 {
    let mut taken = 0.0;
    with_game(|g| {
        g.player = Some(PlayerMotion::new(g.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
        g.player.as_mut().unwrap().entity_id = 77;
        g.armor = [
            Some(ItemStack::new("minecraft:diamond_helmet", 1)),
            Some(ItemStack::new("minecraft:diamond_chestplate", 1)),
            Some(ItemStack::new("minecraft:diamond_leggings", 1)),
            Some(ItemStack::new("minecraft:diamond_boots", 1)),
        ];
        g.apply_equipment();
        let before = g.player.as_ref().unwrap().health.health();
        let kind = g.res.mob_types().get("minecraft:zombie").unwrap().clone();
        let mut zombie = Mob::new(41, kind, Vec3::new(2.5, 64.0, 0.5), 0.0);
        zombie.extra.set_main_hand_stack(hand);
        g.mobs.push(zombie);
        g.mob_hits_player(5.0, [2.5, 64.0, 0.5], "minecraft:mob_attack");
        taken = before - g.player.as_ref().unwrap().health.health();
    });
    taken
}

/// Full diamond is armour 20 and toughness 8, so a 5.0 blow keeps
/// `clamp(20 - 5 / (2 + 8/4), 4, 20)` = 18.75 of the 25 points, 0.75 of the
/// damage: 1.25. Breach IV's `minecraft:add` of -0.15 a level takes 0.6 off
/// that fraction, 0.15 left, so 4.25 of the 5.0. Without a weapon nothing
/// changes the fraction.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_mobs_breach_mace_thins_a_players_armour() {
    let plain = taken(Some(mace(0)));
    assert_eq!(plain, 1.25);
    let breach = taken(Some(mace(4)));
    assert_eq!(breach, 4.25);
    assert_eq!(taken(None), plain);
}
