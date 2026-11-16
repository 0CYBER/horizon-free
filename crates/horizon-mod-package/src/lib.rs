//! Horizon's mod files (`docs/modding/src/how-to/content-mods.md`): content mods (`.hmod`: blocks, items, data and
//! assets, no code), modpacks and, with `index`, the mod registry's index. Code mods are
//! native (docs/modding/src/how-to/native-mods.md).
pub mod content;
pub mod dependencies;
pub mod enabled;
pub mod game_identity;
#[cfg(all(
    feature = "runtime-process",
    any(target_os = "linux", target_os = "macos", windows)
))]
pub mod holder;
pub mod icon;
#[cfg(feature = "index")]
pub mod index;
pub mod install_lock;
pub mod internals;
pub mod layouts;
#[cfg(all(
    feature = "runtime-process",
    any(target_os = "linux", target_os = "macos", windows)
))]
pub mod local_control;
#[cfg(feature = "runtime-process")]
pub mod locks;
pub mod manifest;
pub mod mixins_tool;
pub mod mod_list;
#[cfg(not(target_arch = "wasm32"))]
pub mod modpack;
#[cfg(feature = "runtime-process")]
pub mod output;
pub mod packages;
pub mod runtime;
#[cfg(feature = "runtime-process")]
pub mod runtime_leases;
pub mod sdk;
pub mod selection;

pub use content::Content;
pub use manifest::{Capability, Manifest, Package, find};
/// A build's surface (`surface.json`) and a mod's uses table: the SDK's records the fit test
/// compares.
#[cfg(feature = "prebuilt")]
pub use mod_surface;
/// The SDK's build-pinned data files and their common header.
pub use point_shape::build_data;
