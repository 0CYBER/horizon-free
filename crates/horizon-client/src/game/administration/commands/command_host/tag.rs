use crate::game::{Game, commands::Target};
use horizon_client::{command::Context, text::Text};
use horizon_gameplay::commands::{
    host::{CommandHost, R},
    tag::TagHost,
};

/// The game, with the entities `/tag` acts on.
pub(in crate::game) struct TagTargets<'a, 'g> {
    pub(in crate::game) game: &'a mut Game<'g>,
    pub(in crate::game) targets: Vec<Target>,
}

impl CommandHost for TagTargets<'_, '_> {
    fn error(&self, key: &str, args: &[&dyn std::fmt::Display]) -> horizon_client::command::CommandError {
        <Game as CommandHost>::error(self.game, key, args)
    }
    fn feedback(&mut self, message: Text) {
        Game::feedback(self.game, message);
    }
    fn feedback_unbroadcast(&mut self, message: Text) {
        Game::feedback_unbroadcast(self.game, message);
    }
}

impl TagHost for TagTargets<'_, '_> {
    fn select_targets(&mut self, ctx: &Context, skip_knots: bool) -> R<usize> {
        let targets = Game::entities(self.game, ctx, "targets", false)?;
        self.targets = if skip_knots {
            crate::game::commands::without_knots(targets)?
        } else {
            targets
        };
        Ok(self.targets.len())
    }
    fn tags_of(&self, index: usize) -> Vec<String> {
        Game::tags_of(self.game, self.targets[index])
    }
    fn target_name(&self, index: usize) -> Text {
        Game::target_name(self.game, self.targets[index])
    }
    fn set_tags(&mut self, index: usize, tags: &[String]) {
        Game::set_entity_tags(self.game, self.targets[index], tags);
    }
}
