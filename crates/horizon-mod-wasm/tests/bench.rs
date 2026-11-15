//! Real shared member versus the same source natively. Run in release with the fixture and std
//! built first; this measures 1,000 dispatches into fake, in-memory block entities, not a world tick.
//!
//! From the root of the worktree containing this fixture and runtime, with rust-src
//! and wasm32-unknown-unknown installed:
//! ```sh
//! cargo build -p cargo-horizon-mod
//! (cd crates/horizon-mod-wasm/tests/fixture && "$OLDPWD/target/debug/cargo-horizon-mod" horizon-mod build --wasm --catalog points.json && "$OLDPWD/target/debug/cargo-horizon-mod" horizon-mod wasm-std "$OLDPWD/target/horizon-wasm-std.wasm" --catalog points.json)
//! HORIZON_WASM_STD="$PWD/target/horizon-wasm-std.wasm" HORIZON_WASM_FIXTURE="$PWD/crates/horizon-mod-wasm/tests/fixture/target/horizon-mod/wasm_fixture.hmod" cargo test --release -p horizon-mod-wasm --features listener-fixture --test bench empty_subscribed_listener -- --ignored --exact --nocapture
//! ```
//! Building the fixture rewrites its `horizon-mod.toml` with the generated wasm
//! hash and toolchain identity. Inspect and discard those generated changes after
//! measurement rather than committing them; preserve any intended manifest edits.
//! Use the same machine, build profile and conditions when comparing revisions.
//! The empty listener is `fixture::Mod::server_stopped` on both paths. Each sample
//! times 100,000 real dispatches; 20 warm-up samples precede 100 reported samples.
//! Loading, initialization, subscription installation and the host are outside timing.
//! These are in-memory dispatches, not server teardown or world ticks.
// Generated fixtures exercise every shape; TODO(arismoko/horizon#2273): make the generator emit clippy-clean code.
#![allow(clippy::all, unused_comparisons, unreachable_patterns, dead_code)]
#[path = "support/no_hooks.rs"]
mod no_hooks;
use horizon_mod_package::manifest::Package;
use horizon_mod_wasm::{WasmPackage, load};
use mod_api::{Host, ModBlockEntity, ModEntry, ServerHost};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

static WARNINGS_AND_ERRORS: AtomicUsize = AtomicUsize::new(0);
struct TestLog;
impl log::Log for TestLog {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        if record.level() <= log::Level::Warn {
            WARNINGS_AND_ERRORS.fetch_add(1, Ordering::Relaxed);
        }
        eprintln!("{}: {}", record.level(), record.args());
    }
    fn flush(&self) {}
}
fn init_log() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        log::set_logger(&TestLog).unwrap();
        log::set_max_level(log::LevelFilter::Warn);
    });
}

// Only the block entity parts are measured; the point listeners are tested elsewhere.
#[allow(unused)]
#[path = "fixture/src/lib.rs"]
mod fixture;

struct World;
impl Host for World {}
mod_api::impl_host_areas!(World, except(levels));
impl mod_api::prelude::LevelsHost for World {
    fn is_loaded(&self, id: &str) -> bool {
        id == "wasm_fixture"
    }
}
impl ServerHost for World {}
mod_api::impl_server_host_areas!(World);

fn measure(label: &str, mods: Vec<ModEntry>) {
    mod_api::testing::with_mods(mods.into(), || {
        mod_api::__private::install_parts(
            mod_api::__private::mods(),
            &mod_api::__private::part_tests::rule_schema(),
            |_| false,
        )
        .unwrap();
        mod_api::__private::mark_game_thread();
        let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
        for (kind, expected) in [("counter", 23), ("counter_reads", 25)] {
            assert!(
                mod_api::__private::block_entity_types()
                    .iter()
                    .any(|decl| decl.id == format!("wasm_fixture:{kind}")),
                "{label}: register did not declare wasm_fixture:counter"
            );
            let mut entities: Vec<_> = (0..1000)
                .map(|_| ModBlockEntity::new(format!("wasm_fixture:{kind}"), Default::default()))
                .collect();
            mod_api::dispatch_block_entity_tick(
                &mut World,
                &mut mod_api::BlockEntityTickReply::Changed,
                [0, 64, 0],
                &mut entities[0],
            );
            assert_eq!(
                entities[0].data,
                [expected],
                "{label}: registered tick did not write entity data"
            );
            assert_eq!(
                entities[0].nbt.get(&"counter".into()),
                Some(&mod_api::Tag::Int(17)),
                "{label}: registered tick did not write entity NBT"
            );
            let mut samples = Vec::with_capacity(100);
            for round in 0..120 {
                let now = Instant::now();
                for entity in &mut entities {
                    mod_api::dispatch_block_entity_tick(
                        &mut World,
                        &mut mod_api::BlockEntityTickReply::Changed,
                        [0, 64, 0],
                        entity,
                    );
                }
                let elapsed = now.elapsed().as_nanos();
                if round >= 20 {
                    samples.push(elapsed);
                }
            }
            assert!(entities.iter().all(|entity| entity.data == [expected]
                && entity.nbt.get(&"counter".into()) == Some(&mod_api::Tag::Int(17))));
            samples.sort_unstable();
            let mean = samples.iter().sum::<u128>() / samples.len() as u128;
            println!(
                "{label} {kind}: 1000 entities mean={mean} ns p95={} ns",
                samples[94]
            );
        }
    });
}

