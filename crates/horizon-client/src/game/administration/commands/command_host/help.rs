use crate::game::Game;
use horizon_client::command::{Dispatcher, Registries};
use horizon_gameplay::commands::help::HelpHost;
use std::sync::Arc;

impl HelpHost for Game<'_> {
    fn dispatcher(&self) -> Arc<Dispatcher> {
        self.ui.dispatcher.0.clone()
    }
    fn registries(&self) -> &Registries {
        &self.ui.registries
    }
    fn permission_level(&self) -> u8 {
        self.ui.registries.permission
    }
}
