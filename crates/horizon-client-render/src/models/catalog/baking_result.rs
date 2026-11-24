//! Adapt the staged block and standalone catalog to the native mod bake hook.
use super::{BakedFace, BakedModel, Direction, QuadShape, StateModels, Variant, face_transparency};
use crate::sprites::Sprites;
use horizon_world::registry::BlockRegistry;
use mod_api::{
    __private::{
        ModPhase, PhaseOwner,
        built::{BakedModelFaces, BakedModelsAccess, baked_model_quad},
        game::RegistrationContextNew,
    },
    BakedModelEditError, BakedModelKey, BakedModelQuad, BakedModelView, BakedModels,
    BakedQuadChange, Direction as ApiDirection, HookId, RegistrationContext,
};

/// The client-only hook target. Its catalog's borrow ends before the candidate can publish.
#[inline(never)]
pub fn native_modify_baking_result(context: &RegistrationContext, models: &mut BakedModels<'_>) {
    mod_api::dispatch_modify_baking_result(context, models);
}

pub(in crate::models) fn modify(
    catalog: &mut StateModels,
    registry: &BlockRegistry,
    sprites: &Sprites,
) {
    if !mod_api::__private::listened(HookId::ModifyBakingResult) {
        return;
    }
    let _phase = PhaseOwner::scope(ModPhase::Registration);
    let mut candidate = Candidate {
        catalog,
        registry,
        sprites,
    };
    native_modify_baking_result(
        &RegistrationContext::new(),
        &mut BakedModels::new(&mut candidate),
    );
}

struct Candidate<'a> {
    catalog: &'a mut StateModels,
    registry: &'a BlockRegistry,
    sprites: &'a Sprites,
}

impl Candidate<'_> {
    fn slot(&self, key: BakedModelKey<'_>) -> Option<usize> {
        match key {
            BakedModelKey::Standalone(id) => self.catalog.standalone_slot(id).map(|i| i as usize),
            variant => self
                .catalog
                .baked
                .iter()
                .position(|(v, _)| variant_key(v) == variant),
        }
    }
}

fn variant_key(variant: &Variant) -> BakedModelKey<'_> {
    BakedModelKey::Variant {
        model: &variant.model,
        rotations: variant.rotations,
        uv_lock: variant.uv_lock,
    }
}

