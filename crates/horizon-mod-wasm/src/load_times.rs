//! Where loading the wasm mods spends its time at startup, by phase, for the one line the log gets
//! once they are initialized. The clock is read only when that line would be written.
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

#[derive(Clone, Copy)]
pub(crate) enum Phase {
    /// Finding, reading and checking the installed packages.
    Packages,
    /// Each member's module: its layout read, then the cache's lookup and map, or a compile.
    Modules,
    /// Linking a group, its modules and instances included.
    Group,
    /// Instantiating a group's members and running their constructors.
    Instantiate,
    /// Asking each mod for its subscriptions and checking them.
    Subscribe,
    /// The mods' `INIT` calls and point listeners.
    Initialize,
}

static NANOS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];

/// Adds the time until it is dropped to its phase.
pub(crate) struct Timer(Phase, Option<Instant>);

impl Drop for Timer {
    fn drop(&mut self) {
        if let Some(started) = self.1 {
            let nanos = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
            NANOS[self.0 as usize].fetch_add(nanos, Ordering::Relaxed);
        }
    }
}

/// Times `phase` until the returned guard drops, when the summary would be logged.
pub(crate) fn timer(phase: Phase) -> Timer {
    Timer(
        phase,
        log::log_enabled!(log::Level::Info).then(Instant::now),
    )
}

/// Logs the phases' totals for `modules` loaded mods, once they are initialized.
pub(crate) fn report(modules: usize) {
    if modules == 0 || !log::log_enabled!(log::Level::Info) {
        return;
    }
    let ms = |phase: Phase| NANOS[phase as usize].load(Ordering::Relaxed) as f64 / 1e6;
    let (modules_ms, instantiate_ms) = (ms(Phase::Modules), ms(Phase::Instantiate));
    let link_ms = (ms(Phase::Group) - modules_ms - instantiate_ms).max(0.0);
    log::info!(
        "Horizon: wasm mods loaded ({modules}): packages {:.0} ms, modules {modules_ms:.0} ms, \
         linking {link_ms:.0} ms, instantiation {instantiate_ms:.0} ms, subscriptions {:.0} ms, \
         initialization {:.0} ms",
        ms(Phase::Packages),
        ms(Phase::Subscribe),
        ms(Phase::Initialize),
    );
}
