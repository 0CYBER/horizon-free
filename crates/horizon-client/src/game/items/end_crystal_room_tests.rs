//! `EndCrystalItem.useOn`'s refusals: the cell above must hold no entity at all
//! (`Level.getEntities(null, box)`, every non-spectator kind that is still in
//! the level), and each FAIL ends `Minecraft.startUseItem`'s click before the
//! off hand's turn.

use crate::game::{
    Audio, Game, Hit, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
    native_machines::tests::server_game,
};
use horizon_assets::resource_source::{ResourceRoot, ResourceSource};
use horizon_client::chunk::Chunk;
use horizon_client::entities::ItemEntity;
use horizon_client::first_person::Hand;
use horizon_client::items::ItemStack;
use horizon_client::mobs::Mob;
use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;

const POS: [i32; 3] = [2, 4, 3];
const ABOVE: [i32; 3] = [POS[0], POS[1] + 1, POS[2]];

fn top_face_hit() -> Hit {
    Hit {
        pos: POS,
        face: [0, 1, 0],
        point: [
            f64::from(POS[0]) + 0.5,
            f64::from(POS[1]) + 1.0,
            f64::from(POS[2]) + 0.5,
        ],
        inside: false,
    }
}

/// A dropped item in the cell above `POS`, at its floor.
fn item_in_cell() -> ItemEntity {
    ItemEntity::new(
        7,
        ItemStack::new("minecraft:dirt", 1),
        Vec3::new(
            f64::from(ABOVE[0]) + 0.5,
            f64::from(ABOVE[1]),
            f64::from(ABOVE[2]) + 0.5,
        ),
        Vec3::ZERO,
        0.0,
    )
}

/// A server game with `block` at `POS`, an end crystal in the main hand and
/// stone in the off hand, the click forced onto the block's top face.
fn crystal_game(block: &str) -> Game<'static> {
    let mut game = server_game();
    game.options.dedicated = None;
    let state = game.pack.assets.registry.default_state(block).unwrap();
    game.world.set(POS[0], POS[1], POS[2], state).unwrap();
    // The player stands clear of the cell above the block.
    game.player = Some(
        PlayerMotion::new(
            game.motion,
            Vec3::new(
                f64::from(POS[0]) + 0.5,
                f64::from(POS[1]),
                f64::from(POS[2]) + 2.5,
            ),
        )
        .unwrap(),
    );
    game.creative = false;
    *game.held_mut() = Some(ItemStack::new("minecraft:end_crystal", 1));
    game.offhand = Some(ItemStack::new("minecraft:stone", 1));
    game.forced_hit = Some(top_face_hit());
    game
}

/// This window as a client of a server (`MultiPlayerGameMode`), with `block` at
/// `POS` in its level, the crystal in the main hand and stone in the off hand.
fn client_game(block: &str) -> Game<'static> {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = ExecutionRole::Client;
    let res: &'static _ = Box::leak(Box::new(loaded.resources));
    let mut game = Game::new(
        res,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&ResourceSource::directory(root.clone())),
        None,
    )
    .unwrap();
    game.reset_entity_residency();
    game.native_lifecycle
        .begin_open(mod_api::Authority::IntegratedServer);
    let (client, _server) = horizon_protocol::connection::Pipe::pair();
    game.connect_local(client, "EndCrystalRoomTest").unwrap();
    game.remote.as_mut().unwrap().entity_id = 7;
    let state = game.pack.assets.registry.default_state(block).unwrap();
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(POS[0], POS[1], POS[2], state.0);
    game.world.chunks.insert((0, 0), std::sync::Arc::new(chunk));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.inventory[0] = Some(ItemStack::new("minecraft:end_crystal", 1));
    game.offhand = Some(ItemStack::new("minecraft:stone", 1));
    game.selected = 0;
    game.creative = false;
    game.forced_hit = Some(top_face_hit());
    game
}

fn air_above(game: &Game<'_>) -> bool {
    game.res
        .facts()
        .is_air(game.world.block(ABOVE[0], ABOVE[1], ABOVE[2]))
}

