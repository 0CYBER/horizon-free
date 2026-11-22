//! The server player's impulse context (wind charge, mace launch) over a
//! tick of accepted moves: `ServerGamePacketListenerImpl.handlePlayerPositionChange`
//! ends it once a move is accepted; the grace time counts down in
//! `LivingEntity.tick` (`ImpulseContext::tick_grace_time`, once per body tick).
use crate::motion::{MotionWorld, PlayerMotion};
use horizon_world::living_fall::ImpulseContext;

/// What `handlePlayerPositionChange` reads before it moves the player.
pub struct MoveStart {
    pub grounded: bool,
    gliding: bool,
    spin_attacking: bool,
}

impl MoveStart {
    pub fn of(w: &MotionWorld<'_, '_, '_>) -> Self {
        Self {
            grounded: false,
            gliding: w.player.fall_flying,
            spin_attacking: w.player.auto_spin_attack_ticks > 0,
        }
    }
}

/// After a tick that accepted at least one move: a liquid landing
/// (`hasLandedInLiquid`: a move into a liquid that does not rise), the ground,
/// a climbable, gliding or a spin attack end the context, still deferred by
/// the grace time. A tick with no accepted move runs none of it, as the call
/// sits inside `handlePlayerPositionChange`.
pub fn end_after_moves(w: &mut MotionWorld<'_, '_, '_>, start: &MoveStart, moved: bool) {
    if !moved {
        return;
    }
    let liquid = w.player.fluids.was_touching_water || w.player.is_in_lava();
    let delta_y = w.player.velocity.y;
    let climbing = w.climbable().unwrap_or(false);
    end_context(&mut w.player.impulse, start, delta_y, liquid, climbing);
}

fn end_context(
    impulse: &mut ImpulseContext,
    start: &MoveStart,
    delta_y: f64,
    in_liquid: bool,
    climbing: bool,
) {
    impulse.landed_in_liquid(delta_y, in_liquid);
    impulse.after_move(start.grounded || climbing || start.gliding || start.spin_attacking);
}

/// A spectator's accepted moves end the context; the grace time counts down.
pub fn spectator_tick(player: Option<&mut PlayerMotion>, moved: bool) {
    if let Some(p) = player {
        spectator_step(&mut p.impulse, moved);
    }
}

fn spectator_step(impulse: &mut ImpulseContext, moved: bool) {
    impulse.after_move(moved);
    impulse.tick_grace_time();
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::geometry::Vec3;

    fn launched() -> ImpulseContext {
        let mut c = ImpulseContext::default();
        c.set_ignore_fall_damage(true, Some(Vec3::new(0.0, 64.0, 0.0)));
        c
    }

    fn start(grounded: bool, gliding: bool, spin_attacking: bool) -> MoveStart {
        MoveStart {
            grounded,
            gliding,
            spin_attacking,
        }
    }

    /// `tryResetCurrentImpulseContext` only resets at grace time 0.
    fn after_grace(mut c: ImpulseContext) -> ImpulseContext {
        for _ in 0..40 {
            c.tick_grace_time();
        }
        c
    }

    #[test]
    fn each_reset_condition_ends_the_context_once_the_grace_time_is_over() {
        for (start, climbing) in [
            (start(true, false, false), false),
            (start(false, true, false), false),
            (start(false, false, true), false),
            (start(false, false, false), true),
        ] {
            let mut c = after_grace(launched());
            end_context(&mut c, &start, 0.5, false, climbing);
            assert!(!c.ignores_fall_damage());
        }
    }

    #[test]
    fn a_liquid_landing_ends_it_unless_the_move_rose() {
        let none = start(false, false, false);
        let mut rising = after_grace(launched());
        end_context(&mut rising, &none, 0.1, true, false);
        assert!(rising.ignores_fall_damage());
        let mut landing = after_grace(launched());
        end_context(&mut landing, &none, 0.0, true, false);
        assert!(!landing.ignores_fall_damage());
    }

    #[test]
    fn the_grace_time_defers_every_reset_condition() {
        let mut c = launched();
        end_context(&mut c, &start(true, true, true), -1.0, true, true);
        assert!(c.ignores_fall_damage());
    }

    #[test]
    fn no_condition_leaves_the_context_alone() {
        let mut c = after_grace(launched());
        end_context(&mut c, &start(false, false, false), 0.5, false, false);
        assert!(c.ignores_fall_damage());
    }

    #[test]
    fn a_spectator_keeps_it_through_the_grace_time_then_an_accepted_move_ends_it() {
        let mut c = launched();
        for _ in 0..40 {
            spectator_step(&mut c, true);
            assert!(c.ignores_fall_damage());
        }
        spectator_step(&mut c, false);
        assert!(c.ignores_fall_damage());
        spectator_step(&mut c, true);
        assert!(!c.ignores_fall_damage());
    }
}