#[test]
#[ignore = "measurement; requires a built fixture and shared std"]
fn block_entity_ticks() {
    crate::test_stack::on_guest_stack(|| {
        init_log();
        let path = std::env::var("HORIZON_WASM_FIXTURE").unwrap();
        let path = std::path::Path::new(&path);
        let std = std::fs::read(std::env::var("HORIZON_WASM_STD").unwrap()).unwrap();
        let mut manifest = Package::read(path).unwrap().manifest;
        // This fixture variant exercises ordinary hooks and Parts, not game points.
        manifest.inject.clear();
        let info = mod_api::PackageInfo::new(
            manifest.id.clone(),
            manifest.version.clone(),
            mod_api::PackageKind::Code,
            mod_api::Side::Both,
            vec![manifest.id.clone()],
            Vec::new(),
        );
        let package = WasmPackage {
            manifest,
            module: Package::read_wasm(path).unwrap(),
        };
        let loaded = load(&std, vec![package]);
        assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
        assert_eq!(loaded.mods.len(), 1);
        loaded
            .initialize(&[info])
            .expect("wasm initialization failed");
        measure(
            "native",
            vec![ModEntry::new("wasm_fixture", Box::new(fixture::Mod))],
        );
        measure("wasm", loaded.mods);
    })
}

fn measure_empty_listener(label: &str, mods: Vec<ModEntry>) {
    const ITERATIONS: u128 = 100_000;
    mod_api::testing::with_mods(mods.into(), || {
        assert_eq!(mod_api::__private::mods().len(), 1);
        assert!(mod_api::__private::listened(mod_api::HookId::ServerStopped));
        assert!(mod_api::subscribed_by(
            0,
            Some(mod_api::HookId::ServerStopped)
        ));
        mod_api::__private::mark_game_thread();
        let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
        let mut host = World;
        let warnings_and_errors = WARNINGS_AND_ERRORS.load(Ordering::Relaxed);
        let mut samples = Vec::with_capacity(100);
        for round in 0..120 {
            let now = Instant::now();
            for _ in 0..ITERATIONS {
                mod_api::dispatch_server_stopped(std::hint::black_box(&mut host), false);
            }
            let elapsed = now.elapsed().as_nanos();
            if round >= 20 {
                samples.push(elapsed);
            }
        }
        assert_eq!(
            WARNINGS_AND_ERRORS.load(Ordering::Relaxed),
            warnings_and_errors,
            "{label}: listener logged a warning or error"
        );
        samples.sort_unstable();
        let mean = samples.iter().sum::<u128>() as f64 / samples.len() as f64 / ITERATIONS as f64;
        let p95 = samples[94] as f64 / ITERATIONS as f64;
        println!(
            "{label} empty subscribed server_stopped: {ITERATIONS} dispatches/sample mean={mean:.2} ns/dispatch p95={p95:.2} ns/dispatch"
        );
    });
}

#[test]
#[ignore = "measurement; requires a built fixture and shared std"]
fn empty_subscribed_listener() {
    crate::test_stack::on_guest_stack(|| {
        init_log();
        let path = std::env::var("HORIZON_WASM_FIXTURE").unwrap();
        let path = std::path::Path::new(&path);
        let std = std::fs::read(std::env::var("HORIZON_WASM_STD").unwrap()).unwrap();
        let mut manifest = Package::read(path).unwrap().manifest;
        // This fixture variant exercises ordinary hooks and Parts, not game points.
        manifest.inject.clear();
        let info = mod_api::PackageInfo::new(
            manifest.id.clone(),
            manifest.version.clone(),
            mod_api::PackageKind::Code,
            mod_api::Side::Both,
            vec![manifest.id.clone()],
            Vec::new(),
        );
        let package = WasmPackage {
            manifest,
            module: Package::read_wasm(path).unwrap(),
        };
        let loaded = load(&std, vec![package]);
        assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
        assert_eq!(loaded.mods.len(), 1);
        loaded
            .initialize(&[info])
            .expect("wasm initialization failed");
        measure_empty_listener(
            "native",
            vec![ModEntry::new("wasm_fixture", Box::new(fixture::Mod))],
        );
        measure_empty_listener("wasm", loaded.mods);
    })
}

#[path = "../src/test_stack.rs"]
mod test_stack;
