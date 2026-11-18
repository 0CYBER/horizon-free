//! `Game` as the host of the commands that moved to `horizon_gameplay::commands`. Each command
//! family has its trait there and its impl in a file of `command_host/`; every method
//! calls the `Game` code it names and adds no logic.
pub(super) mod data;
mod entity;
mod fillbiome;
mod forceload;
mod gamerule;
mod help;
mod misc;
mod profiler;
pub(super) mod profiles;
mod publish;
mod recipe;
mod rotate;
mod sound;
mod swing;
pub(super) mod tag;
mod time;
mod version;
mod weather;
mod worldborder;

use super::Game;
use horizon_client::{command::CommandError, text::Text};
use horizon_gameplay::commands::host::CommandHost;

impl CommandHost for Game<'_> {
    fn error(&self, key: &str, args: &[&dyn std::fmt::Display]) -> CommandError {
        super::err(key, args)
    }
    fn feedback(&mut self, message: Text) {
        Game::feedback(self, message);
    }
    fn feedback_unbroadcast(&mut self, message: Text) {
        Game::feedback_unbroadcast(self, message);
    }
}
