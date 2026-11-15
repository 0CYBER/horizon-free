//! Real generated dispatch -> WasmMod proxy -> compiled generated guest -> host write-back.
//! The opt-in mixed catalog is not shipped. Its native conversion callback uses a fixed
//! 64-item fixture limit; real Game registry conversion is covered by item_snapshot tests.
#![cfg(feature = "mixed-output-fixture")]

#[path = "mixed-output-fixture/src/lib.rs"]
mod guest;
use mod_api::{
    Flow, Hooks, ItemStack, MixedOutputFixtureEvent, ModEntry, ServerHost, StackWriteError,
};
use std::sync::Mutex;

struct Quiet;
impl mod_api::Host for Quiet {}
mod_api::impl_host_areas!(Quiet);
impl ServerHost for Quiet {}
mod_api::impl_server_host_areas!(Quiet);

struct Before;
#[mod_api::__subscriptions]
impl Hooks for Before {
    #[listener(priority = 10)]
    fn mixed_output_fixture(
        &self,
        _: &mut dyn ServerHost,
        event: &mut MixedOutputFixtureEvent<'_>,
    ) -> Flow {
        if event.mode == 3 {
            let mut second = event.drops[0].clone();
            event.drops[0].set_count(3).unwrap();
            second.set_count(4).unwrap();
            event.drops.push(second);
            event.damage = 9;
        }
        Flow::Continue
    }
}
struct SkipsCancelled;
#[mod_api::__subscriptions]
impl Hooks for SkipsCancelled {
    #[listener(priority = -5)]
    fn mixed_output_fixture(
        &self,
        _: &mut dyn ServerHost,
        event: &mut MixedOutputFixtureEvent<'_>,
    ) -> Flow {
        assert!(
            !event.cancelled,
            "ordinary listener received a cancelled event"
        );
        assert!(
            !matches!(event.mode, 0 | 2 | 4),
            "cancellation was lost at the guest boundary: {:?}",
            WARNINGS.0.lock().unwrap()
        );
        Flow::Continue
    }
}
struct After;
#[mod_api::__subscriptions]
impl Hooks for After {
    #[listener(priority = -10, receive_cancelled)]
    fn mixed_output_fixture(
        &self,
        _: &mut dyn ServerHost,
        event: &mut MixedOutputFixtureEvent<'_>,
    ) -> Flow {
        if event.mode == 2 {
            assert_eq!(counts(event.drops), [2]);
            assert_eq!((event.damage, event.cancelled), (7, true));
            let mut extra = event.drops[0].clone();
            extra.set_count(5).unwrap();
            event.drops.push(extra);
            event.damage += 1;
            event.cancelled = false;
        }
        Flow::Continue
    }
}

struct Warnings(Mutex<Vec<String>>);
static WARNINGS: Warnings = Warnings(Mutex::new(Vec::new()));
impl log::Log for Warnings {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn
    }
    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            self.0.lock().unwrap().push(record.args().to_string());
        }
    }
    fn flush(&self) {}
}
fn counts(stacks: &[ItemStack]) -> Vec<u32> {
    stacks.iter().map(ItemStack::count).collect()
}

fn matrix(listener: ModEntry) {
    mod_api::testing::with_mods(
        vec![
            ModEntry::new("before", Box::new(Before)),
            listener,
            ModEntry::new("skips_cancelled", Box::new(SkipsCancelled)),
            ModEntry::new("after", Box::new(After)),
        ]
        .into(),
        || {
            mod_api::__private::resolve_items(["minecraft:diamond"]).unwrap();
            let key = mod_api::__private::item_key_by_identifier(
                &mod_api::Identifier::parse("minecraft:diamond").unwrap(),
            )
            .unwrap();
            mod_api::__private::mark_game_thread();
            let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            for (mode, expected, expected_damage, expected_cancel, conversions) in [
                (0, vec![2], 7, true, vec![vec![65]]),
                (1, vec![3, 4], 9, false, vec![vec![3, 4]]),
                (2, vec![2, 5], 8, false, vec![vec![65], vec![2, 5]]),
                (3, vec![3, 4], 11, false, vec![vec![3, 4], vec![66, 4]]),
                (4, vec![2], 13, true, vec![]),
            ] {
                WARNINGS.0.lock().unwrap().clear();
                let mut drops = vec![ItemStack::new(key, 2).unwrap()];
                let (mut damage, mut cancelled) = (0, false);
                let mut converted = Vec::new();
                mod_api::dispatch_mixed_output_fixture(
                    &mut Quiet,
                    mode,
                    &mut drops,
                    &mut damage,
                    &mut cancelled,
                    &mut |stacks| {
                        converted.push(counts(stacks));
                        if stacks.iter().any(|stack| stack.count() > 64) {
                            Err(StackWriteError::CountExceeded { maximum: 64 })
                        } else {
                            Ok(())
                        }
                    },
                );
                assert_eq!(counts(&drops), expected, "mode {mode}");
                assert_eq!(
                    (damage, cancelled),
                    (expected_damage, expected_cancel),
                    "mode {mode}"
                );
                assert_eq!(
                    converted, conversions,
                    "mode {mode}: validate only changed listener lists"
                );
                let warnings = WARNINGS.0.lock().unwrap();
                if matches!(mode, 1 | 4) {
                    assert!(warnings.is_empty(), "{warnings:?}");
                } else {
                    assert_eq!(warnings.len(), 1, "mode {mode}: {warnings:?}");
                    let warning = &warnings[0];
                    assert_eq!(warning.matches("mixed_output_fixture").count(), 2);
                    for required in ["drops", "CountExceeded", "64"] {
                        assert!(warning.contains(required), "mode {mode}: {warning}");
                    }
                }
            }
        },
    );
}

#[test]
#[ignore = "needs HORIZON_WASM_MIXED_OUTPUT_FIXTURE and HORIZON_WASM_STD built by wasm-mods"]
fn generated_mixed_outputs_cross_real_guest_boundary() {
    horizon_guest_stack::run_on_guest_stack(|| {
        log::set_logger(&WARNINGS).unwrap();
        log::set_max_level(log::LevelFilter::Warn);
        mod_api::__private::initialize().unwrap();
        matrix(ModEntry::new("mixed_output_fixture", Box::new(guest::Mod)));

        use horizon_mod_package::manifest::Package;
        let path = std::env::var("HORIZON_WASM_MIXED_OUTPUT_FIXTURE")
            .expect("build the mixed-output-fixture package with cargo horizon-mod build --wasm");
        let path = std::path::Path::new(&path);
        let manifest = Package::read(path).unwrap().manifest;
        let info = mod_api::PackageInfo::new(
            manifest.id.clone(),
            manifest.version.clone(),
            mod_api::PackageKind::Code,
            mod_api::Side::Both,
            vec![manifest.id.clone()],
            vec![],
        );
        let loaded = horizon_mod_wasm::load(
            &std::fs::read(
                std::env::var("HORIZON_WASM_STD").expect("build the shared wasm std member"),
            )
            .unwrap(),
            vec![horizon_mod_wasm::WasmPackage {
                manifest,
                module: Package::read_wasm(path).unwrap(),
            }],
        );
        assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
        assert_eq!(loaded.mods.len(), 1);
        loaded.initialize(&[info]).unwrap();
        matrix(loaded.mods.into_iter().next().unwrap());
    });
}
