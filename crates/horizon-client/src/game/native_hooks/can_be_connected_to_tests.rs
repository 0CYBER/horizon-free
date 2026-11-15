#![cfg(test)]
//! `can_be_connected_to` answered by the block a fence, wall or pane connects to: a lantern's
//! part makes a fence and a wall connect to it, a stone's part refuses its sturdy face, a
//! panicking part keeps vanilla's answer, and a connector's own `connects_to` has the last word.
use crate::game::native_entity_refs::tests::with_game;
use horizon_core::{connection_kind::ConnectionKind, geometry::Direction, position::BlockPos};
use mod_api::{BlockPart, CanBeConnectedToEvent, ConnectsToEvent, Hooks, LevelHost, Registrar};
use std::sync::Arc;

/// The answer of a target block's part, panicking for `None`.
#[derive(Clone)]
struct Target(Option<bool>);
#[mod_api::horizon_part]
impl BlockPart for Target {
    fn can_be_connected_to(
        &self,
        level: &dyn LevelHost,
        event: &mut CanBeConnectedToEvent,
    ) -> Option<bool> {
        // The connector is west of the target, at x 0.
        assert_eq!(event.direction, Direction::West);
        assert_eq!(event.pos.x, 1);
        assert!(level.state_block(event.connector).is_some());
        Some(self.0.expect("a part that panics"))
    }
}
/// A fence that refuses every connection.
struct Refusing;
#[mod_api::horizon_part]
impl BlockPart for Refusing {
    fn connects_to(&self, _: &dyn LevelHost, event: &mut ConnectsToEvent) {
        assert_eq!(event.kind, ConnectionKind::Fence);
        event.connected = false;
    }
}
struct Targets;
#[mod_api::__subscriptions]
impl Hooks for Targets {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for (name, answer) in [
            ("minecraft:lantern", Some(true)),
            ("minecraft:stone", Some(false)),
            ("minecraft:glass", None),
        ] {
            let key = r.block_key(name);
            r.block_part(key, Target(answer));
        }
        let birch = r.block_key("minecraft:birch_fence");
        r.block_part(birch, Refusing);
    }
}

#[test]
#[ignore = "daily"]
fn the_block_beside_a_fence_answers_whether_it_connects() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("targets", Box::new(Targets))].into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            // The connector at x 0 and the target east of it at x 1, on row `z`: the
            // connector's east side.
            let mut place = |connector: &str, target: &str, z| {
                let [connector, target] =
                    [connector, target].map(|name| registry.default_state(name).unwrap());
                let at = BlockPos::new(0, 64, z);
                game.with_level(|b, level, ticks| {
                    b.set_block(level, ticks, at, connector, 3);
                    b.set_block(level, ticks, BlockPos::new(1, 64, z), target, 3);
                });
                registry
                    .state(game.world.block(0, 64, z))
                    .unwrap()
                    .properties["east"]
                    .clone()
            };
            assert_eq!(place("minecraft:oak_fence", "minecraft:lantern", 0), "true");
            assert_eq!(
                place("minecraft:oak_fence", "minecraft:soul_lantern", 2),
                "false"
            );
            assert_eq!(place("minecraft:oak_fence", "minecraft:stone", 4), "false");
            assert_eq!(
                place("minecraft:cobblestone_wall", "minecraft:lantern", 6),
                "low"
            );
            // A panicking part is contained: glass keeps vanilla's sturdy face.
            assert_eq!(place("minecraft:oak_fence", "minecraft:glass", 8), "true");
            // The connector's own part has the last word.
            assert_eq!(
                place("minecraft:birch_fence", "minecraft:lantern", 10),
                "false"
            );
        })
    });
}
