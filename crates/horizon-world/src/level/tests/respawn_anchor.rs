//! Respawn anchors: charging with glowstone and the empty-hand click.
use super::*;

impl World<'_> {
    /// Right-click the north face of `pos` with an empty main hand, the
    /// player's respawn point at `respawn` (`ServerPlayer.RespawnConfig`).
    fn use_empty_hand_at_respawn(&mut self, pos: BlockPos, respawn: BlockPos) -> InteractionResult {
        let player = PlayerContext {
            yaw: 0.0,
            pitch: 0.0,
            may_build: true,
            can_eat: true,
            game_master: false,
            respawn_point: Some(respawn),
        };
        let hit = horizon_core::geometry::Vec3::new(
            f64::from(pos.x) + 0.5,
            f64::from(pos.y) + 0.5,
            f64::from(pos.z),
        );
        self.f.behaviours.use_without_item(
            &mut self.level,
            &mut self.ticks,
            pos,
            Direction::North,
            hit,
            player,
        )
    }
}

#[test]
fn glowstone_charges_a_respawn_anchor_up_to_four() {
    // RespawnAnchorBlock.useItemOn: glowstone on an anchor below 4 charges
    // charges it and the click ends (SUCCESS), so no glowstone is placed (#3034).
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    let empty = w.id("respawn_anchor", &[("charges", "0")]);
    w.put(pos, empty);
    for charges in 1..=4 {
        assert_eq!(
            w.use_item(pos, "minecraft:glowstone"),
            InteractionResult::Success
        );
        assert_eq!(w.prop(pos, "charges"), charges.to_string());
    }
    // A full anchor can't be charged: TRY_WITH_EMPTY_HAND, then useWithoutItem
    // (the respawn point), which leaves the charges alone.
    w.use_item(pos, "minecraft:glowstone");
    assert_eq!(w.prop(pos, "charges"), "4");
    // Anything else on an empty anchor passes, so the item is used instead.
    w.put(pos, empty);
    assert_eq!(w.use_item(pos, "minecraft:stone"), InteractionResult::Pass);
    assert_eq!(w.prop(pos, "charges"), "0");
}

#[test]
fn a_click_on_your_own_respawn_anchor_does_not_swing() {
    // RespawnAnchorBlock.useWithoutItem: an anchor that is already the
    // player's respawn point answers CONSUME, whose swing source is NONE, so
    // the arm does not swing; a new point is SUCCESS_SERVER, which does.
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    let anchor = w.id("respawn_anchor", &[("charges", "2")]);
    w.put(pos, anchor);
    let elsewhere = BlockPos::new(9, 1, 9);
    let fresh = w.use_empty_hand_at_respawn(pos, elsewhere);
    assert_eq!(fresh, InteractionResult::Success);
    assert!(fresh.should_swing());
    let own = w.use_empty_hand_at_respawn(pos, pos);
    assert_eq!(own, InteractionResult::Consume);
    assert!(!own.should_swing());
    // CONSUME still ends the click (DEFAULT_BLOCK_USE), it just does not swing.
    assert!(own.consumes_action());
    // An empty anchor passes whatever the respawn point is.
    w.put(pos, w.id("respawn_anchor", &[("charges", "0")]));
    assert_eq!(
        w.use_empty_hand_at_respawn(pos, pos),
        InteractionResult::Pass
    );
}
