//! `CushionItem.useOn`'s `createDefaultStackConfig` (`Entity.applyImplicitComponents`
//! copies the stack's `CUSTOM_NAME`), `Cushion.dropItem`'s suppression for a
//! player with infinite materials, and `BlockAttachedEntity.tick`'s
//! `discard()` when the support goes away (no `entity_die` game event).
use super::super::*;
use horizon_client::chunk::Chunk;
use horizon_client::items::ItemStack;
use horizon_client::mobs::Who;
use horizon_client::text::Text;
use horizon_core::geometry::Vec3;
use horizon_core::nbt::Tag;
use horizon_core::position::ChunkPos;
use std::sync::Arc;

/// The cushion's position: `Vec3.atCenterOfWithY(4 199 4, 200.0)`.
const AT: [f64; 3] = [4.5, 200.0, 4.5];

fn game() -> Game<'static> {
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
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("forceload add 0 0"));
    assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
    // `Cushion.dropItem` only drops under `ENTITY_DROPS`, and the blast below
    // only reaches the cushion under `MOB_GRIEFING` (both default on).
    assert!(
        game.ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::ENTITY_DROPS)
            && game
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::MOB_GRIEFING)
    );
    game
}

/// The top face of the stone block a cushion goes on.
fn top_hit() -> Hit {
    Hit {
        pos: [4, 199, 4],
        face: [0, 1, 0],
        point: AT,
        inside: false,
    }
}

/// `/give @s minecraft:white_cushion[minecraft:custom_name='"Seat"']`.
fn named_stack() -> ItemStack {
    let mut stack = ItemStack::new("minecraft:white_cushion", 1);
    stack.data_mut().custom_name = Some(Tag::String("Seat".into()));
    stack
}

fn capture() {
    super::super::sculk::TEST_POSTED_EVENTS.with(|events| *events.borrow_mut() = Some(Vec::new()));
}

fn events() -> Vec<(&'static str, [f64; 3], Option<Who>)> {
    super::super::sculk::TEST_POSTED_EVENTS.with(|events| events.borrow_mut().take().unwrap())
}

/// `CushionItem.useOn` builds the entity with `EntityType.createDefaultStackConfig`,
/// whose `applyComponentsFromItemStack` copies `CUSTOM_NAME` onto the cushion.
#[test]
#[ignore = "daily"]
fn a_placed_cushion_keeps_its_stacks_custom_name() {
    let mut game = game();
    game.creative = true;
    assert!(game.place_cushion(&top_hit(), &named_stack()));
    assert_eq!(game.cushions.len(), 1);
    assert_eq!(game.cushions[0].position, Vec3::new(AT[0], AT[1], AT[2]));
    assert_eq!(
        game.cushions[0].custom_name,
        Some(Tag::String("Seat".into())),
        "the placed cushion is named after its stack"
    );
    assert_eq!(
        game.cushions[0]
            .item_stack_with_data()
            .custom_name()
            .as_deref(),
        Some("Seat"),
        "and `getCushionItemStackWithData` hands the name back"
    );
    // An unnamed stack names nothing.
    let plain = ItemStack::new("minecraft:white_cushion", 1);
    game.cushions.clear();
    assert!(game.place_cushion(&top_hit(), &plain));
    assert_eq!(game.cushions[0].custom_name, None);
}

/// `Cushion.dropItem` drops nothing when the cause is a player with infinite
/// materials (`hasInfiniteMaterials`, the TNT's owner for a blast), and the
/// item otherwise. `BlockAttachedEntity.hurtServer` passes the damage source's
/// causing entity.
#[test]
#[ignore = "daily"]
fn a_creative_blast_leaves_no_cushion_item() {
    let mut game = game();
    // (creative, the TNT's owner is a player, the guest behind it, drops)
    for (creative, owner_player, guest, dropped) in [
        // A creative player's TNT: no item.
        (true, true, None, 0),
        // A survival player's: one.
        (false, true, None, 1),
        // A mob's TNT (`PrimedTnt.getOwner`): not a player, so one, whatever
        // the player in place holds.
        (true, false, None, 1),
        // A creative guest's, on the guest's own turn (the hub is out of
        // `self.lan` while it acts): no item.
        (true, true, Some(7), 0),
    ] {
        game.creative = creative;
        game.cushions.clear();
        game.dropped.clear();
        assert!(game.place_cushion(&top_hit(), &named_stack()));
        // `Explosion.getIndirectSourceEntity` of a player's TNT is the player.
        game.combat = Default::default();
        game.combat.blast_owner = Some(if owner_player {
            (1, Text::literal("Steve"))
        } else {
            (700, Text::literal("Zombie"))
        });
        game.combat.blast_owner_player = owner_player;
        game.combat.blast_guest = guest;
        game.lan_acting = guest;
        game.server_explode(
            [4.5, 200.5, 4.5],
            2.0,
            super::super::explosion::BlockInteraction::Keep,
            false,
        )
        .unwrap();
        game.lan_acting = None;
        assert!(game.cushions.is_empty(), "the blast breaks the cushion");
        assert_eq!(
            game.dropped.len(),
            dropped,
            "creative={creative} owner_player={owner_player} guest={guest:?}: \
             the blast's cause decides the drop"
        );
        if dropped == 1 {
            assert_eq!(
                game.dropped[0].stack.custom_name().as_deref(),
                Some("Seat"),
                "a blast that drops the cushion drops it named"
            );
        }
    }
}

/// `BlockAttachedEntity.tick`: a cushion whose support is gone is `discard()`ed
/// and then dropped with `causedBy = null`, which posts no `entity_die`;
/// `kill` (a player's break) still does.
#[test]
#[ignore = "daily"]
fn a_cushion_that_loses_its_support_is_discarded_silently() {
    let mut game = game();
    game.creative = true;
    assert!(game.place_cushion(&top_hit(), &named_stack()));
    capture();
    game.break_cushion(0, true);
    assert_eq!(
        events(),
        vec![("minecraft:entity_die", AT, Some(Who::Player))],
        "a player's break kills the cushion"
    );
    assert!(game.dropped.is_empty(), "a creative breaker gets no item");

    assert!(game.place_cushion(&top_hit(), &named_stack()));
    assert!(game.run_command("setblock 4 199 4 air strict"));
    // The next check (100 ticks) finds the anchor gone.
    game.cushions[0].check_interval = 100;
    capture();
    game.tick_cushions();
    assert!(game.cushions.is_empty(), "the cushion is gone");
    assert_eq!(events(), [], "a discarded cushion posts no game event");
    assert_eq!(game.dropped.len(), 1, "it still drops its item");
    assert_eq!(game.dropped[0].stack.custom_name().as_deref(), Some("Seat"));
}
