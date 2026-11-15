//! `ItemStack.useOn`'s adventure-mode gate: every item's own `useOn` runs
//! only where `Player.mayBuild` holds or the stack's `can_place_on` matches
//! the clicked block. Tools (an axe), honeycomb, bone meal, shears, a water
//! bottle and a spawn egg used on a block pass in adventure mode otherwise.
use crate::game::game_tests::tests::server_game;
use crate::game::{Game, GuestUse, Hit};
use horizon_client::command::args::GameMode;
use horizon_client::first_person::Hand;
use horizon_client::items::ItemStack;
use horizon_core::nbt::Tag;
use horizon_world::registry::BlockStateId;

/// A server's game with the prepared resources, or `None` where CI has none.
fn game() -> Option<Game<'static>> {
    std::env::var_os("HORIZON_TEST_RESOURCES")?;
    let resources = Box::leak(Box::new(None));
    let mut game = server_game(resources);
    game.build_registries();
    Some(game)
}

/// `stack` with a `can_place_on` component naming `block`.
fn placeable(name: &str, count: u32, block: &str) -> ItemStack {
    let mut stack = ItemStack::new(name, count);
    stack.data_mut().other.push((
        "minecraft:can_place_on".into(),
        Tag::compound([("blocks", Tag::List(vec![Tag::String(block.into())]))]),
    ));
    stack
}

/// Puts `name` at `pos` with `properties` overridden, returning its state.
fn place(
    game: &mut Game<'_>,
    pos: [i32; 3],
    name: &str,
    properties: &[(&str, &str)],
) -> BlockStateId {
    let registry = &game.pack.assets.registry;
    let mut state = registry
        .state(registry.default_state(name).unwrap())
        .unwrap()
        .clone();
    for (key, value) in properties {
        state.properties.insert((*key).into(), (*value).into());
    }
    let id = registry.id(&state).unwrap();
    game.world.set(pos[0], pos[1], pos[2], id).unwrap();
    id
}

/// The top face of `pos`, as `Minecraft.hitResult` names it.
fn hit(pos: [i32; 3]) -> Hit {
    Hit {
        pos,
        face: [0, 1, 0],
        point: [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 1.0,
            f64::from(pos[2]) + 0.5,
        ],
        inside: false,
    }
}

/// Clicks the top of `pos` with `stack`, as `Minecraft.startUseItem` does.
fn click(game: &mut Game<'_>, pos: [i32; 3], stack: ItemStack) {
    game.using = None;
    game.cooldowns = Default::default();
    *game.held_mut() = Some(stack);
    let hit = hit(pos);
    game.forced_hit = Some(hit);
    game.with_player_actor(|g| g.use_hand(Some(&hit))).unwrap();
}

/// The same click as a LAN guest's `UseItemOn` packet: the server acts for one
/// hand on the block the guest's client named, and stops at `ItemStack.useOn`.
fn click_as_guest(game: &mut Game<'_>, pos: [i32; 3], stack: ItemStack) {
    game.using = None;
    game.cooldowns = Default::default();
    *game.held_mut() = Some(stack);
    let hit = hit(pos);
    game.forced_hit = Some(hit);
    game.guest_use = Some(GuestUse::OnBlock(Hand::Main));
    game.with_player_actor(|g| g.use_hand(Some(&hit))).unwrap();
    game.guest_use = None;
}

fn state(game: &Game<'_>, pos: [i32; 3]) -> BlockStateId {
    game.world.block(pos[0], pos[1], pos[2])
}

fn held(game: &Game<'_>) -> Option<(String, u32)> {
    game.held().map(|s| (s.item.to_string(), s.count))
}

