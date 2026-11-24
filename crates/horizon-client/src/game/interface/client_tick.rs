//! The window-only lead of `Game::tick`.
use super::{Game, Screen};
use horizon_client::Result;
use horizon_system::power::{self, Source, Thermal};
use std::sync::atomic::{AtomicU32, Ordering};

/// The power changes already said in the log. The sampler counts them; this file prints one line
/// for each and nothing at all while the count stands still.
static POWER_SAID: AtomicU32 = AtomicU32::new(0);

impl Game<'_> {
    pub(super) fn tick_client_lead(&mut self) -> Result<()> {
        // The machine's power state, on the window's own tick and never in the frame loop: the
        // sampler read the OS on its thread, so this is one lock of its snapshot and a copy of it
        // (`horizon_system::power`).
        say_power_change();
        self.tick_debug_keys()?;
        self.tick_debug_charts();
        self.tick_tutorial();
        if self.screen == Screen::Creative {
            self.refresh_creative_tabs();
            // CreativeModeInventoryScreen.containerTick: a player without
            // infinite materials gets the survival inventory instead.
            if !self.creative {
                // The carried stack lives in `inventoryMenu` and stays on the
                // cursor; no `removed` on a client player.
                self.set_screen(Screen::Inventory);
            }
        }
        Ok(())
    }
}

/// One log line for each change the OS's power sampler counted (a cable, the energy saver, the
/// CPU's thermal limit, or the machine waking from sleep), and none for a tick that changed
/// nothing: the line says what the machine did, without the game saying it again every tick.
fn say_power_change() {
    // One snapshot for the whole line: the count and the fields it explains come from the same
    // tick, so a change counted here can't be paired with the state before it.
    let sample = power::sample();
    // `swap`, not `load` then `store`: two ticks can't both say the same change.
    if POWER_SAID.swap(sample.transitions, Ordering::Relaxed) == sample.transitions {
        return;
    }
    let thermal = match sample.thermal {
        Thermal::Unknown => "no thermal report".to_owned(),
        Thermal::Nominal => "not thermally limited".to_owned(),
        Thermal::Limited(percent) => format!("limited to {percent}% of full speed"),
    };
    let source = match sample.source {
        Source::Unknown => "unknown",
        source => source.as_str(),
    };
    log::info!(
        "Horizon: power is {source}, energy saver {}, {thermal}, {} resume(s) so far",
        if sample.saver { "on" } else { "off" },
        sample.resumes,
    );
}