impl BakedModelsAccess for Candidate<'_> {
    fn model(&self, key: BakedModelKey<'_>) -> Option<BakedModelView<'_>> {
        let slot = self.slot(key)?;
        Some(BakedModelView::new(&self.catalog.baked[slot].1))
    }

    fn visit_block_state<'a>(
        &'a self,
        block: &str,
        properties: &[(&str, &str)],
        visit: &mut dyn FnMut(BakedModelKey<'a>, BakedModelView<'a>),
    ) -> bool {
        let Some((choice, selected)) = self
            .registry
            .id_by_properties(block, properties)
            .and_then(|state| self.catalog.state(state))
        else {
            return false;
        };
        let catalog = &*self.catalog;
        catalog.choices[*choice].for_each_alternative(selected, |&slot| {
            let (variant, model) = &catalog.baked[slot as usize];
            visit(variant_key(variant), BakedModelView::new(model));
        });
        true
    }

    fn edit_quads(
        &mut self,
        key: BakedModelKey<'_>,
        edit: &mut dyn FnMut(BakedModelQuad<'_>) -> BakedQuadChange,
    ) -> Result<(), BakedModelEditError> {
        let slot = self.slot(key).ok_or(BakedModelEditError::UnknownModel)?;
        let model = &mut self.catalog.baked[slot].1;
        // Validate all changes before mutating the candidate, storing only changed quads.
        let mut changes = Vec::new();
        for (index, face) in model.faces.iter().enumerate() {
            match edit(quad_view(face)) {
                BakedQuadChange::Keep => (),
                BakedQuadChange::Remove => changes.push((index, Change::Remove)),
                BakedQuadChange::Replace {
                    positions,
                    uvs,
                    cull_face,
                } => {
                    let cull = cull_face.map(model_direction);
                    let quad = &face.quad;
                    if positions == quad.positions && uvs == quad.uv && cull == quad.cull {
                        continue;
                    }
                    // UVs pack into `[0, 2)` fixed point (`mesher::baked_uv`): reject what would clamp.
                    if !positions.iter().flatten().all(|v| v.is_finite())
                        || !uvs.iter().flatten().all(|v| (0.0..2.0).contains(v))
                    {
                        return Err(BakedModelEditError::InvalidGeometry);
                    }
                    let facing = if positions == quad.positions {
                        quad.facing
                    } else {
                        horizon_assets::bake::facing(&positions)
                            .ok_or(BakedModelEditError::InvalidGeometry)?
                    };
                    changes.push((
                        index,
                        Change::Replace {
                            positions,
                            uvs,
                            cull,
                            facing,
                        },
                    ));
                }
                _ => unreachable!("the bake adapter handles all supported quad edits"),
            }
        }
        if changes.is_empty() {
            return Ok(());
        }
        let mut changes = changes.into_iter().peekable();
        let mut index = 0;
        model.faces.retain_mut(|face| {
            let at = index;
            index += 1;
            let Some((_, change)) = changes.next_if(|(target, _)| *target == at) else {
                return true;
            };
            let Change::Replace {
                positions,
                uvs,
                cull,
                facing,
            } = change
            else {
                return false;
            };
            let quad = &mut face.quad;
            if quad.positions != positions {
                quad.positions = positions;
                quad.facing = facing;
                face.shapes = [false, true].map(|full| QuadShape::new(&positions, facing, full));
            }
            if quad.uv != uvs {
                quad.uv = uvs;
                face.uv_fixed = crate::mesher::baked_uv(uvs);
                (face.translucent, face.opaque) = face_transparency(self.sprites, quad, face.layer);
            }
            quad.cull = cull;
            true
        });
        model.cube = crate::models::cube::cube_styles(&model.faces);
        self.catalog.bounds.take();
        Ok(())
    }
}

enum Change {
    Remove,
    Replace {
        positions: [[f32; 3]; 4],
        uvs: [[f32; 2]; 4],
        cull: Option<Direction>,
        facing: Direction,
    },
}

impl BakedModelFaces for BakedModel {
    fn ambient_occlusion(&self) -> bool {
        self.ambient_occlusion
    }

    fn quad_count(&self) -> usize {
        self.faces.len()
    }

    fn quad(&self, index: usize) -> BakedModelQuad<'_> {
        quad_view(&self.faces[index])
    }
}

fn quad_view(face: &BakedFace) -> BakedModelQuad<'_> {
    let quad = &face.quad;
    baked_model_quad(
        &quad.positions,
        &quad.uv,
        &quad.material.sprite,
        api_direction(quad.facing),
        quad.cull.map(api_direction),
        (quad.tint_index >= 0).then_some(quad.tint_index),
        quad.shade,
        quad.shade_direction_override.map(api_direction),
        quad.ambient_occlusion,
        quad.color,
        quad.light,
    )
}

fn api_direction(direction: Direction) -> ApiDirection {
    match direction {
        Direction::Down => ApiDirection::Down,
        Direction::Up => ApiDirection::Up,
        Direction::North => ApiDirection::North,
        Direction::South => ApiDirection::South,
        Direction::West => ApiDirection::West,
        Direction::East => ApiDirection::East,
    }
}

fn model_direction(direction: ApiDirection) -> Direction {
    match direction {
        ApiDirection::Down => Direction::Down,
        ApiDirection::Up => Direction::Up,
        ApiDirection::North => Direction::North,
        ApiDirection::South => Direction::South,
        ApiDirection::West => Direction::West,
        ApiDirection::East => Direction::East,
    }
}
