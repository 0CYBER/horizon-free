//! Filled buckets and cauldrons count as "Times Used": `BucketItem.use`
//! awards `Stats.ITEM_USED` once a filled bucket emptied into the world, and
//! `CauldronInteractions`' bucket and bottle interactions award it next to
//! their own cauldron statistic (washing a shulker box does not).
use super::*;
use crate::game::{Hit, native_machines::tests::server_game};
use horizon_client::items::ItemStack;

const POS: [i32; 3] = [2, 4, 3];

/// A local server game with `state` at `POS`, holding `held`, the click
/// forced onto the block's top.
fn cauldron_game(state: &str, level: Option<u8>, held: ItemStack) -> Game<'static> {
    let mut game = server_game();
    game.creative = false;
    let block = game
        .pack
        .assets
        .registry
        .resolve(&horizon_world::chunk::BlockState {
            name: state.to_owned(),
            properties: level
                .map(|level| ("level".to_owned(), level.to_string()))
                .into_iter()
                .collect(),
        })
        .unwrap();
    game.world.set(POS[0], POS[1], POS[2], block).unwrap();
    *game.held_mut() = Some(held);
    game.forced_hit = Some(Hit {
        pos: POS,
        face: [0, 1, 0],
        point: [
            f64::from(POS[0]) + 0.5,
            f64::from(POS[1]) + 1.0,
            f64::from(POS[2]) + 0.5,
        ],
    });
    game
}

/// The held item's id.
fn hand<'a>(game: &'a Game<'_>) -> &'a str {
    game.held().map_or("", |stack| &stack.item)
}

/// `BucketItem.use`: pouring a water bucket onto a block empties it into the
/// block above, and that use counts as one `used.water_bucket`.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn emptying_a_filled_bucket_counts_as_used() {
    let mut game = cauldron_game(
        "minecraft:stone",
        None,
        ItemStack::new("minecraft:water_bucket", 1),
    );
    let hit = game.forced_hit.unwrap();
    assert!(game.use_hand(Some(&hit)).unwrap(), "the bucket empties");
    let poured = game
        .pack
        .assets
        .registry
        .state(game.world.block(POS[0], POS[1] + 1, POS[2]))
        .map(|state| state.name.as_str());
    assert_eq!(poured, Some("minecraft:water"), "the water went above");
    assert_eq!(hand(&game), "minecraft:bucket", "the bucket came back");
    assert_eq!(game.stats.get("used", "minecraft:water_bucket"), 1);
    // A second pour is a second use.
    *game.held_mut() = Some(ItemStack::new("minecraft:water_bucket", 1));
    assert!(game.use_hand(Some(&hit)).unwrap());
    assert_eq!(game.stats.get("used", "minecraft:water_bucket"), 2);
}

/// `CauldronInteractions.emptyBucket`: pouring a bucket into an empty
/// cauldron earns `fill_cauldron` and `used.water_bucket` together.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn pouring_a_bucket_into_a_cauldron_counts_both() {
    let mut game = cauldron_game(
        "minecraft:cauldron",
        None,
        ItemStack::new("minecraft:water_bucket", 1),
    );
    let hit = game.forced_hit.unwrap();
    assert!(game.use_hand(Some(&hit)).unwrap(), "the cauldron takes it");
    assert_eq!(hand(&game), "minecraft:bucket", "the bucket came back");
    assert_eq!(game.stats.get("custom", "minecraft:fill_cauldron"), 1);
    assert_eq!(game.stats.get("used", "minecraft:water_bucket"), 1);
}

/// `CauldronInteractions.fillBucket`: a bucket dipped into a full water
/// cauldron earns `use_cauldron` and `used.bucket` together.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn filling_a_bucket_from_a_cauldron_counts_both() {
    let mut game = cauldron_game(
        "minecraft:water_cauldron",
        Some(3),
        ItemStack::new("minecraft:bucket", 1),
    );
    let hit = game.forced_hit.unwrap();
    assert!(game.use_hand(Some(&hit)).unwrap(), "the cauldron takes it");
    assert_eq!(hand(&game), "minecraft:water_bucket", "the bucket filled");
    assert_eq!(game.stats.get("custom", "minecraft:use_cauldron"), 1);
    assert_eq!(game.stats.get("used", "minecraft:bucket"), 1);
}

/// `WATER.put(GLASS_BOTTLE, ...)`: the bottle counts as used too.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_glass_bottle_used_on_a_cauldron_counts_as_used() {
    let mut game = cauldron_game(
        "minecraft:water_cauldron",
        Some(3),
        ItemStack::new("minecraft:glass_bottle", 1),
    );
    let hit = game.forced_hit.unwrap();
    assert!(game.use_hand(Some(&hit)).unwrap(), "the cauldron takes it");
    assert_eq!(hand(&game), "minecraft:potion", "the bottle filled");
    assert_eq!(game.stats.get("custom", "minecraft:use_cauldron"), 1);
    assert_eq!(game.stats.get("used", "minecraft:glass_bottle"), 1);
}

/// `EMPTY.put(POTION, ...)`: a water potion counts as used too.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_water_potion_used_on_a_cauldron_counts_as_used() {
    let mut game = cauldron_game(
        "minecraft:cauldron",
        None,
        ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water"),
    );
    let hit = game.forced_hit.unwrap();
    assert!(game.use_hand(Some(&hit)).unwrap(), "the cauldron takes it");
    assert_eq!(hand(&game), "minecraft:glass_bottle", "the potion emptied");
    assert_eq!(game.stats.get("custom", "minecraft:use_cauldron"), 1);
    assert_eq!(game.stats.get("used", "minecraft:potion"), 1);
}

/// `shulkerBoxInteraction`: washing one earns `clean_shulker_box` alone.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn washing_a_shulker_box_is_not_an_item_used() {
    let mut game = cauldron_game(
        "minecraft:water_cauldron",
        Some(3),
        ItemStack::new("minecraft:white_shulker_box", 1),
    );
    let hit = game.forced_hit.unwrap();
    assert!(game.use_hand(Some(&hit)).unwrap(), "the cauldron takes it");
    assert_eq!(hand(&game), "minecraft:shulker_box", "the box washed");
    assert_eq!(game.stats.get("custom", "minecraft:clean_shulker_box"), 1);
    assert_eq!(game.stats.get("used", "minecraft:white_shulker_box"), 0);
}
