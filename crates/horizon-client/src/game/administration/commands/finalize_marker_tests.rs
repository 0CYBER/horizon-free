//! The `horizon:finalize` marker is worldgen's request for a deferred
//! `Mob.finalizeSpawn(STRUCTURE)` on a structure mob it could not finish, not a
//! player's to set: `/summon` with an NBT argument and `/data`'s loader
//! (`SummonCommand.createEntity` passes `finalize = false`; `Entity.load` reads
//! only the fields it knows) never honour it, while a mob the level loads from
//! its chunk still does.
use crate::game::{Audio, Game, Options, Progress, Settings, execution_role, load_resources};
use horizon_client::{
    chunk::Chunk,
    entity_nbt,
    mobs::{
        Mob, Species,
        finalize::{self, Attr},
    },
};
use horizon_core::nbt::Tag;
use std::{collections::HashSet, sync::Arc};

fn with_game(f: impl FnOnce(&mut Game<'_>)) {
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
    assert!(game.run_command("forceload add 0 0"));
    // Normal: a hostile mob cannot be summoned in peaceful.
    assert!(game.run_command("difficulty normal"));
    f(&mut game);
}

/// The zombie standing at `x` in the loaded chunk.
fn zombie<'a>(game: &'a Game<'_>, x: f64) -> &'a Mob {
    game.mobs
        .iter()
        .find(|m| m.species() == Species::Zombie && m.position.x == x)
        .unwrap_or_else(|| panic!("no zombie at x={x}"))
}

/// `Mob.finalizeSpawn` ran: only `Mob.finalizeSpawn` -> `MobSpawn` adds this
/// modifier, and a compound built by `Entity.load` never carries it.
fn finalized(m: &Mob) -> bool {
    finalize::modifier_amount(m, Attr::FollowRange, "minecraft:random_spawn_bonus").is_some()
}

/// The compound `game` would save for the entity with `uuid`.
fn saved_tag(game: &mut Game<'_>, uuid: [i32; 4]) -> Tag {
    game.entity_chunks(&HashSet::from([(0, 0)]), false)
        .iter()
        .flat_map(|(_, tags)| tags)
        .find(|t| entity_nbt::uuid(t) == Some(uuid))
        .unwrap_or_else(|| panic!("no saved entity with UUID {uuid:?}"))
        .clone()
}

#[test]
#[ignore = "daily"]
fn a_summoned_mob_ignores_a_typed_finalize_marker() {
    with_game(|game| {
        assert!(
            game.run_command(
                "summon minecraft:zombie 4.5 200 4.5 \
                 {UUID:[I;77,0,0,1],\"horizon:finalize\":1b,Health:1f,NoAI:1b,NoGravity:1b,\
                 PersistenceRequired:1b,\
                 Passengers:[{id:\"minecraft:zombie\",\"horizon:finalize\":1b,\
                 Pos:[4.5d,200d,4.5d]}]}"
            ),
            "the summon was refused"
        );
        assert!(
            !finalized(zombie(game, 4.5)),
            "a typed horizon:finalize ran Mob.finalizeSpawn on a summoned mob"
        );
        let passenger = game
            .mobs
            .iter()
            .find(|m| m.extra.mount.is_some())
            .expect("the passenger did not ride");
        assert!(
            !finalized(passenger),
            "a typed horizon:finalize ran Mob.finalizeSpawn on a passenger"
        );
        // Kept nowhere: a marker saved with the mob would be honoured by the
        // next load of its chunk.
        assert!(
            saved_tag(game, [77, 0, 0, 1])
                .get("horizon:finalize")
                .is_none(),
            "a typed horizon:finalize reached the save"
        );

        // A chunk's own mob still honours the marker: worldgen leaves it there
        // when it could not finalize the structure's mob itself.
        let queued = Tag::compound([
            ("id", Tag::String("minecraft:zombie".into())),
            (
                "Pos",
                Tag::List(
                    [8.5f64, 200.0, 4.5]
                        .map(|v| Tag::Double(v.to_bits()))
                        .to_vec(),
                ),
            ),
            ("UUID", Tag::IntArray(vec![78, 0, 0, 1])),
            ("horizon:finalize", Tag::Byte(1)),
            ("PersistenceRequired", Tag::Byte(1)),
        ]);
        game.world.queue_entity_nbt((0, 0), vec![queued]);
        game.spawn_loaded_entities();
        assert!(
            finalized(zombie(game, 8.5)),
            "a loaded structure mob lost its deferred Mob.finalizeSpawn"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_data_merged_finalize_marker_is_not_honoured() {
    with_game(|game| {
        assert!(
            game.run_command(
                "summon minecraft:zombie 4.5 200 4.5 {UUID:[I;79,0,0,1],NoAI:1b,NoGravity:1b,\
                 PersistenceRequired:1b}"
            ),
            "the summon was refused"
        );
        // `/data merge entity` loads the compound back into the mob
        // (`EntityDataAccessor.setData`), and `Entity.load` keeps only the
        // fields it reads: vanilla drops the key there, and Horizon must not
        // keep it for the next chunk load to honour.
        assert!(
            game.run_command(
                "data merge entity 0000004f-0000-0000-0000-000000000001 \
                 {\"horizon:finalize\":1b}"
            ),
            "the merge was refused"
        );
        // What the chunk writes must not arm the next load.
        let written: Vec<Tag> = game
            .entity_chunks(&HashSet::from([(0, 0)]), true)
            .into_iter()
            .flat_map(|(_, tags)| tags)
            .collect();
        let saved = written
            .iter()
            .find(|t| entity_nbt::uuid(t) == Some([79, 0, 0, 1]))
            .expect("the chunk wrote no zombie");
        assert!(
            saved.get("horizon:finalize").is_none(),
            "a data-merged horizon:finalize reached the save"
        );
        // Loaded again, it is the mob vanilla would build.
        game.world.queue_entity_nbt((0, 0), written);
        game.spawn_loaded_entities();
        assert!(
            !finalized(zombie(game, 4.5)),
            "a data-merged horizon:finalize ran Mob.finalizeSpawn on the next load"
        );
    });
}
