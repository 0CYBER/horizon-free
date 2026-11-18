use crate::game::Game;
use horizon_client::{command::Context, text::Text};
use horizon_gameplay::commands::{forceload::ForceloadHost, host::R};

impl ForceloadHost for Game<'_> {
    fn dimension_id(&self) -> String {
        self.dim.id().to_string()
    }
    fn commanded_chunks(&self) -> Vec<(i32, i32)> {
        self.streaming.commanded().iter().copied().collect()
    }
    fn is_commanded(&self, key: (i32, i32)) -> bool {
        self.streaming.commanded().contains(&key)
    }
    fn command_failure(&mut self, message: Text) {
        Game::command_failure(self, message);
    }
    fn column_arg(&self, ctx: &Context, name: &str) -> R<[i32; 2]> {
        Game::column_arg(self, ctx, name)
    }
    fn clear_forced(&mut self) {
        self.streaming.clear_forced();
    }
    fn set_forced_loaded(&mut self, x: i32, z: i32, forced: bool) -> bool {
        self.streaming
            .set_forced_loaded(&mut self.world, x, z, forced)
    }
}
