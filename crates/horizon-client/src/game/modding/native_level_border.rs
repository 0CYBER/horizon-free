//! World-border writes for native server mods through a `LevelRef`, as `/worldborder` makes
//! them: `WorldBorder.setCenter`, `setSize` / `lerpSizeBetween`, `setDamagePerBlock`,
//! `setSafeZone`, `setWarningBlocks` and `setWarningTime`. Horizon keeps one border for every
//! level, so a write through any level the server has opened moves it everywhere. The players
//! are told on the next host sync, as for the command.
use super::Game;
use crate::game::level_control_hooks::LevelControl;
use horizon_client::worldborder::{MAX_CENTER, MAX_SIZE};
use mod_api::LevelRef;

impl Game<'_> {
    /// `WorldBorder.setCenter(x, z)`; the border-center hook hears it. The centre it already
    /// has is true with nothing heard, as `/worldborder center` changes nothing. False for a
    /// centre that isn't finite or lies past 2.9999984E7 on x or z, inside the border-center
    /// hook, and for a stale handle, a level the server has not opened or a client.
    pub(super) fn level_set_world_border_center(
        &mut self,
        level: LevelRef,
        center: [f64; 2],
    ) -> bool {
        if self.native_level_world(level).is_none()
            || !center
                .iter()
                .all(|v| v.is_finite() && v.abs() <= MAX_CENTER)
            || LevelControl::BorderCenter.is_heard()
        {
            return false;
        }
        if self.border.center() == center {
            return true;
        }
        self.border.set_center(center);
        self.native_world_border_center_changed();
        true
    }
    /// `setSize(size)` when `ticks` is 0, otherwise `lerpSizeBetween(current, size, ticks)`; the
    /// border-bounds hook hears it. A still border set to the size it has is true with nothing
    /// heard. False for a size outside 1 to 5.9999968E7, a negative `ticks`, inside the
    /// border-bounds hook, a stale handle, a level the server has not opened or a client.
    pub(super) fn level_set_world_border_size(
        &mut self,
        level: LevelRef,
        size: f64,
        ticks: i64,
    ) -> bool {
        if self.native_level_world(level).is_none()
            || !(1.0..=MAX_SIZE).contains(&size)
            || ticks < 0
            || LevelControl::BorderBounds.is_heard()
        {
            return false;
        }
        if ticks == 0 && self.border.remaining_ticks() == 0 && self.border.size() == size {
            return true;
        }
        self.border.lerp_to(size, ticks);
        self.native_world_border_bounds_changed();
        true
    }
    /// `setDamagePerBlock` and `setSafeZone`: the damage a player outside takes per block past
    /// the safe zone, and how far outside they are safe. False for a value that is negative
    /// or not finite, a stale handle, a level the server has not opened or a client.
    pub(super) fn level_set_world_border_damage(
        &mut self,
        level: LevelRef,
        per_block: f64,
        safe_zone: f64,
    ) -> bool {
        let ok = |v: f64| v.is_finite() && v >= 0.0;
        if self.native_level_world(level).is_none() || !ok(per_block) || !ok(safe_zone) {
            return false;
        }
        self.border.damage_per_block = per_block;
        self.border.safe_zone = safe_zone;
        true
    }
    /// `setWarningBlocks` and `setWarningTime`: how near the border, in blocks, and how soon a
    /// shrinking border reaches the player, in ticks, before their screen tints red. False
    /// for a negative value, a stale handle, a level the server has not opened or a client.
    pub(super) fn level_set_world_border_warning(
        &mut self,
        level: LevelRef,
        distance: i32,
        ticks: i32,
    ) -> bool {
        if self.native_level_world(level).is_none() || distance < 0 || ticks < 0 {
            return false;
        }
        self.border.warning_distance = distance;
        self.border.warning_time = ticks;
        true
    }
}

#[cfg(test)]
mod tests {
    use mod_api::{LevelRef, WorldEpoch};

    #[test]
    #[ignore = "daily"]
    fn border_writes_move_the_shared_border_and_refuse_bad_values() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());

            assert!(level.set_world_border_center(game, [100.0, -50.0]));
            assert_eq!(game.border.center(), [100.0, -50.0]);
            assert!(!level.set_world_border_center(game, [f64::NAN, 0.0]));
            assert!(
                !level.set_world_border_center(game, [0.0, 3.0e7]),
                "past the limit"
            );
            assert!(!stale.set_world_border_center(game, [0.0, 0.0]));
            assert_eq!(
                game.border.center(),
                [100.0, -50.0],
                "refusals change nothing"
            );

            assert!(level.set_world_border_size(game, 200.0, 0));
            assert_eq!(level.world_border(game), Some([0.0, -150.0, 200.0, 50.0]));
            assert!(level.set_world_border_size(game, 100.0, 20));
            assert_eq!(game.border.remaining_ticks(), 20, "a timed size lerps");
            assert_eq!(game.border.target_size(), 100.0);
            assert!(!level.set_world_border_size(game, 0.5, 0));
            assert!(!level.set_world_border_size(game, 100.0, -1));

            assert!(level.set_world_border_damage(game, 0.5, 3.0));
            assert_eq!(
                (game.border.damage_per_block, game.border.safe_zone),
                (0.5, 3.0)
            );
            assert!(!level.set_world_border_damage(game, -1.0, 3.0));
            assert!(level.set_world_border_warning(game, 8, 100));
            assert_eq!(
                (game.border.warning_distance, game.border.warning_time),
                (8, 100)
            );
            assert!(!level.set_world_border_warning(game, -1, 100));
            assert!(!stale.set_world_border_warning(game, 1, 1));
        });
    }
}
