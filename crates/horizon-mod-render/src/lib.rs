//! Unstable native rendering escape hatch. No API stability is promised.
#[cfg(target_arch = "wasm32")]
compile_error!("horizon-mod-render is a native-only escape hatch");

/// Raw GPU frame data and the places native mods can inject into.
pub mod render;