/// A dropped item is no mob, but it is an entity in the cell above the
/// obsidian, so `EndCrystalItem.useOn` is a FAIL. With the item gone the same
/// click places the crystal.
#[test]
#[ignore = "daily"]
fn a_dropped_item_in_the_cell_refuses_the_crystal() {
    let mut game = crystal_game("minecraft:obsidian");
    game.dropped.push(item_in_cell());
    assert_eq!(game.use_item().unwrap(), Some(Hand::Main));
    assert!(game.crystals.is_empty(), "no crystal on the dropped item");
    assert_eq!(
        game.held().map(|s| s.count),
        Some(1),
        "the crystal stayed in the main hand"
    );
    assert!(air_above(&game), "nothing was placed above the obsidian");
    assert_eq!(
        game.offhand.as_ref().map(|s| s.count),
        Some(1),
        "the refused click never reached the off hand's stone"
    );
    // The item gone, the same click places the crystal: the refusal above was
    // the room check and not another gate.
    game.dropped.clear();
    assert_eq!(game.use_item().unwrap(), Some(Hand::Main));
    assert_eq!(game.crystals.len(), 1, "the free cell takes the crystal");
    assert!(
        game.held().is_none(),
        "the placed crystal was spent from the main hand"
    );
}

/// `Entity.setRemoved` takes a mob out of the level at once, so its box no
/// longer counts even though its list entry is still there.
#[test]
#[ignore = "daily"]
fn a_removed_mob_in_the_cell_does_not_refuse_the_crystal() {
    let mut game = crystal_game("minecraft:obsidian");
    let kind = game
        .res
        .mob_types()
        .get("minecraft:zombie")
        .unwrap()
        .clone();
    let mut mob = Mob::new(
        7,
        kind,
        Vec3::new(
            f64::from(ABOVE[0]) + 0.5,
            f64::from(ABOVE[1]),
            f64::from(ABOVE[2]) + 0.5,
        ),
        0.0,
    );
    mob.removed = true;
    game.mobs.push(mob);
    assert_eq!(game.use_item().unwrap(), Some(Hand::Main));
    assert_eq!(
        game.crystals.len(),
        1,
        "a removed mob is out of the level and does not stand in the way"
    );
    assert!(game.held().is_none(), "the crystal was placed");
}

/// `EndCrystalItem.useOn`'s FAIL on a block that is neither obsidian nor
/// bedrock ends the click: the off hand's stone is not placed either.
#[test]
#[ignore = "daily"]
fn a_refused_crystal_ends_the_click_before_the_off_hand() {
    let mut game = crystal_game("minecraft:dirt");
    assert_eq!(
        game.use_item().unwrap(),
        Some(Hand::Main),
        "the FAIL ends the click on the main hand"
    );
    assert!(air_above(&game), "the off hand's stone was not placed");
    assert_eq!(
        game.offhand.as_ref().map(|s| s.count),
        Some(1),
        "the off hand's stone stayed put"
    );
    assert_eq!(
        game.held().map(|s| s.count),
        Some(1),
        "the crystal stayed in the main hand"
    );
}

/// The clicker's own client runs `EndCrystalItem.useOn` on its level before
/// the off hand gets a turn (`Minecraft.startUseItem`): a refused crystal ends
/// the click there and is not spent, a placed one is.
#[test]
#[ignore = "daily"]
fn the_clicking_client_ends_the_click_where_the_crystal_is_refused() {
    let mut game = client_game("minecraft:obsidian");
    game.dropped.push(item_in_cell());
    assert!(
        game.remote_use_block(Hand::Main),
        "the refused crystal ends the click, so the off hand is never tried"
    );
    assert_eq!(
        game.held().map(|s| s.count),
        Some(1),
        "a refused crystal is not spent on the client"
    );
    game.dropped.clear();
    assert!(
        game.remote_use_block(Hand::Main),
        "the placed crystal ends the click"
    );
    assert!(
        game.held().is_none(),
        "the client spends the crystal the server places"
    );
}

/// A dragon over the cell stands in it, as each of its parts does, and once it
/// has flown away it no longer does: the box is the dragon's own, not a copy
/// left where it spawned.
#[test]
#[ignore = "daily"]
fn a_dragon_over_the_cell_refuses_the_crystal_and_its_flight_frees_it() {
    let mut game = client_game("minecraft:obsidian");
    let over_cell = [
        f64::from(ABOVE[0]) + 0.5,
        f64::from(ABOVE[1]),
        f64::from(ABOVE[2]) + 0.5,
    ];
    game.spawn_entity(11, "minecraft:ender_dragon", over_cell, [0.0; 3]);
    assert!(
        game.remote_use_block(Hand::Main),
        "a dragon over the cell ends the click"
    );
    assert_eq!(
        game.held().map(|s| s.count),
        Some(1),
        "the dragon's box refuses the crystal"
    );
    // The server's next move: one step straight to the far side of the island.
    game.tick_remote_dragon(Some(([100.0, 100.0, 100.0], None, 1)));
    assert!(
        game.remote_use_block(Hand::Main),
        "the cell is free again"
    );
    assert!(
        game.held().is_none(),
        "a dragon that flew away does not hold the cell"
    );
}
