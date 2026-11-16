//! The adventure-mode gates on a right click: `TntBlock.prime` refuses to
//! light TNT for a player whose held stack's `can_break` does not name it,
//! and `ItemStack.useOn`'s `can_place_on` is what lets a placement through.
use crate::game::{Hit, game_tests::tests::server_game};
use horizon_client::command::args::GameMode;
use horizon_client::items::ItemStack;
use horizon_client::mobs::Who;
use horizon_core::nbt::Tag;

type Posted = (&'static str, [f64; 3], Option<Who>);

/// A server's game with nobody playing, or `None` without prepared resources.
fn game() -> Option<crate::game::Game<'static>> {
    std::env::var_os("HORIZON_TEST_RESOURCES")?;
    let resources = Box::leak(Box::new(None));
    let mut game = server_game(resources);
    game.build_registries();
    game.set_allow_commands(true);
    game.player = None;
    Some(game)
}

/// `stack` with a `can_break` or `can_place_on` component naming `block`.
fn stack_with(name: &str, component: &str, block: &str) -> ItemStack {
    let mut stack = ItemStack::new(name, 1);
    stack.data_mut().other.push((
        component.into(),
        Tag::compound([("blocks", Tag::List(vec![Tag::String(block.into())]))]),
    ));
    stack
}

/// A click on the block at `pos` from above, at the centre of its top face.
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

/// An adventure player right-clicks the block at `pos` with what is in hand,
/// as `Minecraft.startUseItem` does; whether the click consumed the action and
/// the game events the click posted.
fn click(game: &mut crate::game::Game<'_>, pos: [i32; 3]) -> (bool, Vec<Posted>) {
    let hit = hit(pos);
    game.forced_hit = Some(hit);
    crate::game::sculk::TEST_POSTED_EVENTS.with(|events| *events.borrow_mut() = Some(Vec::new()));
    let consumed = game.with_player_actor(|g| g.use_hand(Some(&hit))).unwrap();
    game.forced_hit = None;
    let events =
        crate::game::sculk::TEST_POSTED_EVENTS.with(|events| events.borrow_mut().take().unwrap());
    (consumed, events)
}

/// `TntBlock.prime`: an adventure player lights TNT only with a stack whose
/// `can_break` names it. A refused click is still `TntBlock.useItemOn`'s
/// SUCCESS, so nothing but the swing happens: no fuse, no sound, no wear.
#[test]
fn an_adventure_click_primes_tnt_only_with_a_can_break_that_names_it() {
    let Some(mut game) = game() else { return };
    assert!(
        game.ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::TNT_EXPLODES)
    );
    let pos = [2, 4, 2];
    let tnt = game
        .pack
        .assets
        .registry
        .default_state("minecraft:tnt")
        .unwrap();
    game.world.set(pos[0], pos[1], pos[2], tnt).unwrap();
    game.set_mode(GameMode::Adventure);
    let air = game.res.facts().air;

    // No `can_break`: the block stays, the fuse is not lit and the flint and
    // steel is not worn.
    *game.held_mut() = Some(ItemStack::new("minecraft:flint_and_steel", 1));
    let (consumed, events) = click(&mut game, pos);
    assert!(
        consumed,
        "TntBlock.useItemOn's SUCCESS still ends the click"
    );
    assert!(
        game.tnts.is_empty(),
        "an adventure player without can_break primes nothing"
    );
    assert_eq!(game.world.block(pos[0], pos[1], pos[2]), tnt);
    assert_eq!(game.held().map(|s| s.damage), Some(0));
    assert!(events.is_empty(), "no game event, got {events:?}");

    // A `can_break` naming TNT lets the same click prime it: the block goes,
    // the fuse event names the player, and the flint and steel wears by one.
    *game.held_mut() = Some(stack_with(
        "minecraft:flint_and_steel",
        "minecraft:can_break",
        "minecraft:tnt",
    ));
    let events = click(&mut game, pos).1;
    assert_eq!(game.tnts.len(), 1);
    assert_eq!(game.world.block(pos[0], pos[1], pos[2]), air);
    assert_eq!(game.held().map(|s| s.damage), Some(1));
    assert_eq!(
        events,
        vec![("minecraft:prime_fuse", [2.5, 4.5, 2.5], Some(Who::Player))]
    );
}

/// `ItemStack.useOn` passes in adventure mode unless the stack's
/// `can_place_on` names the clicked block, so only then does the client run
/// `BlockItem.place` and predict where the block goes.
#[test]
fn an_adventure_placement_is_predicted_when_can_place_on_names_the_clicked_block() {
    let Some(mut game) = game() else { return };
    let pos = [2, 4, 2];
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.world.set(pos[0], pos[1], pos[2], stone).unwrap();
    game.world
        .set(pos[0], pos[1] + 1, pos[2], game.res.facts().air)
        .unwrap();
    game.set_mode(GameMode::Adventure);
    let hit = hit(pos);
    // No `can_place_on`, and one that names another block: nothing to predict.
    for stack in [
        ItemStack::new("minecraft:stone", 1),
        stack_with(
            "minecraft:stone",
            "minecraft:can_place_on",
            "minecraft:dirt",
        ),
    ] {
        *game.held_mut() = Some(stack.clone());
        assert!(game.remote_can_place(&hit, &stack).is_none());
    }
    // One that names the clicked stone: the placement on top of it is the
    // client's own.
    let allowed = stack_with(
        "minecraft:stone",
        "minecraft:can_place_on",
        "minecraft:stone",
    );
    *game.held_mut() = Some(allowed.clone());
    let preview = game
        .remote_can_place(&hit, &allowed)
        .expect("an allowed placement is predicted");
    assert_eq!([preview.0.x, preview.0.y, preview.0.z], [2, 5, 2]);
    assert_eq!(
        game.world.block(pos[0], pos[1] + 1, pos[2]),
        game.res.facts().air,
        "the prediction is dry"
    );
    // `ItemStack.useOn` tests the stack the use names, so the other hand's
    // component neither allows nor refuses it.
    *game.held_mut() = Some(ItemStack::new("minecraft:stone", 1));
    assert!(game.remote_can_place(&hit, &allowed).is_some());
    assert!(
        game.remote_can_place(&hit, &ItemStack::new("minecraft:stone", 1))
            .is_none()
    );
}
