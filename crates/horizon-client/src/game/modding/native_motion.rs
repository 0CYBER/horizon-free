//! The player's motion for mods (docs/modding/src/how-to/native-mods.md, "A player's motion"): a push for the local player, on a
//! client.
use super::Game;

/// Each component of a push is clamped to this, in blocks a tick.
const PUSH_MAX: f32 = 4.0;
/// The velocity after a push: `kick` added, each component clamped; none when one of `kick`'s isn't a number.
fn pushed(velocity: [f64; 3], kick: [f32; 3]) -> Option<[f64; 3]> {
    if !kick.iter().all(|v| v.is_finite()) {
        return None;
    }
    Some(std::array::from_fn(|i| {
        velocity[i] + f64::from(kick[i].clamp(-PUSH_MAX, PUSH_MAX))
    }))
}

/// Validates a direct velocity write: none when any component is non-finite.
fn finite_velocity(velocity: [f64; 3]) -> Option<[f64; 3]> {
    velocity.iter().all(|v| v.is_finite()).then_some(velocity)
}

#[allow(dead_code)]
impl Game<'_> {
    /// For a client mod: a push added to the local player's velocity, in blocks a tick, as an
    /// explosion's knockback is (the window's player moves itself, as vanilla's client does). Each component is
    /// clamped to -4..4; a spectator, or a value that is not a number, is left alone.
    pub(super) fn push_player(&mut self, kick: [f32; 3]) {
        if let Some(p) = self.player.as_mut()
            && !p.spectator
            && let Some([x, y, z]) = pushed([p.velocity.x, p.velocity.y, p.velocity.z], kick)
        {
            p.velocity = horizon_core::geometry::Vec3::new(x, y, z);
            if kick[1] > 0.0 {
                p.ground.on_ground = false;
            }
        }
    }

    /// For a client mod: sets the local player's velocity directly, in blocks a tick, without
    /// sending any network packet (typically called from `player_tick_pre`). Non-finite
    /// values, a spectator, or no local player leave velocity unchanged.
    pub(super) fn set_local_player_velocity(&mut self, velocity: [f64; 3]) {
        if let Some(p) = self.player.as_mut()
            && !p.spectator
            && let Some([x, y, z]) = finite_velocity(velocity)
        {
            p.velocity = horizon_core::geometry::Vec3::new(x, y, z);
        }
    }

    /// For a client mod: sets the local player's on-ground flag directly without sending any
    /// network packet (typically called from `player_tick_pre`). A spectator or no local
    /// player is left alone.
    pub(super) fn set_local_player_on_ground(&mut self, on_ground: bool) {
        if let Some(p) = self.player.as_mut()
            && !p.spectator
        {
            p.ground.on_ground = on_ground;
        }
    }
}

#[cfg(test)]
mod motion_tests {
    use super::*;

    #[test]
    fn a_push_adds_to_the_velocity_clamped() {
        assert_eq!(
            pushed([0.1, -0.08, 0.0], [0.8, 0.55, -0.5]),
            Some([0.1 + 0.8f32 as f64, -0.08 + 0.55f32 as f64, -0.5])
        );
        assert_eq!(pushed([0.0; 3], [9.0, -9.0, 0.0]), Some([4.0, -4.0, 0.0]));
        assert_eq!(pushed([1.0; 3], [f32::NAN, 0.0, 0.0]), None);
        assert_eq!(pushed([1.0; 3], [0.0, f32::INFINITY, 0.0]), None);
    }

    #[test]
    fn local_player_velocity_rejects_non_finite_components() {
        assert_eq!(finite_velocity([0.25, 0.42, -0.5]), Some([0.25, 0.42, -0.5]));
        assert_eq!(finite_velocity([f64::NAN, 0.0, 0.0]), None);
        assert_eq!(finite_velocity([0.0, f64::INFINITY, 0.0]), None);
        assert_eq!(finite_velocity([0.0, 0.0, f64::NEG_INFINITY]), None);
    }

    #[test]
    fn local_player_velocity_and_on_ground_update_player_and_ignore_spectator() {
        if std::env::var_os("HORIZON_TEST_RESOURCES").is_none() {
            return;
        }
        let mut game = crate::game::native_machines::tests::server_game();
        game.player = Some(
            horizon_client::motion::PlayerMotion::new(
                game.motion,
                horizon_core::geometry::Vec3::new(0.0, 64.0, 0.0),
            )
            .unwrap(),
        );

        <dyn mod_api::ClientHost>::set_local_player_velocity(&mut game, [0.3, 0.42, -0.15]);
        <dyn mod_api::ClientHost>::set_local_player_on_ground(&mut game, false);
        let p = game.player.as_ref().unwrap();
        assert_eq!([p.velocity.x, p.velocity.y, p.velocity.z], [0.3, 0.42, -0.15]);
        assert!(!p.ground.on_ground);

        // Non-finite velocity is ignored without disturbing existing velocity.
        <dyn mod_api::ClientHost>::set_local_player_velocity(&mut game, [f64::NAN, 1.0, 0.0]);
        let p = game.player.as_ref().unwrap();
        assert_eq!([p.velocity.x, p.velocity.y, p.velocity.z], [0.3, 0.42, -0.15]);

        // Spectators are left alone by both setters.
        game.player.as_mut().unwrap().spectator = true;
        <dyn mod_api::ClientHost>::set_local_player_velocity(&mut game, [1.0, 2.0, 3.0]);
        <dyn mod_api::ClientHost>::set_local_player_on_ground(&mut game, true);
        let p = game.player.as_ref().unwrap();
        assert_eq!([p.velocity.x, p.velocity.y, p.velocity.z], [0.3, 0.42, -0.15]);
        assert!(!p.ground.on_ground);
    }
}
