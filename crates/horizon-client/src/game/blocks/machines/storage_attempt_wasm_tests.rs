//! Compiled guest storage -> real Game notification -> same active guest callback, then abort.
use super::super::Game;
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
use horizon_client::{
    chunk::Chunk,
    containers::{BlockEntity, ModBlockEntity, ModContainer},
    items::ItemStack,
};
use mod_api::{BroadcastMessageEvent, Flow, Hooks, ModEntry, PackageInfo, Text};
use std::sync::{Arc, Mutex};

const TEST: &str = "game::machines::native_items::storage_attempt_wasm_tests::compiled_guest_storage_notifies_reentrantly_before_actual_hopper_abort";
const OBSERVER: &str = "wasm_storage_attempt_observer";

struct Heard(Arc<Mutex<Vec<Text>>>);

#[mod_api::__subscriptions]
impl Hooks for Heard {
    fn broadcast_message(
        &self,
        _: &dyn mod_api::Host,
        event: &mut BroadcastMessageEvent<'_>,
    ) -> Flow {
        self.0.lock().unwrap().push(event.line.clone());
        Flow::Continue
    }
}

#[test]
#[ignore = "needs prepared HORIZON_TEST_RESOURCES and matching compiled HORIZON_WASM_FIXTURE/HORIZON_WASM_STD"]
fn compiled_guest_storage_notifies_reentrantly_before_actual_hopper_abort() {
    horizon_guest_stack::run_on_guest_stack(|| {
        horizon_mod_wasm::copy_probe::with_load_budget(run);
    });
}

fn run() {
    if run_fresh_cases(TEST, "HORIZON_WASM_STORAGE_CASE", &["pull", "push"]) {
        return;
    }
    use horizon_mod_package::manifest::Package;
    let path = std::path::PathBuf::from(
        std::env::var_os("HORIZON_WASM_FIXTURE").expect("build the matching compiled wasm-fixture"),
    );
    let manifest = Package::read(&path).unwrap().manifest;
    let info = PackageInfo::new(
        manifest.id.clone(),
        manifest.version.clone(),
        mod_api::PackageKind::Code,
        mod_api::Side::Both,
        vec![manifest.id.clone()],
        vec![],
    );
    let observer_info = PackageInfo::new(
        OBSERVER.into(),
        "1".into(),
        mod_api::PackageKind::Code,
        mod_api::Side::Both,
        vec![OBSERVER.into()],
        vec![],
    );
    let loaded = horizon_mod_wasm::load(
        &std::fs::read(
            std::env::var_os("HORIZON_WASM_STD").expect("build the matching shared WASM std"),
        )
        .unwrap(),
        vec![horizon_mod_wasm::WasmPackage {
            manifest,
            module: Package::read_wasm(&path).unwrap(),
        }],
    );
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    loaded.initialize(&[info, observer_info]).unwrap();
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mut mods = loaded.mods;
    mods.push(ModEntry::new(OBSERVER, Box::new(Heard(heard.clone()))));
    mod_platform::testing::with_mods(mods.into(), || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let pull = std::env::var("HORIZON_WASM_STORAGE_CASE").unwrap() == "pull";
            let storage = if pull { (8, 65, 8) } else { (8, 63, 8) };
            place(game, (8, 64, 8), "minecraft:hopper");
            place(game, storage, "minecraft:barrel");
            place(
                game,
                if pull { (8, 63, 8) } else { (8, 65, 8) },
                "minecraft:bedrock",
            );
            let mut items = vec![None; 5];
            if pull {
                items.fill(Some(ItemStack::new("minecraft:stone", 64)));
                items[0] = Some(ItemStack::new("minecraft:dirt", 63));
            } else {
                items[0] = Some(ItemStack::new("minecraft:diamond", 1));
            }
            let original = items.clone();
            game.world
                .block_entities
                .insert((8, 64, 8), BlockEntity::Chest(items));
            let mut holder =
                ModBlockEntity::new("wasm_fixture:storage_attempts".into(), Default::default());
            holder.items = vec![pull.then(|| ItemStack::new("minecraft:diamond", 1))];
            holder.data = vec![0];
            game.world
                .block_entities
                .insert(storage, BlockEntity::Mod(Box::new(holder)));
            let state = game.world.block(storage.0, storage.1, storage.2);
            game.world.block_entities.set_mod_containers(
                [(
                    state.0,
                    Arc::new(ModContainer {
                        slots: 1,
                        inputs: vec![0],
                        outputs: vec![0],
                        faces: std::array::from_fn(|_| vec![0]),
                        menu: None,
                    }),
                )]
                .into_iter()
                .collect(),
            );
            let traces = Arc::new(Mutex::new(Vec::new()));
            game.transactions.trace = Some(traces.clone());
            let failures = game.wasm_dispatch_failures.get();
            let dropped = game.dropped.len();
            for tick in 1..=2 {
                game.world.game_time = tick;
                game.tick_hopper((8, 64, 8));
                let trace = std::mem::take(&mut *traces.lock().unwrap());
                assert_eq!(
                    trace,
                    if pull {
                        vec!["open", "abort", "restore:8,65,8:0"]
                    } else {
                        vec!["open", "abort", "restore:8,64,8:0"]
                    }
                );
                assert_eq!(
                    game.world.block_entities.get(&(8, 64, 8)).unwrap().slots(),
                    original.as_slice()
                );
                let Some(BlockEntity::Mod(holder)) = game.world.block_entities.get(&storage) else {
                    panic!("missing real storage")
                };
                assert_eq!(
                    holder.items,
                    [pull.then(|| ItemStack::new("minecraft:diamond", 1))]
                );
                assert_eq!(
                    holder.data,
                    [0],
                    "aborted guest writes restore native backing storage"
                );
                assert_eq!(game.hopper_cooldown((8, 64, 8)), 0);
                assert_eq!(game.dropped.len(), dropped);
                assert!(game.transactions.is_empty());
                assert_eq!(
                    game.wasm_dispatch_failures.get(),
                    failures,
                    "nested same-guest notification must not be refused/trapped"
                );
                let expected: Vec<_> = (1..=tick)
                    .map(|call| {
                        Text::literal(format!(
                            "wasm storage {} notice {call} value {} Open",
                            if pull { "pull" } else { "push" },
                            u32::from(!pull)
                        ))
                    })
                    .collect();
                assert_eq!(
                    *heard.lock().unwrap(),
                    expected,
                    "completed guest callbacks survive each actual root abort"
                );
            }
        })
    });
}

fn place(game: &mut Game<'_>, pos: (i32, i32, i32), block: &str) {
    let state = game.pack.assets.registry.default_state(block).unwrap();
    game.world.set(pos.0, pos.1, pos.2, state).unwrap();
}
