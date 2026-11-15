#![cfg(test)]
//! `on_chunk_unloaded` through a real game (arismoko/horizon#3145): a chunk with two mod block entities and a
//! vanilla chest leaves the world when its `/forceload` ticket goes, through the game's own
//! streaming, save and unload; the chunk comes back from its saved data; then the world closes.
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game_at};
use crate::game::world_loading::WorldRequest;
use horizon_client::containers::BlockEntity;
use horizon_core::nbt::{Compound, Tag};
use mod_api::{
    BlockEntityPart, BlockPart, BlocksEvent, Hooks, HostSide, OnChunkUnloadedEvent, Registrar,
    RegistrationContext,
};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const POT: &str = "unload_probe:pot";
/// Chunk (100, 100): far from the spawn, so only its ticket keeps it.
const CHUNK: (i32, i32) = (100, 100);
const POTS: [[i32; 3]; 2] = [[1601, 100, 1602], [1610, 120, 1613]];
const CHEST: [i32; 3] = [1605, 100, 1605];

/// What each call saw: the side, the position its handle names, its entity's id, and
/// whether the merge the part tried inside it was refused.
static CALLS: Mutex<Vec<(&'static str, [i32; 3], String, bool)>> = Mutex::new(Vec::new());

struct Pot;
#[mod_api::horizon_part]
impl BlockPart for Pot {}
#[mod_api::horizon_part]
impl BlockEntityPart for Pot {
    fn on_chunk_unloaded(&self, host: HostSide<'_>, event: &mut OnChunkUnloadedEvent<'_>) {
        let HostSide::Server(host) = host else {
            CALLS
                .lock()
                .unwrap()
                .push(("client", event.pos, event.entity.id().to_owned(), true));
            return;
        };
        let mut written = Compound::new();
        written.insert("written_on_unload".into(), Tag::Byte(1));
        let refused = event.block_entity.merge_data(host, written) != Ok(true);
        CALLS
            .lock()
            .unwrap()
            .push(("server", event.pos, event.entity.id().to_owned(), refused));
    }
}

struct Probe;
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let kind = r.block_entity_type("pot").blocks(&[POT]).build();
        r.block_entity_part(&kind, Pot).unwrap();
    }
    fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
        event
            .blocks
            .push(mod_api::Block::new(POT, "minecraft:stone"));
    }
}

fn request(world: &Path, create: bool) -> WorldRequest {
    let rules = std::sync::Arc::new(horizon_client::gamerules::RuleRegistry::vanilla());
    let options = horizon_client::title::WorldOptions {
        world_type: horizon_client::save::WorldPreset::Flat,
        rules: horizon_client::gamerules::GameRules::with_registry(rules.clone(), &[]),
        ..Default::default()
    };
    let mut request = WorldRequest::new(
        "unload",
        Some("unload"),
        Some(1),
        true,
        create.then_some(&options),
        rules,
    )
    .unwrap();
    request.save_root = world.to_owned();
    request
}

/// The prepared resources with the installed mods' content.
fn prepared_root() -> std::path::PathBuf {
    let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    crate::game::mods::content_root(&base, &Default::default())
        .unwrap()
        .prepared()
        .to_path_buf()
}

/// Runs the game's streaming until chunk `CHUNK` is resident (`true`) or gone, and
/// nothing is left on its way (a ticket removed mid-generation is another matter).
fn pump_until(game: &mut crate::game::Game<'_>, resident: bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while game.world.chunks.contains_key(&CHUNK) != resident || game.streaming.busy() {
        assert!(Instant::now() < deadline, "chunk {CHUNK:?} did not change");
        game.streaming.wait(Duration::from_millis(5));
        game.update_world();
    }
}

