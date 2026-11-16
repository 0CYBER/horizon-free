//! Shape queries for a supplied state without replacing the block in the world.
use super::{Game, perspective};
use horizon_client::{
    block_entity_anim::BlockEntityAnims, collisions::SceneCollisions, world::World,
};
use horizon_core::position::BlockPos;
use horizon_world::{collision_context::CollisionContext, registry::BlockStateId};
use mod_api::BlockShapeKind;

impl Game<'_> {
    /// The same outline traversal without allocating a box list.
    pub(super) fn outline_boxes_iter(
        &self,
        state: BlockStateId,
        anims: &horizon_client::block_entity_anim::BlockEntityAnims,
        pos: [i32; 3],
        player_context: bool,
    ) -> impl Iterator<Item = [f64; 6]> + '_ {
        let declared = self.collision.cached_shapes().context_outline_shape(
            state,
            if player_context {
                self.outline_context_kind()
            } else {
                horizon_world::collision_context::EntityContextKind::Other
            },
        );
        let context = declared
            .is_none()
            .then(|| self.context_outline(pos, state, player_context, anims))
            .flatten();
        let f = self.res.facts().get(state);
        let o = perspective::outline_offset(
            &self.pack.context.tables,
            state,
            pos[0],
            pos[2],
            f.outline_offset,
        );
        let base = [
            f64::from(pos[0]) + o[0],
            f64::from(pos[1]) + o[1],
            f64::from(pos[2]) + o[2],
        ];
        let boxes = if context.is_some() || declared.is_some() {
            &[][..]
        } else {
            &f.outline[..]
        };
        context
            .map(|b| std::array::from_fn(|i| f64::from(pos[i % 3]) + b[i]))
            .into_iter()
            .chain(
                declared
                    .into_iter()
                    .flat_map(|shape| shape.boxes())
                    .map(move |b| {
                        let min = b.min_position();
                        let max = b.max_position();
                        let coords = [min.x, min.y, min.z, max.x, max.y, max.z];
                        std::array::from_fn(|i| base[i % 3] + coords[i])
                    }),
            )
            .chain(
                boxes
                    .iter()
                    .map(move |b| std::array::from_fn(|i| base[i % 3] + b[i])),
            )
    }

    pub(super) fn state_shape_boxes(
        &self,
        world: &World,
        anims: &BlockEntityAnims,
        state: BlockStateId,
        pos: [i32; 3],
        kind: BlockShapeKind,
    ) -> Vec<[f64; 6]> {
        match kind {
            BlockShapeKind::Outline => self
                .outline_boxes_iter(state, anims, pos, false)
                .map(|bounds| std::array::from_fn(|i| bounds[i] - f64::from(pos[i % 3])))
                .collect(),
            BlockShapeKind::Collision => {
                let mut scene = SceneCollisions {
                    world,
                    binding: self.collision,
                    subject: None,
                };
                self.collision
                    .collision_shape_with_context(
                        &mut scene,
                        state,
                        BlockPos::new(pos[0], pos[1], pos[2]),
                        &CollisionContext::empty(false),
                    )
                    .map(|shape| {
                        shape
                            .boxes()
                            .iter()
                            .map(|bounds| {
                                let min = bounds.min_position();
                                let max = bounds.max_position();
                                [min.x, min.y, min.z, max.x, max.y, max.z]
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            }
            _ => unreachable!("BlockShapeKind: unhandled variant"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::api_values::TryIntoApi;

    #[test]
    #[ignore = "daily"]
    fn supplied_shape_state_does_not_replace_world_state() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            let pos = [0, 64, 0];
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let slab = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone_slab")
                .unwrap();
            let before = game.world.block(pos[0], pos[1], pos[2]);
            assert_ne!(before, stone);
            for kind in [BlockShapeKind::Outline, BlockShapeKind::Collision] {
                assert_eq!(
                    level.block_shape(game, stone.try_into_api().unwrap(), pos, kind),
                    Some(vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]])
                );
                assert_eq!(
                    level.block_shape(game, slab.try_into_api().unwrap(), pos, kind),
                    Some(vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]])
                );
                assert_eq!(
                    level.block_shape(game, before.try_into_api().unwrap(), pos, kind),
                    Some(Vec::new())
                );
            }
            assert_eq!(game.world.block(pos[0], pos[1], pos[2]), before);
        });
    }
}