/// In adventure mode a click without a matching `can_place_on` leaves the
/// block unchanged; one with it acts, as `ItemStack.useOn` does.
#[test]
fn an_adventure_use_on_needs_may_build_or_a_matching_can_place_on() {
    let Some(mut game) = game() else {
        return;
    };
    game.set_mode(GameMode::Adventure);

    // AxeItem.useOn: strip an oak log.
    let log = [2, 4, 3];
    let oak_log = place(&mut game, log, "minecraft:oak_log", &[]);
    click(&mut game, log, ItemStack::new("minecraft:iron_axe", 1));
    assert_eq!(state(&game, log), oak_log, "the axe stripped nothing");
    assert_eq!(
        held(&game),
        Some(("minecraft:iron_axe".into(), 1)),
        "the axe was not used"
    );
    let stripped = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stripped_oak_log")
        .unwrap();
    click(
        &mut game,
        log,
        placeable("minecraft:iron_axe", 1, "minecraft:oak_log"),
    );
    assert_eq!(state(&game, log), stripped, "a matching axe strips");

    // The same click through a guest's `UseItemOn`, as single player runs.
    let guest_log = [3, 4, 3];
    let guest_oak = place(&mut game, guest_log, "minecraft:oak_log", &[]);
    click_as_guest(
        &mut game,
        guest_log,
        ItemStack::new("minecraft:iron_axe", 1),
    );
    assert_eq!(
        state(&game, guest_log),
        guest_oak,
        "the guest's axe stripped nothing"
    );

    // HoneycombItem.useOn: wax a copper block.
    let copper = [4, 4, 3];
    let copper_block = place(&mut game, copper, "minecraft:copper_block", &[]);
    click(&mut game, copper, ItemStack::new("minecraft:honeycomb", 2));
    assert_eq!(state(&game, copper), copper_block, "the honeycomb waxed");
    assert_eq!(
        held(&game),
        Some(("minecraft:honeycomb".into(), 2)),
        "the honeycomb was not used"
    );
    let waxed = game
        .pack
        .assets
        .registry
        .default_state("minecraft:waxed_copper_block")
        .unwrap();
    click(
        &mut game,
        copper,
        placeable("minecraft:honeycomb", 2, "minecraft:copper_block"),
    );
    assert_eq!(state(&game, copper), waxed, "a matching honeycomb waxes");

    // BoneMealItem.useOn: grow a wheat crop, which is always used up.
    let wheat = [6, 4, 3];
    place(&mut game, wheat, "minecraft:wheat", &[]);
    click(&mut game, wheat, ItemStack::new("minecraft:bone_meal", 3));
    assert_eq!(
        held(&game),
        Some(("minecraft:bone_meal".into(), 3)),
        "the bone meal was not used"
    );
    click(
        &mut game,
        wheat,
        placeable("minecraft:bone_meal", 3, "minecraft:wheat"),
    );
    assert_eq!(
        held(&game),
        Some(("minecraft:bone_meal".into(), 2)),
        "a matching bone meal is used"
    );

    // ShearsItem.useOn: crop a growing plant's head to its top age.
    let vines = [8, 4, 3];
    let young = place(&mut game, vines, "minecraft:weeping_vines", &[("age", "0")]);
    click(&mut game, vines, ItemStack::new("minecraft:shears", 1));
    assert_eq!(state(&game, vines), young, "the shears cropped");
    let grown = {
        let registry = &game.pack.assets.registry;
        let mut state = registry.state(young).unwrap().clone();
        state.properties.insert("age".into(), "25".into());
        registry.id(&state).unwrap()
    };
    click(
        &mut game,
        vines,
        placeable("minecraft:shears", 1, "minecraft:weeping_vines"),
    );
    assert_eq!(state(&game, vines), grown, "matching shears crop");

    // PotionItem.useOn: a water bottle turns dirt to mud.
    let dirt = [10, 4, 3];
    let dirt_state = place(&mut game, dirt, "minecraft:dirt", &[]);
    click(
        &mut game,
        dirt,
        ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water"),
    );
    assert_eq!(state(&game, dirt), dirt_state, "the water bottle poured");
    let mud = game
        .pack
        .assets
        .registry
        .default_state("minecraft:mud")
        .unwrap();
    click(
        &mut game,
        dirt,
        placeable("minecraft:potion", 1, "minecraft:dirt").with_potion("minecraft:water"),
    );
    assert_eq!(state(&game, dirt), mud, "a matching water bottle pours");

    // SpawnEggItem.useOn: a pig egg spawns a pig on the clicked face.
    let stone = [12, 4, 3];
    place(&mut game, stone, "minecraft:stone", &[]);
    let mobs = game.mobs.len();
    click(
        &mut game,
        stone,
        ItemStack::new("minecraft:pig_spawn_egg", 1),
    );
    assert_eq!(game.mobs.len(), mobs, "the egg spawned a mob");
    assert_eq!(
        held(&game),
        Some(("minecraft:pig_spawn_egg".into(), 1)),
        "the egg was not used"
    );
    click(
        &mut game,
        stone,
        placeable("minecraft:pig_spawn_egg", 1, "minecraft:stone"),
    );
    assert_eq!(game.mobs.len(), mobs + 1, "a matching egg spawns");
    assert_eq!(held(&game), None, "a matching egg is used up");
}

