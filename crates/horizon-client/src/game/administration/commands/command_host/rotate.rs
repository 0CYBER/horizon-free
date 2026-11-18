use super::entity::{entity_ref, target_of};
use super::super::{Target, without_knots, world_cmds};
use crate::game::Game;
use horizon_client::command::Context;
use horizon_gameplay::commands::{entity::EntityRef, host::R, rotate::RotateHost};

impl RotateHost for Game<'_> {
    fn rotate_targets(&mut self, ctx: &Context) -> R<Vec<EntityRef>> {
        let targets = without_knots(self.entities(ctx, "target", false)?)?;
        Ok(targets.into_iter().map(entity_ref).collect())
    }
    fn facing_entities(&mut self, ctx: &Context) -> R<Vec<EntityRef>> {
        let faced = self.entities(ctx, "facingEntity", false)?;
        Ok(faced.into_iter().map(entity_ref).collect())
    }
    fn source_rotation(&self) -> [f32; 2] {
        Game::source_rotation(self)
    }
    fn source_anchored_eyes(&self) -> bool {
        self.command_source.is_some_and(|s| s.eyes)
    }
    fn entity_eye_height(&self, entity: EntityRef) -> f64 {
        self.target_eye_height(target_of(entity))
    }
    fn look_at(&self, from: [f64; 3], to: [f64; 3]) -> [f32; 2] {
        world_cmds::look_at(from, to)
    }
    fn set_rotation(&mut self, entity: EntityRef, rotation: [f32; 2]) {
        Game::set_rotation(self, target_of(entity), rotation);
    }
    fn send_player_look_at(
        &mut self,
        entity: EntityRef,
        from_eyes: bool,
        target: [f64; 3],
        facing: Option<(EntityRef, bool)>,
    ) {
        let facing = facing.map(|(e, eyes)| (target_of(e), eyes));
        Game::send_player_look_at(self, target_of(entity), from_eyes, target, facing);
    }
    fn set_player_pitch(&mut self, entity: EntityRef, pitch: f32) {
        if target_of(entity) == Target::Player {
            self.pitch = pitch;
        }
    }
}
