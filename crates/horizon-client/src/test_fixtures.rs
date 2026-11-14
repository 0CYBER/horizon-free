//! The prepared vanilla data unit tests read, loaded once per test process.
//!
//! Decoding the block registry, baking every block model and reading the
//! movement tables takes seconds in a debug build, and a test that did it
//! for itself spent most of its time there. Tests only read these, so one
//! copy serves them all; each test still builds its own `World` over it.
use crate::{
    assets::{Assets, RenderAssets},
    motion::MotionResources,
};
use horizon_world::block_collision::BlockCollisionRegistry;
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub(crate) struct Vanilla {
    pub assets: Assets,
    pub render: RenderAssets,
    pub collision: BlockCollisionRegistry<'static>,
    pub motion: MotionResources,
}

/// The prepared vanilla data at `HORIZON_TEST_RESOURCES`, or the checkout's
/// `.local/horizon-client/<version>`; `None` when neither has it (CI).
pub(crate) fn vanilla() -> Option<&'static Vanilla> {
    static VANILLA: OnceLock<Option<Vanilla>> = OnceLock::new();
    let resources = VANILLA
        .get_or_init(|| {
            let root = std::env::var_os("HORIZON_TEST_RESOURCES")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../.local/horizon-client")
                        .join(horizon_core::VANILLA_VERSION)
                });
            if !root.join("movement-properties.nbt").exists() {
                return None;
            }
            let assets = Assets::load(&root).unwrap();
            let render = assets
                .reload(&horizon_assets::resource_source::ResourceRoot::directory(
                    root.clone(),
                ))
                .unwrap();
            let collision = crate::collisions::load(&root, assets.registry).unwrap();
            let motion = MotionResources::load(&root, &assets).unwrap();
            Some(Vanilla {
                assets,
                render,
                collision,
                motion,
            })
        })
        .as_ref();
    if resources.is_none() {
        report_missing();
    }
    resources
}

/// Attribute missing prepared data to every calling test, including cached misses.
pub(crate) fn report_missing() {
    use std::{
        collections::HashSet,
        io::Write,
        sync::{LazyLock, Mutex},
    };
    static SKIPPED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("<unnamed test>");
    let mut skipped = SKIPPED.lock().unwrap();
    if skipped.insert(name.to_owned()) {
        writeln!(
            std::io::stderr().lock(),
            "skipped: no vanilla data; test={name}; skipped tests={}",
            skipped.len()
        )
        .expect("writing missing-data skip report");
    }
}
