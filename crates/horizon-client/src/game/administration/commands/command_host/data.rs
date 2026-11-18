use crate::game::Game;
use super::super::data::DataRef;
use horizon_client::{nbt_path::NbtPath, text::Text};
use horizon_core::nbt::Tag;
use horizon_gameplay::commands::{
    data::DataHost,
    host::{CommandHost, R},
};

/// The game, acting on one data accessor: what `/data` reads and writes.
pub(in crate::game) struct DataTarget<'a, 'g> {
    pub(in crate::game) game: &'a mut Game<'g>,
    pub(in crate::game) target: DataRef,
}

impl CommandHost for DataTarget<'_, '_> {
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

impl DataHost for DataTarget<'_, '_> {
    fn data_of(&mut self) -> R<Tag> {
        Game::data_of(self.game, &self.target)
    }
    fn set_data_of(&mut self, tag: Tag) -> R<()> {
        Game::set_data_of(self.game, &self.target, tag)
    }
    fn modified_text(&self) -> Text {
        Game::modified_text(self.game, &self.target)
    }
    fn query_text(&self, data: &Tag, path: Option<&NbtPath>) -> Text {
        Game::query_text(self.game, &self.target, data, path)
    }
    fn get_text(&self, path: &NbtPath, scale: f64, value: i32) -> Text {
        Game::get_text(self.game, &self.target, path, scale, value)
    }
    fn path_arg(&self, ctx: &horizon_client::command::Context, name: &str) -> R<NbtPath> {
        Game::path_arg(self.game, ctx, name)
    }
    fn system_feedback(&mut self, message: Text) {
        Game::system_feedback(self.game, message);
    }
    fn source_data(&mut self, ctx: &horizon_client::command::Context) -> R<Tag> {
        let source = Game::data_ref(self.game, ctx, 1)?;
        Game::data_of(self.game, &source)
    }
    fn compute_source(&mut self, ctx: &horizon_client::command::Context, integer: bool) -> R<Tag> {
        let compute = Game::compute_context(self.game, ctx)?;
        let registry = if integer {
            "minecraft:context_int_provider"
        } else {
            "minecraft:context_float_provider"
        };
        let (_, provider) = Game::provider_arg(self.game, ctx, registry)?;
        if integer {
            // ContextIntProvider.getInt: an ArithmeticException reads as 0.
            let value = Game::provider_int(self.game, &provider, &compute, 0).unwrap_or(0);
            Ok(Tag::Int(value))
        } else {
            // ContextFloatProvider.getFloat: a failure or a non-finite
            // value reads as 0.
            let value = Game::provider_float(self.game, &provider, &compute, 0).unwrap_or(0.0);
            let value = if value.is_finite() { value } else { 0.0 };
            Ok(Tag::Float(value.to_bits()))
        }
    }
    fn modify_data(&mut self, ctx: &horizon_client::command::Context) -> R<i32> {
        Game::cmd_data_modify(self.game, ctx, self.target.clone())
    }
    fn keep_unsaved_storage(&mut self, data: Tag) {
        if let DataRef::Storage(id) = &self.target {
            self.game.score.storage.set_unsaved(id, data);
        }
    }
}
