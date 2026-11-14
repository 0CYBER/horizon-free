//! Levels a mod opens at run time: the table's rules, and a mod's calls against a game.

use super::native_entity_refs::tests::with_game;
use super::runtime_levels::RuntimeTable;
use mod_api::{
    Hooks, HostSide, LevelLoadEvent, LevelRef, LevelTickPostEvent, LevelUnloadEvent, ModEntry,
    RuntimeGenerator, RuntimeLevel, RuntimeLevelError,
};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

fn flat() -> RuntimeLevel {
    RuntimeLevel::new("minecraft:overworld", RuntimeGenerator::Flat, 1)
}

#[test]
fn the_cap_refuses_the_open_past_it_and_a_removal_frees_a_place() {
    let mut table = RuntimeTable::default();
    table.set_cap(2);
    assert!(table.queue_open("test", "cap:a", flat()).is_ok());
    assert!(table.queue_open("test", "cap:b", flat()).is_ok());
    assert_eq!(
        table.queue_open("test", "cap:c", flat()),
        Err(RuntimeLevelError::CapReached)
    );
    table.remove("cap:a");
    assert!(table.queue_open("test", "cap:c", flat()).is_ok());
}

#[test]
fn the_default_cap_is_64() {
    let mut table = RuntimeTable::default();
    for n in 0..64 {
        assert!(
            table
                .queue_open("test", &format!("cap:l{n}"), flat())
                .is_ok()
        );
    }
    assert_eq!(
        table.queue_open("test", "cap:l64", flat()),
        Err(RuntimeLevelError::CapReached)
    );
}

#[test]
fn an_id_is_open_once_and_each_open_has_its_own_generation() {
    let mut table = RuntimeTable::default();
    let first = table.queue_open("test", "gen:a", flat()).unwrap();
    assert_eq!(
        table.queue_open("test", "gen:a", flat()),
        Err(RuntimeLevelError::AlreadyOpen)
    );
    assert_eq!(table.generation_of("gen:a"), first);
    table.remove("gen:a");
    let second = table.queue_open("test", "gen:a", flat()).unwrap();
    assert_ne!(first, second);
    assert_eq!(table.generation_of("gen:a"), second);
    assert_eq!(table.generation_of("minecraft:the_nether"), 0);
}

#[test]
fn a_level_is_open_after_its_request_is_applied_and_until_its_close_is() {
    let mut table = RuntimeTable::default();
    table.queue_open("test", "state:a", flat()).unwrap();
    assert!(!table.is_open("state:a"));
    assert_eq!(
        table.queue_unload("state:a"),
        Err(RuntimeLevelError::NotOpen)
    );
    table.mark_open("state:a");
    assert!(table.is_open("state:a"));
    assert_eq!(table.queue_unload("state:a"), Ok(()));
    assert!(
        table.is_open("state:a"),
        "still open until the close is applied"
    );
    assert_eq!(
        table.queue_unload("state:a"),
        Err(RuntimeLevelError::NotOpen)
    );
    assert_eq!(
        table.queue_unload("state:none"),
        Err(RuntimeLevelError::NotOpen)
    );
}

type Seen<T> = Arc<Mutex<Vec<T>>>;

/// What the mod does in each level tick: open `id`, delete it, or nothing.
const OPEN: u8 = 0;
const DELETE: u8 = 1;
const IDLE: u8 = 2;

/// A mod that opens `id` in `level_tick_post` and says what each call answered.
struct Opener {
    id: &'static str,
    level: RuntimeLevel,
    mode: Arc<AtomicU8>,
    deletes: Seen<Result<(), RuntimeLevelError>>,
    answers: Seen<Result<LevelRef, RuntimeLevelError>>,
    open_right_after: Seen<bool>,
    loads: Seen<String>,
    load_contexts: Seen<(String, String, bool)>,
    unloads: Seen<String>,
}

#[mod_api::__subscriptions]
impl Hooks for Opener {
    fn level_tick_post(&self, host: HostSide, _: &mut LevelTickPostEvent) {
        let HostSide::Server(host) = host else {
            return;
        };
        match self.mode.load(Ordering::SeqCst) {
            OPEN => {}
            DELETE => {
                let answer = host.runtime_levels().delete(self.id);
                self.deletes.lock().unwrap().push(answer);
                let open = host.runtime_levels().is_open(self.id);
                self.open_right_after.lock().unwrap().push(open);
                return;
            }
            _ => return,
        }
        let answer = host.runtime_levels().open(self.id, self.level.clone());
        self.answers.lock().unwrap().push(answer);
        let open = host.runtime_levels().is_open(self.id);
        self.open_right_after.lock().unwrap().push(open);
    }
    fn level_load(&self, host: &mut dyn mod_api::ServerHost, event: &mut LevelLoadEvent<'_>) {
        self.loads.lock().unwrap().push(event.dimension.to_owned());
        self.load_contexts.lock().unwrap().push((
            event.dimension.to_owned(),
            host.dimension().to_owned(),
            host.level(event.dimension).unwrap().in_place(host),
        ));
    }
    fn level_unload(&self, _: &mut dyn mod_api::ServerHost, event: &mut LevelUnloadEvent<'_>) {
        self.unloads
            .lock()
            .unwrap()
            .push(event.dimension.to_owned());
    }
}

struct Fixture {
    mode: Arc<AtomicU8>,
    deletes: Seen<Result<(), RuntimeLevelError>>,
    answers: Seen<Result<LevelRef, RuntimeLevelError>>,
    open_right_after: Seen<bool>,
    loads: Seen<String>,
    load_contexts: Seen<(String, String, bool)>,
    unloads: Seen<String>,
}

