//! `/rotate`.
use super::entity::{EntityHost, EntityRef};
use super::host::R;
use horizon_client::{
    command::{Arg, Context},
    text::Text,
};

/// What `/rotate` needs from the game.
pub trait RotateHost: EntityHost {
    /// The entities of the `target` argument, without natural leash knots.
    fn rotate_targets(&mut self, ctx: &Context) -> R<Vec<EntityRef>>;
    /// The entities of the `facingEntity` argument.
    fn facing_entities(&mut self, ctx: &Context) -> R<Vec<EntityRef>>;
    /// The command source's rotation.
    fn source_rotation(&self) -> [f32; 2];
    /// Whether the command source is anchored at its eyes.
    fn source_anchored_eyes(&self) -> bool;
    /// The entity's eye height above its position.
    fn entity_eye_height(&self, entity: EntityRef) -> f64;
    /// `Entity.lookAt`: yaw and pitch from `from` towards `to`.
    fn look_at(&self, from: [f64; 3], to: [f64; 3]) -> [f32; 2];
    /// `Entity.forceSetRotation`, not a teleport.
    fn set_rotation(&mut self, entity: EntityRef, rotation: [f32; 2]);
    /// The look-at packet of `/rotate facing`, for a player entity.
    fn send_player_look_at(
        &mut self,
        entity: EntityRef,
        from_eyes: bool,
        target: [f64; 3],
        facing: Option<(EntityRef, bool)>,
    );
    /// Sets the pitch of the player in place when the entity is that player.
    fn set_player_pitch(&mut self, entity: EntityRef, pitch: f32);
}

/// `/rotate <target> <rotation>|facing ...`.
pub fn rotate(host: &mut dyn RotateHost, ctx: &Context) -> R<i32> {
    let targets = host.rotate_targets(ctx)?;
    let Some(t) = targets.first().copied() else {
        return Ok(0);
    };
    let Some(pos) = host.entity_position(t) else {
        return Ok(0);
    };
    let mut look_at_target = None;
    let rot = if let Some(Arg::Coords(c)) = ctx.get("rotation") {
        // RotateCommand.rotate: relative parts are offsets from the command
        // source's rotation (WorldCoordinates.getRotation); the entity's own
        // rotation cancels out in Entity.forceSetRotation.
        c.rotation(host.source_rotation())
    } else {
        let mut faced_entity = None;
        let to = if ctx.has("entity") {
            let faced = host.facing_entities(ctx)?;
            let Some(f) = faced.first().copied() else {
                return Ok(0);
            };
            let mut p = host.entity_position(f).unwrap_or(pos);
            let eyes = matches!(ctx.get("facingAnchor"), Some(Arg::Anchor(true)));
            if eyes {
                p[1] += host.entity_eye_height(f);
            }
            faced_entity = Some((f, eyes));
            p
        } else {
            host.position_arg(ctx, "facingLocation").unwrap_or(pos)
        };
        // LookAt.perform: Entity.lookAt from the source's anchor (the
        // feet, unless `anchored eyes`).
        let anchored_eyes = host.source_anchored_eyes();
        let eye = if anchored_eyes {
            host.entity_eye_height(t)
        } else {
            0.0
        };
        let from = [pos[0], pos[1] + eye, pos[2]];
        look_at_target = Some((anchored_eyes, to, faced_entity));
        host.look_at(from, to)
    };
    // RotateCommand: Entity.forceSetRotation, not a teleport.
    host.set_rotation(t, rot);
    if let Some((eyes, to, entity)) = look_at_target {
        host.send_player_look_at(t, eyes, to, entity);
    }
    host.set_player_pitch(t, rot[1].clamp(-90.0, 90.0));
    let name = host.entity_name(t);
    host.feedback(Text::translate("commands.rotate.success", &[name]));
    Ok(1)
}
