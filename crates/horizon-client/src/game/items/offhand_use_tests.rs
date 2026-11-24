//! `Minecraft.startUseItem`'s two hands: `ServerPlayerGameMode.useItemOn`
//! runs the block's own `useItemOn` for either hand, and
//! `Equippable.swapWithEquipmentSlot`'s FAIL does not consume the click, so
//! the off hand still gets its turn after a refused swap.
use super::*;
use crate::game::{Game, Hit, native_machines::tests::server_game};
use horizon_client::{items::ItemStack, mobs::Who, motion::PlayerMotion};
use horizon_core::geometry::Vec3;
use horizon_world::registry::BlockStateId;

const POS: [i32; 3] = [2, 4, 3];

type Posted = (&'static str, [f64; 3], Option<Who>);

/// A server game with a full cake at `POS`, `main` in the main hand and `off`
/// in the off hand, the click forced onto the cake's top; `None` without
/// prepared resources.
fn cake_game(main: &str, off: &str) -> Option<Game<'static>> {
    std::env::var_os("HORIZON_TEST_RESOURCES")?;
    let mut game = server_game();
    let cake = game.pack.assets.registry.default_state("minecraft:cake")?;
    let air = game.res.facts().air;
    game.world.set(POS[0], POS[1], POS[2], cake).unwrap();
    game.world.set(POS[0], POS[1] + 1, POS[2], air).unwrap();
    // A player who cannot eat (`Player.canEat(false)`: full and not
    // invulnerable), so a hand's `useWithoutItem` passes on the cake.
    let mut player = PlayerMotion::new(
        game.motion,
        Vec3::new(
            f64::from(POS[0]) + 0.5,
            f64::from(POS[1]),
            f64::from(POS[2]) + 2.5,
        ),
    )
    .unwrap();
    player.health.food.food = 20;
    game.player = Some(player);
    game.creative = false;
    *game.held_mut() = Some(ItemStack::new(main, 1));
    game.offhand = Some(ItemStack::new(off, 1));
    game.forced_hit = Some(Hit {
        pos: POS,
        face: [0, 1, 0],
        point: [
            f64::from(POS[0]) + 0.5,
            f64::from(POS[1]) + 1.0,
            f64::from(POS[2]) + 0.5,
        ],
        inside: false,
    });
    Some(game)
}

/// `name` with one property set (`minecraft:candle` with `lit=true`).
fn state_with(game: &Game<'_>, name: &str, property: &str, value: &str) -> BlockStateId {
    let mut state = horizon_world::chunk::BlockState {
        name: name.into(),
        properties: Default::default(),
    };
    state.properties.insert(property.into(), value.into());
    game.pack.assets.registry.resolve(&state).unwrap()
}

/// The block state of the default state of `name`.
fn state(game: &Game<'_>, name: &str) -> BlockStateId {
    game.pack
        .assets
        .registry
        .default_state(name)
        .unwrap_or_else(|| panic!("no {name}"))
}

/// The game events the click posted.
fn posted() -> Vec<Posted> {
    crate::game::sculk::TEST_POSTED_EVENTS
        .with(|events| events.borrow_mut().take().unwrap_or_default())
}

/// `CakeBlock.useItemOn` takes the candle from whichever hand holds it: the
/// main hand's stick only passes (`TRY_WITH_EMPTY_HAND`, then the
/// `useWithoutItem` that cannot eat), so the off hand's turn turns the cake
/// into a candle cake, uses the candle up and posts BLOCK_CHANGE.
#[test]
fn the_off_hands_candle_goes_on_the_cake() {
    let Some(mut game) = cake_game("minecraft:stick", "minecraft:candle") else {
        return;
    };
    let candle_cake = state(&game, "minecraft:candle_cake");
    let air = game.res.facts().air;
    crate::game::sculk::TEST_POSTED_EVENTS.with(|e| *e.borrow_mut() = Some(Vec::new()));

    let used = game.use_item().unwrap();

    assert_eq!(used, Some(Hand::Off), "the off hand took the click");
    assert_eq!(
        game.world.block(POS[0], POS[1], POS[2]),
        candle_cake,
        "the off hand's candle went on the cake"
    );
    assert!(game.offhand.is_none(), "the candle was used up");
    assert_eq!(
        game.held().map(|s| s.item.to_string()).as_deref(),
        Some("minecraft:stick"),
        "the main hand kept its stick"
    );
    assert_eq!(
        game.world.block(POS[0], POS[1] + 1, POS[2]),
        air,
        "the candle was not placed above the cake"
    );
    assert_eq!(
        posted(),
        vec![(
            "minecraft:block_change",
            [
                f64::from(POS[0]) + 0.5,
                f64::from(POS[1]) + 0.5,
                f64::from(POS[2]) + 0.5
            ],
            Some(Who::Player)
        )],
        "CakeBlock.useItemOn's gameEvent(player, BLOCK_CHANGE, pos)"
    );
}

