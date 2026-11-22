//! Named scopes in Horizon's built-in profiler. Not on wasm.

use std::{any::Any, sync::OnceLock};

/// The game's scope for a name, None when nothing would see it (nothing records or samples).
type Provider = fn(&'static str) -> Option<Box<dyn Any>>;
static PROVIDER: OnceLock<Provider> = OnceLock::new();

/// A named profiler scope. Dropping it ends the measurement on the current thread.
#[must_use = "keep the guard until the end of the measured scope"]
// no wire: the guard owns a scope in the native thread that opened it.
pub struct Guard {
    _game_guard: Option<Box<dyn Any>>,
}

/// Start a named scope in the running game's timing report. Standalone mod
/// tests have no recorder. Declare the name once and reuse it on every call.
pub fn guard(name: &'static str) -> Guard {
    Guard {
        _game_guard: PROVIDER.get().and_then(|start| start(name)),
    }
}

pub(crate) fn install(provider: Provider) {
    PROVIDER.get_or_init(|| provider);
}