/// A mod named `name` that opens `id` each time a level ticks, in a game, for `test`.
fn with_opener(
    name: &'static str,
    id: &'static str,
    level: RuntimeLevel,
    test: impl FnOnce(&mut super::Game<'_>, &Fixture),
) {
    let fixture = Fixture {
        mode: Arc::new(AtomicU8::new(OPEN)),
        deletes: Seen::default(),
        answers: Seen::default(),
        open_right_after: Seen::default(),
        loads: Seen::default(),
        load_contexts: Seen::default(),
        unloads: Seen::default(),
    };
    let opener = Opener {
        id,
        level,
        mode: fixture.mode.clone(),
        deletes: fixture.deletes.clone(),
        answers: fixture.answers.clone(),
        open_right_after: fixture.open_right_after.clone(),
        loads: fixture.loads.clone(),
        load_contexts: fixture.load_contexts.clone(),
        unloads: fixture.unloads.clone(),
    };
    let mods = vec![ModEntry::new(name, Box::new(opener))].into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            // Levels' folders go under a world root of the test's own, never the checkout.
            let root = TempRoot::new(name);
            game.world_root = root.0.clone();
            test(game, &fixture);
        });
    });
}

/// A world root of one test, removed when the test ends, even by a failed assertion.
struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "horizon-runtime-levels-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `RuntimeLevels::unload` as the mod `name` calls it.
fn unload_as(
    game: &mut super::Game<'_>,
    name: &'static str,
    id: &str,
) -> Result<(), RuntimeLevelError> {
    let _call = mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
        id: name,
        hook: "level_tick_post",
        part: None,
    });
    game.runtime_level_unload(id)
}

/// `RuntimeLevels::delete` as the mod `name` calls it.
fn delete_as(
    game: &mut super::Game<'_>,
    name: &'static str,
    id: &str,
) -> Result<(), RuntimeLevelError> {
    let _call = mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
        id: name,
        hook: "level_tick_post",
        part: None,
    });
    game.runtime_level_delete(id)
}

/// Generate the chunk (0, 0) of the open level `id`, put `block` at (1, -50, 1) and leave the
/// level again.
fn place_block(game: &mut super::Game<'_>, id: &str, block: &str) {
    let state = game.pack.assets.registry.default_state(block).unwrap();
    let dim = super::Dim::extra(id).unwrap();
    game.enter_level(dim).unwrap();
    game.streaming.forced.insert((0, 0));
    for _ in 0..2000 {
        game.update_world();
        if game.world.chunks.contains_key(&(0, 0)) {
            break;
        }
        game.streaming.wait(std::time::Duration::from_millis(5));
    }
    assert!(
        game.world.chunks.contains_key(&(0, 0)),
        "the chunk generated"
    );
    game.world.set(1, -50, 1, state).unwrap();
    game.enter_level(super::Dim::Overworld).unwrap();
}

/// The block at (1, -50, 1) of the open level `id`, loading it as `place_block` does.
fn block_at(game: &mut super::Game<'_>, id: &str) -> horizon_world::registry::BlockStateId {
    let dim = super::Dim::extra(id).unwrap();
    game.enter_level(dim).unwrap();
    game.streaming.forced.insert((0, 0));
    for _ in 0..2000 {
        game.update_world();
        if game.world.chunks.contains_key(&(0, 0)) {
            break;
        }
        game.streaming.wait(std::time::Duration::from_millis(5));
    }
    assert!(game.world.chunks.contains_key(&(0, 0)), "the chunk loaded");
    let block = game.world.block(1, -50, 1);
    game.enter_level(super::Dim::Overworld).unwrap();
    block
}

/// A file with some bytes in it, its folders made.
fn write_file(path: &std::path::Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"x").unwrap();
}

