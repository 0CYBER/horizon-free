//! `BowItem.releaseUsing` through `ProjectileWeaponItem.draw` and `shoot`: the
//! bow's enchantment components decide the arrows it fires, and the ammunition
//! is whatever `#minecraft:arrows` lists.
use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
use horizon_client::items::ItemStack;

/// A game on the prepared resources with a creative player (as `crossbow.rs`).
fn game_with_player(root: &std::path::Path) -> Game<'static> {
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        Vec::new(),
        None,
        root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.to_path_buf();
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
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
    game.creative = true;
    game.player = Some(
        horizon_client::motion::PlayerMotion::new(
            game.motion,
            horizon_core::geometry::Vec3::new(10.5, 64.0, -3.25),
        )
        .unwrap(),
    );
    // A still player: `known_player_movement` is the last tick's step.
    game.previous_position = game.player.as_ref().expect("a player").position;
    game.audio.capture = Some(Vec::new());
    game
}

/// The horizontal heading `p` was shot with, in degrees:
/// `Projectile.shootFromRotation` sends it along `(-sin(yaw), ..., cos(yaw))`.
fn heading(p: &horizon_client::projectiles::Projectile) -> f32 {
    (-p.velocity.x as f32)
        .atan2(p.velocity.z as f32)
        .to_degrees()
}

/// `AbstractArrow`'s constructor asks the weapon for its piercing count
/// (`EnchantmentHelper.getPiercingCount`): a bow with Piercing 4 shoots arrows
/// that pass through four entities. Before, only a crossbow's arrows carried
/// a pierce level, so a piercing bow's arrow stopped at the first mob.
#[test]
#[ignore = "daily"]
fn a_piercing_bow_shoots_arrows_that_pierce_their_level() {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let mut game = game_with_player(&root);
    let mut bow = ItemStack::new("minecraft:bow", 1);
    bow.enchant("minecraft:piercing", 4);
    *game.held_mut() = Some(bow);
    let bow = game.held().unwrap().clone();
    let before = game.creatures.projectiles.len();
    game.release_bow(&bow, 20);
    let pierce: Vec<u8> = game.creatures.projectiles[before..]
        .iter()
        .filter_map(|p| p.arrow.as_ref().map(|a| a.pierce))
        .collect();
    assert_eq!(pierce, vec![4]);
}

/// `ProjectileWeaponItem.draw` counts the projectiles through the weapon's
/// `projectile_count` and `shoot` fans them by its `projectile_spread`:
/// Multishot's +2 arrows, 10 degrees apart, centred on the view — so 0, -10
/// and +10 degrees in the order they are shot. `useAmmo` takes one arrow and
/// marks the extra ones `INTANGIBLE_PROJECTILE`, so only the first is the
/// player's to take. Before, a multishot bow shot one arrow.
#[test]
#[ignore = "daily"]
fn a_multishot_bow_fires_three_arrows_ten_degrees_apart() {
    use horizon_client::arrows::Pickup;
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let mut game = game_with_player(&root);
    // Not creative, with a stack of arrows to draw from.
    game.creative = false;
    let mut bow = ItemStack::new("minecraft:bow", 1);
    bow.enchant("minecraft:multishot", 1);
    *game.held_mut() = Some(bow);
    game.inventory[9] = Some(ItemStack::new("minecraft:arrow", 4));
    let bow = game.held().unwrap().clone();
    // Looking somewhere other than the axes: the fan is centred on the view.
    game.yaw = 37.0;
    let before = game.creatures.projectiles.len();
    game.release_bow(&bow, 20);
    let arrows = &game.creatures.projectiles[before..];
    let headings: Vec<f32> = arrows.iter().map(heading).collect();
    assert_eq!(headings.len(), 3, "{headings:?}");
    // One of the four arrows is used, and only the first is the player's.
    assert_eq!(game.inventory[9].as_ref().map(|s| s.count), Some(3));
    let pickup: Vec<Pickup> = arrows
        .iter()
        .map(|p| p.arrow.as_ref().expect("an arrow").pickup)
        .collect();
    assert_eq!(
        pickup,
        [Pickup::Allowed, Pickup::CreativeOnly, Pickup::CreativeOnly]
    );
    // `Projectile.shoot`'s inaccuracy is up to about a degree of that fan.
    let yaw = game.yaw;
    for (i, want) in [0.0f32, -10.0, 10.0].into_iter().enumerate() {
        let mut off = (headings[i] - yaw + 180.0) % 360.0;
        if off < 0.0 {
            off += 360.0;
        }
        let off = off - 180.0;
        assert!(
            (off - want).abs() < 1.5,
            "arrow {i} at {off} degrees, want {want}: {headings:?}"
        );
    }
}

/// `ProjectileWeaponItem.ARROW_ONLY` is `itemStack.is(ItemTags.ARROWS)`: an
/// item a data pack adds to `#minecraft:arrows` is bow and crossbow ammunition.
/// Before, `is_arrow` listed the three vanilla ids.
#[test]
fn an_item_a_pack_adds_to_the_arrows_tag_is_ammunition() {
    use crate::game::crossbow::projectile_slot;
    use crate::game::shield::OFFHAND_SLOT;
    use horizon_client::items::Items;
    let tag = |values: &[&str]| {
        std::collections::HashMap::from([(
            "minecraft:arrows".to_owned(),
            values.iter().map(|v| (*v).to_owned()).collect(),
        )])
    };
    let items = Items::for_tests(tag(&["minecraft:arrow", "test:bone_arrow"]));
    let mut inventory = vec![None; 36];
    inventory[9] = Some(ItemStack::new("test:bone_arrow", 4));
    let hands = [(OFFHAND_SLOT, None), (0, None)];
    assert_eq!(
        projectile_slot("minecraft:bow", hands, &inventory, &items),
        Some(9)
    );
    // An item the tag does not list stays out of the bow.
    inventory[9] = Some(ItemStack::new("test:pebble", 4));
    assert_eq!(
        projectile_slot("minecraft:bow", hands, &inventory, &items),
        None
    );
}