#[test]
#[ignore = "daily"]
fn each_mod_block_entity_of_an_unloading_chunk_is_told_once() {
    if run_fresh_cases(
        "game::native_hooks::on_chunk_unloaded_tests::each_mod_block_entity_of_an_unloading_chunk_is_told_once",
        "HORIZON_ON_CHUNK_UNLOADED_CHILD",
        &["game"],
    ) {
        return;
    }
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("unload_probe", Box::new(Probe))].into();
    mod_platform::__private::install(mods.clone());
    crate::game::api_values::install_parts(&mods).unwrap();
    let root = prepared_root();
    let saves = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!("on-chunk-unloaded-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&saves);

    with_game_at(root, |game| {
        game.load_world(request(&saves, true)).unwrap();
        game.set_allow_commands(true);
        assert!(game.run_command("forceload add 1600 1600"));
        pump_until(game, true);
        game.tick_world(true).unwrap();
        for [x, y, z] in POTS {
            assert!(game.run_command(&format!("setblock {x} {y} {z} {POT}")));
        }
        let [x, y, z] = CHEST;
        assert!(game.run_command(&format!("setblock {x} {y} {z} minecraft:chest")));
        game.tick_world(true).unwrap();
        assert!(CALLS.lock().unwrap().is_empty(), "told before any unload");

        // The ticket goes: the chunk is saved, its parts are told, its block entities dropped.
        assert!(game.run_command("forceload remove 1600 1600"));
        pump_until(game, false);
        let mut calls = std::mem::take(&mut *CALLS.lock().unwrap());
        calls.sort();
        assert_eq!(
            calls,
            POTS.map(|pos| ("server", pos, POT.to_owned(), true))
                .to_vec(),
            "one call for each pot, none for the chest"
        );
        for [x, y, z] in POTS {
            assert!(game.world.block_entities.get(&(x, y, z)).is_none());
        }
        // A few more streaming rounds tell nothing again.
        for _ in 0..20 {
            game.streaming.wait(Duration::from_millis(5));
            game.update_world();
        }
        assert!(CALLS.lock().unwrap().is_empty(), "told twice");

        // Back from its save: both pots, without what was written while it unloaded.
        assert!(game.run_command("forceload add 1600 1600"));
        pump_until(game, true);
        for [x, y, z] in POTS {
            let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&(x, y, z)) else {
                panic!("pot at {x} {y} {z} was not saved");
            };
            assert_eq!(entity.id(), POT);
            assert!(!entity.nbt.contains_key(&"written_on_unload".into()));
        }

        // Closing the world, its chunks with it, tells nothing.
        game.begin_closing_save(false).unwrap();
        let deadline = Instant::now() + Duration::from_secs(120);
        while game.pending_save.is_some() {
            assert!(Instant::now() < deadline, "the world did not close");
            game.poll_closing_save().unwrap();
        }
        assert!(
            game.world.chunks.is_empty(),
            "the world's chunks went with it"
        );
        assert!(
            CALLS.lock().unwrap().is_empty(),
            "told when the world closed"
        );
    });
    let _ = std::fs::remove_dir_all(&saves);
}

/// A client's copies: a column the server forgets tells its mod block entities, while
/// the client level going (a respawn, a dimension change, leaving) tells none.
#[test]
#[ignore = "daily"]
fn a_client_tells_a_forgotten_column_but_not_its_level_going() {
    use crate::game::execution_role::ExecutionRole;
    use horizon_client::{chunk::Chunk, containers::ModBlockEntity};
    use horizon_protocol::connection::Pipe;
    if run_fresh_cases(
        "game::native_hooks::on_chunk_unloaded_tests::a_client_tells_a_forgotten_column_but_not_its_level_going",
        "HORIZON_ON_CHUNK_UNLOADED_CLIENT_CHILD",
        &["game"],
    ) {
        return;
    }
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("unload_probe", Box::new(Probe))].into();
    mod_platform::__private::install(mods.clone());
    crate::game::api_values::install_parts(&mods).unwrap();
    with_game_at(prepared_root(), |game| {
        game.options.execution_role = ExecutionRole::Client;
        game.options.dedicated = None;
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "UnloadProbe").unwrap();
        game.remote.as_mut().unwrap().logged_in = true;
        let other = [1620, 64, 1600];
        for (x, z) in [CHUNK, (101, 100)] {
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(x, z));
        }
        for [x, y, z] in POTS.into_iter().chain([other]) {
            let pot = ModBlockEntity::new(POT.into(), Default::default());
            game.world
                .block_entities
                .insert((x, y, z), BlockEntity::Mod(Box::new(pot)));
        }
        let [x, y, z] = CHEST;
        game.world
            .block_entities
            .insert((x, y, z), BlockEntity::Chest(vec![]));

        game.forget_remote_columns(&mut [CHUNK].into_iter().collect(), true);
        let mut calls = std::mem::take(&mut *CALLS.lock().unwrap());
        calls.sort();
        assert_eq!(
            calls,
            POTS.map(|pos| ("client", pos, POT.to_owned(), true))
                .to_vec()
        );

        game.forget_all_remote_columns();
        assert!(game.world.block_entities.is_empty());
        assert!(CALLS.lock().unwrap().is_empty(), "told when the level went");
    });
}
