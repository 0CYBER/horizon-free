//! Headless native wind-burst hook regression. Uses the dispenser fixture's
//! synthetic server Game; never starts a server or opens a world.
use mod_api::{Hooks, ModEntry, ServerHost, WindBurstEvent};

struct Protect;
#[mod_api::__subscriptions]
impl Hooks for Protect {
    fn wind_burst(&self, _host: &mut dyn ServerHost, event: &mut WindBurstEvent<'_>) {
        assert_eq!(event.dimension, "minecraft:overworld");
        assert_eq!(event.center, [0.5, 100.5, 0.5]);
        // The protected lever and candle are present in the real ray result.
        assert!(event.blocks.contains(&[0, 100, 0]));
        assert!(event.blocks.contains(&[1, 100, 0]));
        if std::env::var_os("HORIZON_WIND_PROTECT").is_some() {
            event
                .blocks
                .retain(|p| *p != [0, 100, 0] && *p != [1, 100, 0]);
        }
    }
}
fn main() {
    horizon_mod_host::exit_if_compile_child();
    let startup = horizon_client::entry::prepare();
    let mods: Vec<ModEntry> = if std::env::var_os("HORIZON_WIND_NO_MOD").is_some() {
        Vec::new()
    } else {
        vec![ModEntry::new("protect", Box::new(Protect))]
    };
    // Debug Game construction exceeds the Windows main thread's default stack.
    // This worker only loads the isolated component fixture; it has no window.
    std::thread::Builder::new()
        .name("wind-burst-fixture".into())
        .stack_size(64 * 1024 * 1024)
        .spawn(move || horizon_mod_host::main_with(mods, &[], &[], startup))
        .expect("start wind fixture")
        .join()
        .expect("wind fixture panicked");
}
