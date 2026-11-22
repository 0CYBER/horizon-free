//! `/summon item` and `/summon experience_orb`: `SummonCommand.createEntity`
//! builds both from `{id:..}` through their `(EntityType, Level)` factory, so
//! the orb is never unstuck, and an empty item is discarded while loading
//! (`ItemEntity.readAdditionalSaveData`) yet the command still succeeds.
use super::super::{Audio, Game, Options, Progress, Settings, execution_role, load_resources};
use super::Target;
use horizon_client::chunk::Chunk;
use horizon_client::text::Text;
use horizon_core::geometry::Vec3;
use std::sync::Arc;

fn with_game(test: impl FnOnce(&mut Game<'_>)) {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
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
    let mut game = Game::new(
        &loaded.resources,
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
    test(&mut game);
}

/// `ItemEntity(EntityType, Level)` leaves the stack `ItemStack.EMPTY`, and
/// `readAdditionalSaveData` discards such an entity, so `addEntity` adds
/// nothing; `tryAddFreshEntityWithPassengers` still answers true, so both
/// summon forms report success. The entity id is taken all the same, and the
/// name is the air item's: `ItemStack.EMPTY.getItemName()` is "Air".
#[test]
#[ignore = "daily"]
fn summon_item_succeeds_with_and_without_nbt_without_leaving_an_entity() {
    with_game(|game| {
        for command in [
            "summon minecraft:item 4.5 200 4.5",
            "summon minecraft:item 4.5 200 4.5 {}",
        ] {
            let before = game.next_entity;
            assert!(
                game.run_command(command),
                "{command}: {:?}",
                game.ui.chat.recent_lines(3)
            );
            assert_eq!(game.next_entity, before + 1, "{command}");
            assert!(game.dropped.is_empty(), "{command}");
            assert_eq!(
                game.ui.chat.recent_components(1),
                vec![serde_json::json!({
                    "translate": "commands.summon.success",
                    "with": [{ "translate": "block.minecraft.air" }],
                })],
                "{command}"
            );
        }
    });
}

/// `ExperienceOrb(EntityType, Level)` is the factory `EntityType` uses: the
/// position is `SummonCommand`'s `snapTo` and nothing unsticks it, so an orb
/// summoned against the top of a block stays where it was asked for.
#[test]
#[ignore = "daily"]
fn a_summoned_orb_keeps_the_position_it_was_asked_for() {
    with_game(|game| {
        assert!(game.run_command("setblock 4 100 4 stone"));
        // Its box (0.5 tall) reaches into the stone's top 0.2, which is what
        // makes the constructor's unstuck search move it in the first place.
        assert!(
            game.run_command("summon minecraft:experience_orb 4.5 100.8 4.5"),
            "{:?}",
            game.ui.chat.recent_lines(3)
        );
        let orb = game.orbs.last().expect("an orb");
        assert_eq!(orb.position, Vec3::new(4.5, 100.8, 4.5));
        assert_eq!(orb.velocity, Vec3::ZERO);
        assert_eq!(orb.points(), 0);
        assert_eq!(orb.count, 1);
    });
}

/// `Entity.getDisplayName` of an ominous item spawner: its CustomName from the
/// compound it keeps (`OminousItemSpawner.raw`), else the entity type's name.
#[test]
#[ignore = "daily"]
fn an_ominous_item_spawner_is_named_by_its_custom_name() {
    with_game(|game| {
        assert!(
            game.run_command(
                r#"summon minecraft:ominous_item_spawner 4.5 200 4.5 {CustomName:"sw"}"#
            ),
            "{:?}",
            game.ui.chat.recent_lines(3)
        );
        let id = game.ominous_spawners.last().expect("a spawner").id;
        assert_eq!(game.target_name(Target::Spawner(id)), Text::literal("sw"));
    });
}