/// `Equippable.swapWithEquipmentSlot`'s FAIL for the helmet already worn (or
/// bound) does not consume the click, so `startUseItem` goes on to the off
/// hand: its candle still goes on the cake, and only the off hand swings.
#[test]
fn a_refused_swap_leaves_the_other_hand_its_turn() {
    let Some(mut game) = cake_game("minecraft:diamond_helmet", "minecraft:candle") else {
        return;
    };
    let worn = ItemStack::new("minecraft:diamond_helmet", 1);
    game.armor[0] = Some(worn.clone());
    let candle_cake = state(&game, "minecraft:candle_cake");
    let swings = game.swings_started;

    let used = game.use_item().unwrap();

    assert_eq!(used, Some(Hand::Off), "the off hand took the click");
    assert_eq!(
        game.world.block(POS[0], POS[1], POS[2]),
        candle_cake,
        "the off hand's candle went on the cake"
    );
    assert_eq!(
        game.held().map(|s| s.item.to_string()).as_deref(),
        Some("minecraft:diamond_helmet"),
        "the refused swap left the helmet in hand"
    );
    assert!(
        game.armor[0]
            .as_ref()
            .is_some_and(|s| s.same_item_same_components(&worn)),
        "the worn helmet stayed on"
    );
    assert_eq!(
        game.swings_started - swings,
        1,
        "only the off hand swung (a FAIL does not swing)"
    );
}

/// A window that joined a server (so its own predictions run), with a player
/// at `POS` whose click is forced onto that block's top, `main` in the main
/// hand and `off` in the off hand; `None` without prepared resources.
fn client_game(main: &str, off: Option<&str>) -> Option<Game<'static>> {
    use crate::game::{
        Audio, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
    };
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES")?);
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
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
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    let (client, _server) = horizon_protocol::connection::Pipe::pair();
    game.connect_local(client, "OffhandTest").unwrap();
    game.world.chunks.insert(
        (0, 0),
        std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
    );
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
    game.selected = 0;
    *game.held_mut() = Some(ItemStack::new(main, 1));
    game.offhand = off.map(|off| ItemStack::new(off, 1));
    game.forced_hit = Some(Hit {
        pos: POS,
        face: [0, 1, 0],
        point: [
            f64::from(POS[0]) + 0.5,
            f64::from(POS[1]) + 1.0,
            f64::from(POS[2]) + 0.5,
        ],
        inside: false,
    });
    Some(game)
}

/// `MultiPlayerGameMode.useItemOn` runs the block's own `useItemOn` for the
/// off hand too (only `useWithoutItem` is the main hand's): this client's
/// prediction of the off hand's candle on the cake is SUCCESS, which ends the
/// click and swings the off hand, so no UseItem follows for it.
#[test]
fn the_prediction_runs_the_off_hands_block_use() {
    let Some(mut game) = client_game("minecraft:stick", Some("minecraft:candle")) else {
        return;
    };
    let cake = state(&game, "minecraft:cake");
    let air = game.res.facts().air;
    game.world.set(POS[0], POS[1], POS[2], cake).unwrap();
    game.world.set(POS[0], POS[1] + 1, POS[2], air).unwrap();
    let swings = game.swings_started;
    let done = game.remote_use_block(Hand::Off);
    assert!(done, "the off hand's click ended on the block");
    assert_eq!(
        game.swings_started - swings,
        1,
        "CakeBlock.useItemOn's SUCCESS swings the off hand"
    );
    assert_eq!(
        game.swing.current.map(|s| s.hand),
        Some(Hand::Off),
        "the off hand swung, not the main hand"
    );
}

/// `CandleBlock.useItemOn` snuffs a lit candle for either hand: an empty off
/// hand (a stick in the main one) ends the click with SUCCESS, swings the off
/// hand and draws the wick's smoke on this client.
#[test]
fn the_prediction_snuffs_the_off_hands_lit_candle() {
    let Some(mut game) = client_game("minecraft:stick", None) else {
        return;
    };
    let lit = state_with(&game, "minecraft:candle", "lit", "true");
    game.world.set(POS[0], POS[1], POS[2], lit).unwrap();
    game.particles.requested = Some(Vec::new());
    let swings = game.swings_started;
    let done = game.remote_use_block(Hand::Off);
    assert!(done, "the off hand's click ended on the block");
    assert_eq!(
        game.swings_started - swings,
        1,
        "CandleBlock.useItemOn's SUCCESS swings the off hand"
    );
    let asked = game.particles.requested.take().unwrap();
    assert!(
        asked
            .iter()
            .any(|(options, ..)| options.ty == horizon_client::particles::ParticleType::Smoke),
        "the wick's smoke: {:?}",
        asked.iter().map(|(o, ..)| o.ty).collect::<Vec<_>>()
    );
}

/// The same turn through a guest's `UseItemOn` packet
/// (`ServerGamePacketListenerImpl.handleUseItemOn`: `use_item` with the
/// packet's hand), which is what a LAN guest and the window's own client send:
/// the server runs the block's own `useItemOn` for the off hand too.
#[test]
fn the_guests_off_hand_candle_goes_on_the_cake() {
    let Some(mut game) = cake_game("minecraft:stick", "minecraft:candle") else {
        return;
    };
    let candle_cake = state(&game, "minecraft:candle_cake");
    game.guest_use = Some(GuestUse::OnBlock(Hand::Off));
    let used = game.use_item().unwrap();
    assert_eq!(
        used,
        Some(Hand::Off),
        "the off hand's packet took the click"
    );
    assert_eq!(
        game.world.block(POS[0], POS[1], POS[2]),
        candle_cake,
        "the off hand's candle went on the cake"
    );
    assert!(game.offhand.is_none(), "the candle was used up");
}