/// A player who may build keeps every one of those uses.
#[test]
fn a_builder_use_on_is_unaffected() {
    let Some(mut game) = game() else {
        return;
    };
    game.set_mode(GameMode::Survival);

    let log = [2, 4, 3];
    place(&mut game, log, "minecraft:oak_log", &[]);
    click(&mut game, log, ItemStack::new("minecraft:iron_axe", 1));
    let stripped = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stripped_oak_log")
        .unwrap();
    assert_eq!(state(&game, log), stripped, "the axe strips");

    let copper = [4, 4, 3];
    place(&mut game, copper, "minecraft:copper_block", &[]);
    click(&mut game, copper, ItemStack::new("minecraft:honeycomb", 2));
    let waxed = game
        .pack
        .assets
        .registry
        .default_state("minecraft:waxed_copper_block")
        .unwrap();
    assert_eq!(state(&game, copper), waxed, "the honeycomb waxes");

    let wheat = [6, 4, 3];
    place(&mut game, wheat, "minecraft:wheat", &[]);
    click(&mut game, wheat, ItemStack::new("minecraft:bone_meal", 3));
    assert_eq!(
        held(&game),
        Some(("minecraft:bone_meal".into(), 2)),
        "the bone meal is used"
    );

    let vines = [8, 4, 3];
    let young = place(&mut game, vines, "minecraft:weeping_vines", &[("age", "0")]);
    click(&mut game, vines, ItemStack::new("minecraft:shears", 1));
    assert_ne!(state(&game, vines), young, "the shears crop");

    let dirt = [10, 4, 3];
    place(&mut game, dirt, "minecraft:dirt", &[]);
    click(
        &mut game,
        dirt,
        ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water"),
    );
    let mud = game
        .pack
        .assets
        .registry
        .default_state("minecraft:mud")
        .unwrap();
    assert_eq!(state(&game, dirt), mud, "the water bottle pours");

    let stone = [12, 4, 3];
    place(&mut game, stone, "minecraft:stone", &[]);
    let mobs = game.mobs.len();
    click(
        &mut game,
        stone,
        ItemStack::new("minecraft:pig_spawn_egg", 1),
    );
    assert_eq!(game.mobs.len(), mobs + 1, "the egg spawns");
    assert_eq!(held(&game), None, "the egg is used up");
}

/// `Minecraft.startUseItem`'s own prediction runs the same gate on the client:
/// in adventure mode off a matching `can_place_on` an axe does not predict the
/// strip (no sound, no swing), while a builder or a matching stack does.
#[test]
fn an_adventure_use_on_is_not_predicted_on_the_client() {
    let Some(mut game) = game() else {
        return;
    };
    game.set_mode(GameMode::Adventure);
    let log = [2, 4, 3];
    place(&mut game, log, "minecraft:oak_log", &[]);
    let axe = ItemStack::new("minecraft:iron_axe", 1);
    *game.held_mut() = Some(axe.clone());
    assert!(
        !predicts(&mut game, &axe, &hit(log)),
        "the axe's strip was predicted"
    );
    let matching = placeable("minecraft:iron_axe", 1, "minecraft:oak_log");
    *game.held_mut() = Some(matching.clone());
    assert!(
        predicts(&mut game, &matching, &hit(log)),
        "a matching can_place_on still predicts the strip"
    );
    game.set_mode(GameMode::Survival);
    assert!(
        predicts(&mut game, &matching, &hit(log)),
        "a builder's axe still predicts the strip"
    );
}

/// The client's `Item.useOn` prediction behind `ItemStack.useOn`'s adventure gate, which the
/// fork's use phase applies before it selects the prediction.
fn predicts(game: &mut Game<'_>, stack: &ItemStack, hit: &Hit) -> bool {
    game.with_hand(Hand::Main, |g| {
        g.may_use_on(hit.pos) && g.predict_item_on_block(stack, hit)
    })
}