fn tick_post(game: &mut super::Game<'_>) {
    let home = game.level("minecraft:overworld").unwrap();
    game.native_level_tick_post(home);
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_mod_opens_only_ids_in_its_own_namespace() {
    for (id, expected) in [
        ("other:x", Err(RuntimeLevelError::OutsideNamespace)),
        ("mymod:../x", Err(RuntimeLevelError::InvalidId)),
        ("mymod:a//b", Err(RuntimeLevelError::InvalidId)),
        (
            "minecraft:overworld",
            Err(RuntimeLevelError::OutsideNamespace),
        ),
        // Names Windows maps to a device or to another folder: refused before any folder exists.
        ("mymod:con", Err(RuntimeLevelError::InvalidId)),
        ("mymod:a/nul.txt", Err(RuntimeLevelError::InvalidId)),
        ("mymod:com1", Err(RuntimeLevelError::InvalidId)),
        ("mymod:lpt9/x", Err(RuntimeLevelError::InvalidId)),
        ("mymod:a.", Err(RuntimeLevelError::InvalidId)),
        ("mymod:x/b.", Err(RuntimeLevelError::InvalidId)),
        ("mymod:console", Ok(())),
        ("mymod:com10", Ok(())),
        ("mymod:instance/ab", Ok(())),
    ] {
        with_opener("mymod", id, flat(), |game, seen| {
            tick_post(game);
            let answer = seen.answers.lock().unwrap()[0].map(|_| ());
            assert_eq!(answer, expected, "{id}");
        });
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn nothing_opens_inside_a_tick_and_the_queue_is_applied_after_the_levels() {
    with_opener("phase", "phase:a", flat(), |game, seen| {
        tick_post(game);
        assert!(seen.answers.lock().unwrap()[0].is_ok());
        assert_eq!(*seen.open_right_after.lock().unwrap(), [false]);
        assert!(!game.runtime_level_is_open("phase:a"));
        assert!(seen.loads.lock().unwrap().is_empty());
        game.tick_server(false).unwrap();
        assert!(game.runtime_level_is_open("phase:a"));
        assert_eq!(*seen.loads.lock().unwrap(), ["phase:a"]);
        assert_eq!(
            *seen.load_contexts.lock().unwrap(),
            [("phase:a".into(), "minecraft:overworld".into(), false)]
        );
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_handle_from_before_an_unload_never_names_the_reopened_level() {
    with_opener("stale", "stale:a", flat(), |game, seen| {
        tick_post(game);
        game.apply_runtime_levels();
        let first = seen.answers.lock().unwrap()[0].unwrap();
        assert!(game.level_handle_current(first));
        assert!(game.native_level_world(first).is_some());
        unload_as(game, "stale", "stale:a").unwrap();
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("stale:a"));
        assert!(game.native_level_world(first).is_none());
        // The same id opens again: a new open, a new handle.
        seen.answers.lock().unwrap().clear();
        tick_post(game);
        game.apply_runtime_levels();
        let second = seen.answers.lock().unwrap()[0].unwrap();
        assert_ne!(first, second);
        assert!(game.native_level_world(second).is_some());
        assert!(!game.level_handle_current(first));
        assert!(game.native_level_world(first).is_none());
        assert_eq!(*seen.loads.lock().unwrap(), ["stale:a", "stale:a"]);
        assert_eq!(*seen.unloads.lock().unwrap(), ["stale:a"]);
    });
}

/// The idle close drops a runtime level from memory as any level, but it stays open: entering
/// it opens it again with the same handle, and only `unload` closes it for good.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn the_idle_close_keeps_a_runtime_level_open_until_its_unload() {
    with_opener("idle", "idle:a", flat(), |game, seen| {
        tick_post(game);
        game.apply_runtime_levels();
        let handle = seen.answers.lock().unwrap()[0].unwrap();
        let dim = super::Dim::extra("idle:a").unwrap();
        // What the idle close does to a level nobody is in.
        game.close_parked_level(dim).unwrap();
        assert_eq!(*seen.unloads.lock().unwrap(), ["idle:a"]);
        assert!(game.runtime_level_is_open("idle:a"));
        assert!(game.level_handle_current(handle));
        game.enter_level(dim).unwrap();
        assert_eq!(*seen.loads.lock().unwrap(), ["idle:a", "idle:a"]);
        game.enter_level(super::Dim::Overworld).unwrap();
        assert_eq!(unload_as(game, "idle", "idle:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("idle:a"));
        assert!(super::Dim::extra("idle:a").is_none());
        assert_eq!(*seen.unloads.lock().unwrap(), ["idle:a", "idle:a"]);
    });
}

/// An unload whose save fails leaves the level open and in place: the mod can unload it again,
/// and that unload goes through once the save can be written.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_failed_unload_leaves_the_level_open_for_a_retry() {
    with_opener("stuck", "stuck:a", flat(), |game, _| {
        tick_post(game);
        game.apply_runtime_levels();
        let dim = super::Dim::extra("stuck:a").unwrap();
        // A broken carrier transfer refuses every save of the level (`admit_level_save`).
        game.enter_level(dim).unwrap();
        game.world.carrier_fences.mark_broken();
        assert_eq!(unload_as(game, "stuck", "stuck:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(game.runtime_level_is_open("stuck:a"));
        assert!(super::Dim::extra("stuck:a").is_some());
        assert_eq!(game.dim, dim);
        // The save can be written again: the same unload, asked again, goes through.
        game.world.carrier_fences = Default::default();
        assert_eq!(unload_as(game, "stuck", "stuck:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("stuck:a"));
        assert!(super::Dim::extra("stuck:a").is_none());
    });
}

/// A save a mod asked for, then an unload in the same tick: the save leaves with the level, so
/// the next tick does not open it again.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn an_unload_drops_a_save_waiting_for_its_level() {
    with_opener("saved", "saved:a", flat(), |game, seen| {
        tick_post(game);
        game.apply_runtime_levels();
        let dim = super::Dim::extra("saved:a").unwrap();
        game.levels.request_save(dim);
        assert_eq!(unload_as(game, "saved", "saved:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!game.levels.save_pending(dim));
        game.run_requested_saves();
        assert!(!game.runtime_level_is_open("saved:a"));
        assert!(!game.levels.parked.contains_key(&dim));
        assert_eq!(*seen.loads.lock().unwrap(), ["saved:a"]);
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_level_with_a_player_in_it_is_not_unloaded() {
    with_opener("crowd", "crowd:a", flat(), |game, seen| {
        tick_post(game);
        game.apply_runtime_levels();
        let dim = super::Dim::extra("crowd:a").unwrap();
        game.enter_level(dim).unwrap();
        // The player stands in the level in place.
        game.player = Some(
            horizon_client::motion::PlayerMotion::new(
                game.motion,
                horizon_core::geometry::Vec3::new(0.5, 80.0, 0.5),
            )
            .unwrap(),
        );
        assert_eq!(
            unload_as(game, "crowd", "crowd:a"),
            Err(RuntimeLevelError::PlayersInside)
        );
        game.enter_level(super::Dim::Overworld).unwrap();
        assert_eq!(unload_as(game, "crowd", "crowd:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("crowd:a"));
        assert_eq!(*seen.unloads.lock().unwrap(), ["crowd:a"]);
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_reopened_level_comes_back_from_its_folder_with_its_block() {
    with_opener("keep", "keep:a", flat(), |game, seen| {
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        tick_post(game);
        game.apply_runtime_levels();
        let dim = super::Dim::extra("keep:a").unwrap();
        game.enter_level(dim).unwrap();
        game.streaming.forced.insert((0, 0));
        for _ in 0..2000 {
            game.update_world();
            if game.world.chunks.contains_key(&(0, 0)) {
                break;
            }
            game.streaming.wait(std::time::Duration::from_millis(5));
        }
        assert!(
            game.world.chunks.contains_key(&(0, 0)),
            "the chunk generated"
        );
        game.world.set(1, -50, 1, stone).unwrap();
        game.enter_level(super::Dim::Overworld).unwrap();
        unload_as(game, "keep", "keep:a").unwrap();
        game.apply_runtime_levels();
        assert!(
            game.world_root.join("dimensions/keep/a").is_dir(),
            "the folder stays"
        );

        seen.answers.lock().unwrap().clear();
        tick_post(game);
        game.apply_runtime_levels();
        let dim = super::Dim::extra("keep:a").unwrap();
        game.enter_level(dim).unwrap();
        game.streaming.forced.insert((0, 0));
        for _ in 0..2000 {
            game.update_world();
            if game.world.chunks.contains_key(&(0, 0)) {
                break;
            }
            game.streaming.wait(std::time::Duration::from_millis(5));
        }
        assert_eq!(game.world.block(1, -50, 1), stone);
        game.enter_level(super::Dim::Overworld).unwrap();
        unload_as(game, "keep", "keep:a").unwrap();
        game.apply_runtime_levels();
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn failed_construction_rolls_back_every_published_table() {
    with_opener("rollback", "rollback:a", flat(), |game, seen| {
        tick_post(game);
        let first = seen.answers.lock().unwrap()[0].unwrap();
        game.levels.runtime.fail_open_after_publication = true;
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("rollback:a"));
        assert!(!game.levels.runtime.holds("rollback:a"));
        assert!(super::Dim::extra("rollback:a").is_none());
        assert!(
            !game
                .level_shapes
                .extra
                .iter()
                .any(|level| level.id == "rollback:a")
        );
        assert!(seen.loads.lock().unwrap().is_empty());
        assert!(!game.level_handle_current(first));
        game.levels.runtime.fail_open_after_publication = false;
        seen.answers.lock().unwrap().clear();
        tick_post(game);
        game.apply_runtime_levels();
        assert!(game.runtime_level_is_open("rollback:a"));
        assert_ne!(seen.answers.lock().unwrap()[0].unwrap(), first);
        unload_as(game, "rollback", "rollback:a").unwrap();
        game.apply_runtime_levels();
    });
}

/// Deleting an open, empty level closes it, then removes its folder and the structure references
/// kept beside the world, and the folders they leave empty.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_delete_closes_the_level_and_removes_its_folder_and_references_file() {
    with_opener("erase", "erase:instance/ab", flat(), |game, seen| {
        tick_post(game);
        game.apply_runtime_levels();
        place_block(game, "erase:instance/ab", "minecraft:stone");
        let folder = game.world_root.join("dimensions/erase/instance/ab");
        let references = game
            .world_root
            .join("data/horizon_structure_references_erase_instance/ab.json");
        write_file(&references);
        // The save of the unload writes the folder; a stray file is in it too.
        write_file(&folder.join("data/minecraft/raids.dat"));
        assert_eq!(delete_as(game, "erase", "erase:instance/ab"), Ok(()));
        assert!(
            game.runtime_level_is_open("erase:instance/ab"),
            "open until the delete is applied"
        );
        assert!(folder.is_dir() && references.is_file());
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("erase:instance/ab"));
        assert!(!folder.exists(), "the folder is gone");
        assert!(!references.exists(), "the references file is gone");
        assert!(
            !game
                .world_root
                .join("data/horizon_structure_references_erase_instance")
                .exists(),
            "so is the folder it left empty"
        );
        assert!(!game.world_root.join("dimensions/erase").exists());
        assert!(game.world_root.join("dimensions").is_dir());
        assert!(game.world_root.join("data").is_dir());
        assert_eq!(*seen.unloads.lock().unwrap(), ["erase:instance/ab"]);
    });
}

/// A level that is not open, in the mod's namespace, is only files: what a crash left.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_delete_of_a_level_that_is_not_open_removes_its_leftovers() {
    with_opener("left", "left:room", flat(), |game, seen| {
        let folder = game.world_root.join("dimensions/left/room");
        let sibling = game.world_root.join("dimensions/left/other");
        let references = game
            .world_root
            .join("data/horizon_structure_references_left_room.json");
        write_file(&folder.join("region/r.0.0.mca"));
        write_file(&sibling.join("region/r.0.0.mca"));
        write_file(&references);
        assert!(!game.runtime_level_is_open("left:room"));
        assert_eq!(delete_as(game, "left", "left:room"), Ok(()));
        assert!(
            folder.is_dir(),
            "nothing is removed before the queue is applied"
        );
        game.apply_runtime_levels();
        assert!(!folder.exists() && !references.exists());
        assert!(
            sibling.join("region/r.0.0.mca").is_file(),
            "another level of the namespace stays"
        );
        // Nothing left to remove is nothing to refuse.
        assert_eq!(delete_as(game, "left", "left:room"), Ok(()));
        game.apply_runtime_levels();
        assert!(seen.unloads.lock().unwrap().is_empty(), "it was never open");
    });
}

/// The folder of another mod's level, of a vanilla level and of a path that leaves the world is
/// never touched.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_delete_outside_the_mods_namespace_is_refused_and_removes_nothing() {
    with_opener("mine", "mine:a", flat(), |game, _| {
        let other = game.world_root.join("dimensions/other/x/region/r.0.0.mca");
        let vanilla = game
            .world_root
            .join("dimensions/minecraft/overworld/region/r.0.0.mca");
        let escape = game.world_root.join("dimensions/mine/x");
        write_file(&other);
        write_file(&vanilla);
        write_file(&escape.join("keep.txt"));
        for (id, expected) in [
            ("other:x", RuntimeLevelError::OutsideNamespace),
            ("minecraft:overworld", RuntimeLevelError::OutsideNamespace),
            ("mine:../other/x", RuntimeLevelError::InvalidId),
            ("mine:x/../../other/x", RuntimeLevelError::InvalidId),
            ("mine:con", RuntimeLevelError::InvalidId),
        ] {
            assert_eq!(delete_as(game, "mine", id), Err(expected), "{id}");
        }
        game.apply_runtime_levels();
        assert!(other.is_file() && vanilla.is_file() && escape.join("keep.txt").is_file());
    });
}

/// A level deleted and opened again starts empty: the block placed before is gone.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_level_opened_again_after_a_delete_starts_empty() {
    with_opener("fresh", "fresh:a", flat(), |game, seen| {
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        tick_post(game);
        game.apply_runtime_levels();
        let first = seen.answers.lock().unwrap()[0].unwrap();
        place_block(game, "fresh:a", "minecraft:stone");
        assert_eq!(block_at(game, "fresh:a"), stone);
        assert_eq!(delete_as(game, "fresh", "fresh:a"), Ok(()));
        // The open waits for the delete to be applied.
        seen.answers.lock().unwrap().clear();
        tick_post(game);
        assert_eq!(
            seen.answers.lock().unwrap()[0],
            Err(RuntimeLevelError::AlreadyOpen)
        );
        game.apply_runtime_levels();
        assert!(!game.world_root.join("dimensions/fresh/a").exists());
        seen.answers.lock().unwrap().clear();
        tick_post(game);
        game.apply_runtime_levels();
        let second = seen.answers.lock().unwrap()[0].unwrap();
        assert_ne!(first, second);
        assert!(!game.level_handle_current(first));
        let air = game
            .pack
            .assets
            .registry
            .default_state("minecraft:air")
            .unwrap();
        assert_eq!(block_at(game, "fresh:a"), air, "the stone is not there");
        delete_as(game, "fresh", "fresh:a").unwrap();
        game.apply_runtime_levels();
    });
}

/// An open made after the delete of a level that is not open is applied after it: the leftovers
/// are gone before the level opens.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn an_open_queued_behind_a_delete_of_leftovers_finds_no_folder() {
    with_opener("behind", "behind:a", flat(), |game, seen| {
        let leftover = game.world_root.join("dimensions/behind/a/region/r.0.0.mca");
        write_file(&leftover);
        assert_eq!(delete_as(game, "behind", "behind:a"), Ok(()));
        tick_post(game);
        assert!(seen.answers.lock().unwrap()[0].is_ok());
        game.apply_runtime_levels();
        assert!(game.runtime_level_is_open("behind:a"));
        assert!(!leftover.exists());
        delete_as(game, "behind", "behind:a").unwrap();
        game.apply_runtime_levels();
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_level_with_a_player_in_it_is_not_deleted() {
    with_opener("guest", "guest:a", flat(), |game, _| {
        tick_post(game);
        game.apply_runtime_levels();
        let dim = super::Dim::extra("guest:a").unwrap();
        game.enter_level(dim).unwrap();
        game.player = Some(
            horizon_client::motion::PlayerMotion::new(
                game.motion,
                horizon_core::geometry::Vec3::new(0.5, 80.0, 0.5),
            )
            .unwrap(),
        );
        let marker = game.world_root.join("dimensions/guest/a/marker.txt");
        write_file(&marker);
        assert_eq!(
            delete_as(game, "guest", "guest:a"),
            Err(RuntimeLevelError::PlayersInside)
        );
        game.apply_runtime_levels();
        assert!(game.runtime_level_is_open("guest:a"));
        assert!(marker.is_file());
        game.enter_level(super::Dim::Overworld).unwrap();
        assert_eq!(delete_as(game, "guest", "guest:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!marker.exists());
        assert!(!game.runtime_level_is_open("guest:a"));
    });
}

/// A delete asked inside a level's tick leaves the level and its files as they are until the
/// queue is applied after the levels.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_delete_is_applied_after_the_levels_and_not_inside_a_tick() {
    with_opener("later", "later:a", flat(), |game, seen| {
        tick_post(game);
        game.apply_runtime_levels();
        let marker = game.world_root.join("dimensions/later/a/marker.txt");
        write_file(&marker);
        seen.open_right_after.lock().unwrap().clear();
        seen.mode.store(DELETE, Ordering::SeqCst);
        tick_post(game);
        assert_eq!(*seen.deletes.lock().unwrap(), [Ok(())]);
        assert_eq!(*seen.open_right_after.lock().unwrap(), [true]);
        assert!(marker.is_file(), "nothing is removed inside the tick");
        assert!(seen.unloads.lock().unwrap().is_empty());
        seen.mode.store(IDLE, Ordering::SeqCst);
        game.tick_server(false).unwrap();
        assert!(!marker.exists());
        assert!(!game.runtime_level_is_open("later:a"));
        assert_eq!(*seen.unloads.lock().unwrap(), ["later:a"]);
    });
}

/// A delete of a level whose open is still waiting is refused: its open comes first.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_delete_of_a_level_still_waiting_to_open_is_refused() {
    with_opener("wait", "wait:a", flat(), |game, _| {
        tick_post(game);
        assert_eq!(
            delete_as(game, "wait", "wait:a"),
            Err(RuntimeLevelError::AlreadyOpen)
        );
        game.apply_runtime_levels();
        assert!(game.runtime_level_is_open("wait:a"));
        delete_as(game, "wait", "wait:a").unwrap();
        game.apply_runtime_levels();
    });
}

#[test]
fn a_delete_marks_an_open_level_closing_and_a_second_unload_finds_it_not_open() {
    let mut table = RuntimeTable::default();
    table.queue_open("test", "del:a", flat()).unwrap();
    assert_eq!(
        table.queue_delete("test", "del:a"),
        Err(RuntimeLevelError::AlreadyOpen)
    );
    table.mark_open("del:a");
    assert_eq!(table.queue_delete("test", "del:a"), Ok(()));
    assert!(table.is_open("del:a"));
    assert_eq!(table.queue_unload("del:a"), Err(RuntimeLevelError::NotOpen));
    assert_eq!(
        table.queue_open("test", "del:a", flat()),
        Err(RuntimeLevelError::AlreadyOpen),
        "no reopen until the delete is applied"
    );
    // A level the table does not hold has only files to delete.
    assert_eq!(table.queue_delete("test", "del:none"), Ok(()));
    assert!(!table.holds("del:none"));
}

/// The folders of levels nest: a delete of a level with a level below it, open or only a folder,
/// is refused and removes nothing of either.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_delete_of_a_level_with_a_level_below_it_is_refused() {
    with_opener("kid", "kid:instance/ab", flat(), |game, _| {
        let parent_file = game
            .world_root
            .join("dimensions/kid/instance/region/r.0.0.mca");
        write_file(&parent_file);
        tick_post(game);
        // Waiting to open, then open: the table knows the level below.
        for _ in 0..2 {
            assert_eq!(
                delete_as(game, "kid", "kid:instance"),
                Err(RuntimeLevelError::LevelsBelow)
            );
            game.apply_runtime_levels();
        }
        assert!(game.runtime_level_is_open("kid:instance/ab"));
        place_block(game, "kid:instance/ab", "minecraft:stone");
        assert!(parent_file.is_file());
        // The child goes: the parent's own files stay, and the parent can go next.
        assert_eq!(delete_as(game, "kid", "kid:instance/ab"), Ok(()));
        game.apply_runtime_levels();
        assert!(!game.world_root.join("dimensions/kid/instance/ab").exists());
        assert!(
            parent_file.is_file(),
            "a level's own files are not the parent's"
        );
        assert_eq!(delete_as(game, "kid", "kid:instance"), Ok(()));
        game.apply_runtime_levels();
        assert!(!game.world_root.join("dimensions/kid/instance").exists());
    });
}

/// A level below that only exists as a folder, as after a crash, stops the delete too, even when
/// its folder has the name of one a level writes.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_delete_stops_at_a_folder_that_holds_a_level() {
    with_opener("fold", "fold:unused", flat(), |game, _| {
        let child = game
            .world_root
            .join("dimensions/fold/p/ab/region/r.0.0.mca");
        // `fold:q/region` is a level too, and its folder is where `fold:q` keeps its regions.
        let named_region = game
            .world_root
            .join("dimensions/fold/q/region/data/minecraft/raids.dat");
        let own = game.world_root.join("dimensions/fold/p/region/r.0.0.mca");
        write_file(&child);
        write_file(&own);
        write_file(&named_region);
        // `fold:r/data/ab` is a level that only exists as a folder under `data`.
        let in_data = game
            .world_root
            .join("dimensions/fold/r/data/ab/region/r.0.0.mca");
        let own_data = game
            .world_root
            .join("dimensions/fold/r/data/minecraft/raids.dat");
        write_file(&in_data);
        write_file(&own_data);
        for id in ["fold:p", "fold:q", "fold:r"] {
            assert_eq!(
                delete_as(game, "fold", id),
                Err(RuntimeLevelError::LevelsBelow),
                "{id}"
            );
        }
        game.apply_runtime_levels();
        assert!(child.is_file() && own.is_file() && named_region.is_file() && in_data.is_file());
        // Only the level itself: `fold:p/ab` has no folder below it.
        assert_eq!(delete_as(game, "fold", "fold:p/ab"), Ok(()));
        game.apply_runtime_levels();
        assert!(!child.exists() && own.is_file());
        assert_eq!(delete_as(game, "fold", "fold:p"), Ok(()));
        game.apply_runtime_levels();
        assert!(!own.exists());
        // What `data` holds besides a level goes with its level.
        assert_eq!(delete_as(game, "fold", "fold:r/data/ab"), Ok(()));
        game.apply_runtime_levels();
        assert_eq!(delete_as(game, "fold", "fold:r"), Ok(()));
        game.apply_runtime_levels();
        assert!(!own_data.exists() && !in_data.exists());
    });
}

/// Two ids can name one structure references file (`a_b:c`, `a:b_c`): a delete of one keeps the
/// file while the other exists, and removes it once the other is gone.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_delete_keeps_a_references_file_another_level_shares() {
    with_opener("a_b", "a_b:c", flat(), |game, _| {
        let mine = game.world_root.join("dimensions/a_b/c/region/r.0.0.mca");
        let other = game.world_root.join("dimensions/a/b_c/region/r.0.0.mca");
        let shared = game
            .world_root
            .join("data/horizon_structure_references_a_b_c.json");
        write_file(&mine);
        write_file(&other);
        write_file(&shared);
        assert_eq!(delete_as(game, "a_b", "a_b:c"), Ok(()));
        game.apply_runtime_levels();
        assert!(!mine.exists());
        assert!(
            other.is_file(),
            "the other namespace's level is not touched"
        );
        assert!(shared.is_file(), "its references file is kept");
        std::fs::remove_dir_all(game.world_root.join("dimensions/a")).unwrap();
        assert_eq!(delete_as(game, "a_b", "a_b:c"), Ok(()));
        game.apply_runtime_levels();
        assert!(!shared.exists(), "nothing else shares it now");
    });
}

/// A removal that fails leaves the level closed, its id kept from `open` until a `delete` has
/// removed the files.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_failed_removal_leaves_the_level_closed_and_its_id_closed_to_open() {
    with_opener("stuck2", "stuck2:a", flat(), |game, seen| {
        tick_post(game);
        game.apply_runtime_levels();
        let first = seen.answers.lock().unwrap()[0].unwrap();
        let marker = game.world_root.join("dimensions/stuck2/a/data/marker.dat");
        write_file(&marker);
        game.levels.runtime.fail_erase = true;
        assert_eq!(delete_as(game, "stuck2", "stuck2:a"), Ok(()));
        game.apply_runtime_levels();
        // Closed: not open, out of the world's levels, its handle stale, `level_unload` heard.
        assert!(!game.runtime_level_is_open("stuck2:a"));
        assert!(super::Dim::extra("stuck2:a").is_none());
        assert!(!game.level_handle_current(first));
        assert_eq!(*seen.unloads.lock().unwrap(), ["stuck2:a"]);
        assert!(marker.is_file(), "the files are left");
        // Its id cannot be opened while they are.
        seen.answers.lock().unwrap().clear();
        tick_post(game);
        assert_eq!(
            seen.answers.lock().unwrap()[0],
            Err(RuntimeLevelError::Removing)
        );
        assert_eq!(delete_as(game, "stuck2", "stuck2:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(marker.is_file(), "the removal failed again");
        // Once it can, a second delete finishes it and the id opens again, empty.
        game.levels.runtime.fail_erase = false;
        assert_eq!(delete_as(game, "stuck2", "stuck2:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!marker.exists());
        seen.answers.lock().unwrap().clear();
        tick_post(game);
        game.apply_runtime_levels();
        assert!(seen.answers.lock().unwrap()[0].is_ok());
        assert!(game.runtime_level_is_open("stuck2:a"));
        assert!(!marker.exists());
    });
}

/// An `open` made after the `delete` of leftovers is applied after it; if the removal failed it
/// is dropped, so the level never starts from the files that are left.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn an_open_queued_behind_a_failed_removal_is_dropped() {
    with_opener("behind2", "behind2:a", flat(), |game, seen| {
        let leftover = game
            .world_root
            .join("dimensions/behind2/a/region/r.0.0.mca");
        write_file(&leftover);
        game.levels.runtime.fail_erase = true;
        assert_eq!(delete_as(game, "behind2", "behind2:a"), Ok(()));
        tick_post(game);
        assert!(seen.answers.lock().unwrap()[0].is_ok());
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("behind2:a"));
        assert!(!game.levels.runtime.holds("behind2:a"));
        assert!(super::Dim::extra("behind2:a").is_none());
        assert!(leftover.is_file());
        assert!(seen.loads.lock().unwrap().is_empty());
    });
}

/// A symbolic link, or on Windows a junction, from `at` to the folder `to`. Panics if the machine
/// cannot make one: these tests are about links, and a skip would look like a pass.
fn make_link(to: &std::path::Path, at: &std::path::Path) {
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    // A symbolic link needs a privilege on Windows; a junction does not.
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_dir(to, at).or_else(|_| {
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(at.to_string_lossy().replace('/', "\\"))
            .arg(to.to_string_lossy().replace('/', "\\"))
            .output()
            .map(|_| ())
            .and_then(|()| std::fs::symlink_metadata(at).map(|_| ()))
    });
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(to, at);
    made.expect("this machine cannot create a symbolic link or a junction");
}

/// Remove the link `link` made by `make_link`, not what it leads to: a folder link on Windows, a
/// file-like symbolic link on Unix.
fn remove_link(link: &std::path::Path) {
    #[cfg(windows)]
    std::fs::remove_dir(link).unwrap();
    #[cfg(not(windows))]
    std::fs::remove_file(link).unwrap();
}

/// A link under `dimensions` or `data`, or one of the two being a link, is never followed out of
/// the world, and nothing is removed first.
#[test]
#[ignore = "needs permission to create a symbolic link or junction"]
fn a_removal_does_not_follow_links_out_of_the_world() {
    let root = TempRoot::new("links");
    let outside = root.0.join("outside");
    let victim = outside.join("region/r.0.0.mca");
    write_file(&victim);
    let erase = |world: &std::path::Path, id: &str| {
        super::runtime_levels::erase_level_files(world, id, true)
    };
    // The namespace folder is a link.
    let world = root.0.join("world");
    make_link(&outside, &world.join("dimensions/lnk"));
    assert!(erase(&world, "lnk:x").is_err());
    // The level's own folder is a link.
    remove_link(&world.join("dimensions/lnk"));
    make_link(&outside, &world.join("dimensions/ns/x"));
    assert!(erase(&world, "ns:x").is_err());
    // The folder of the references file is a link; nothing of the level's own is removed first.
    let own = world.join("dimensions/ns/y/region/r.0.0.mca");
    write_file(&own);
    write_file(&outside.join("ab.json"));
    make_link(
        &outside,
        &world.join("data/horizon_structure_references_ns_y"),
    );
    assert!(erase(&world, "ns:y/ab").is_err());
    assert!(victim.is_file() && outside.join("ab.json").is_file() && own.is_file());
    // `dimensions` is a link.
    let by_dimensions = root.0.join("by_dimensions");
    write_file(&outside.join("ns/x/region/r.0.0.mca"));
    make_link(&outside, &by_dimensions.join("dimensions"));
    assert!(erase(&by_dimensions, "ns:x").is_err());
    assert!(outside.join("ns/x/region/r.0.0.mca").is_file());
    // `data` is a link, with the references file behind it.
    let by_data = root.0.join("by_data");
    let reference = outside.join("horizon_structure_references_ns_z.json");
    write_file(&reference);
    make_link(&outside, &by_data.join("data"));
    assert!(erase(&by_data, "ns:z").is_err());
    assert!(reference.is_file());
}

/// Something that makes the removal of a file in a folder fail, until it is dropped: on Unix the
/// folder loses its write permission, on Windows the file is held open without sharing.
struct Lock {
    #[cfg(windows)]
    _file: std::fs::File,
    #[cfg(unix)]
    folder: std::path::PathBuf,
}

impl Lock {
    /// None when the machine does not enforce it (a process that is root ignores permissions).
    fn on(file: &std::path::Path) -> Option<Self> {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let held = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(file)
                .unwrap();
            Some(Self { _file: held })
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let folder = file.parent().unwrap().to_owned();
            std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
            let lock = Self { folder };
            let probe = lock.folder.join("probe");
            if std::fs::write(&probe, b"x").is_ok() {
                let _ = std::fs::remove_file(&probe);
                return None;
            }
            Some(lock)
        }
    }
}

#[cfg(unix)]
impl Drop for Lock {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.folder, std::fs::Permissions::from_mode(0o755));
    }
}

/// A removal that really fails part way, a file that cannot be removed: the level is closed and
/// its id is refused to `open`, the file is still there, and a `delete` asked again, once the
/// file can go, finishes the work.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_removal_that_fails_part_way_is_finished_by_a_second_delete() {
    with_opener("part", "part:a", flat(), |game, seen| {
        tick_post(game);
        game.apply_runtime_levels();
        let folder = game.world_root.join("dimensions/part/a");
        let locked = folder.join("data/locked/x.dat");
        write_file(&locked);
        write_file(&folder.join("poi/r.0.0.mca"));
        let Some(lock) = Lock::on(&locked) else {
            panic!(
                "partial-removal coverage requires enforced file permissions; run as non-root on Unix or run on Windows"
            );
        };
        assert_eq!(delete_as(game, "part", "part:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("part:a"));
        assert!(super::Dim::extra("part:a").is_none());
        assert_eq!(*seen.unloads.lock().unwrap(), ["part:a"]);
        assert!(
            locked.is_file(),
            "the file that could not go is still there"
        );
        seen.answers.lock().unwrap().clear();
        tick_post(game);
        assert_eq!(
            seen.answers.lock().unwrap()[0],
            Err(RuntimeLevelError::Removing)
        );
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("part:a"));
        // The file can go: the same delete, asked again, finishes.
        drop(lock);
        assert_eq!(delete_as(game, "part", "part:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!folder.exists());
        seen.answers.lock().unwrap().clear();
        tick_post(game);
        game.apply_runtime_levels();
        assert!(seen.answers.lock().unwrap()[0].is_ok());
        assert!(game.runtime_level_is_open("part:a"));
        assert!(!locked.exists(), "the level starts empty");
    });
}

/// Holds a scratch directory unreadable, restoring its original mode even on assertion failure.
#[cfg(unix)]
struct UnreadableFolder {
    folder: std::path::PathBuf,
    permissions: std::fs::Permissions,
}

#[cfg(unix)]
impl UnreadableFolder {
    fn on(folder: &std::path::Path) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let guard = Self {
            folder: folder.to_owned(),
            permissions: std::fs::metadata(folder).unwrap().permissions(),
        };
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o000)).unwrap();
        assert_eq!(
            std::fs::read_dir(folder).err().map(|error| error.kind()),
            Some(std::io::ErrorKind::PermissionDenied),
            "unreadable-folder coverage requires enforced permissions; run as non-root on Unix"
        );
        guard
    }
}

#[cfg(unix)]
impl Drop for UnreadableFolder {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.folder, self.permissions.clone());
    }
}

#[cfg(unix)]
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_delete_refuses_unreadable_data_and_removes_nothing() {
    with_opener("blind", "blind:a", flat(), |game, _| {
        let folder = game.world_root.join("dimensions/blind/a");
        let marker = folder.join("data/private/marker.dat");
        let region = folder.join("region/r.0.0.mca");
        let references = game
            .world_root
            .join("data/horizon_structure_references_blind_a.json");
        write_file(&marker);
        write_file(&region);
        write_file(&references);
        let guard = UnreadableFolder::on(marker.parent().unwrap());
        assert_eq!(
            delete_as(game, "blind", "blind:a"),
            Err(RuntimeLevelError::LevelsBelow)
        );
        game.apply_runtime_levels();
        drop(guard);
        assert!(marker.is_file() && region.is_file() && references.is_file());
        assert_eq!(delete_as(game, "blind", "blind:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!folder.exists() && !references.exists());
    });
}

#[cfg(unix)]
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_delete_keeps_references_when_shared_level_metadata_is_unreadable() {
    with_opener("a_b", "a_b:c", flat(), |game, _| {
        let folder = game.world_root.join("dimensions/a_b/c");
        let references = game
            .world_root
            .join("data/horizon_structure_references_a_b_c.json");
        let other_namespace = game.world_root.join("dimensions/a");
        write_file(&folder.join("region/r.0.0.mca"));
        write_file(&references);
        std::fs::create_dir_all(&other_namespace).unwrap();
        let guard = UnreadableFolder::on(&other_namespace);
        assert_eq!(
            std::fs::symlink_metadata(other_namespace.join("b_c"))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
        assert_eq!(delete_as(game, "a_b", "a_b:c"), Ok(()));
        game.apply_runtime_levels();
        assert!(!folder.exists(), "the requested level folder was removed");
        assert!(
            references.is_file(),
            "unknown sharing must preserve references"
        );
        drop(guard);
        assert!(!other_namespace.join("b_c").exists());
        assert_eq!(delete_as(game, "a_b", "a_b:c"), Ok(()));
        game.apply_runtime_levels();
        assert!(!folder.exists() && !references.exists());
    });
}

/// Single player: the integrated server and the window's client share the process's table of
/// levels. A runtime level the server opened, which the client entered and left, stays the
/// server's: it still unloads, and, opened again, entered and left again, still deletes.
#[test]
#[ignore = "daily"]
fn a_level_the_player_left_in_single_player_still_unloads_and_deletes() {
    use horizon_protocol::packets::play::{self, Clientbound as C};
    let travel = |dimension: &str| {
        C::Respawn(play::Respawn {
            spawn: super::viewed_worlds::tests::spawn(dimension),
            keep: 3,
        })
    };
    with_opener("solo", "solo:a", flat(), |game, _| {
        tick_post(game);
        game.apply_runtime_levels();
        assert!(game.runtime_level_is_open("solo:a"));
        let mut resources = None;
        let mut window = super::viewed_worlds::tests::client(&mut resources);
        window.handle_packet(travel("solo:a")).unwrap();
        window.handle_packet(travel("minecraft:overworld")).unwrap();
        assert!(
            super::Dim::extra("solo:a").is_some(),
            "still the server's level"
        );
        assert_eq!(unload_as(game, "solo", "solo:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("solo:a"));
        assert!(
            super::Dim::extra("solo:a").is_none(),
            "closed by the server"
        );
        // Opened again by the mod, entered and left again, then deleted.
        tick_post(game);
        game.apply_runtime_levels();
        assert!(game.runtime_level_is_open("solo:a"));
        window.handle_packet(travel("solo:a")).unwrap();
        window.handle_packet(travel("minecraft:overworld")).unwrap();
        assert_eq!(delete_as(game, "solo", "solo:a"), Ok(()));
        game.apply_runtime_levels();
        assert!(!game.runtime_level_is_open("solo:a"));
        assert!(!game.world_root.join("dimensions/solo/a").exists());
    });
}
