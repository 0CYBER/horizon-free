//! The window-only lead of `Game::tick`.
use super::{Game, Screen};
use horizon_client::Result;

impl Game<'_> {
    pub(super) fn tick_client_lead(&mut self) -> Result<()> {
        self.tick_debug_keys()?;
        self.tick_debug_charts();
        self.tick_tutorial();
        if self.screen == Screen::Creative {
            self.refresh_creative_tabs();
            // CreativeModeInventoryScreen.containerTick: a player without
            // infinite materials gets the survival inventory instead.
            if !self.creative {
                if let Some(c) = self.carried.take() {
                    let _ = self.add_stack_to_inventory(c);
                }
                self.set_screen(Screen::Inventory);
            }
        }
        Ok(())
    }
}
