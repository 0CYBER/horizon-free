//! The game owns mod callback dispatch; the world owns the placement seam.
use horizon_core::nbt::{Compound, JavaString, Tag};
use horizon_world::{
    registry::BlockRegistry,
    worldgen::structure::{Mirror, Rotation},
};
use std::{collections::HashMap, sync::Arc};
type Steps = HashMap<JavaString, Arc<dyn mod_api::BlockEntityTransform>>;
struct Adapter(Steps);
impl horizon_world::worldgen::structure::block_entity_transform::BlockEntityTransform for Adapter {
    fn transform(
        &self,
        position: [i32; 3],
        data: &mut Compound,
        rotation: Rotation,
        mirror: Mirror,
    ) {
        apply(&self.0, position, data, rotation, mirror);
    }
}
/// Freeze the callbacks with the registry; unused placement performs one OnceLock lookup.
pub(super) fn prepare(registry: &BlockRegistry) {
    if registry.structure_block_entity_transform.get().is_some() {
        return;
    }
    let steps: Steps = mod_api::__private::block_entity_transforms()
        .into_iter()
        .map(|(id, step)| (JavaString::from(id.as_str()), step))
        .collect();
    if !steps.is_empty() {
        let _ = registry
            .structure_block_entity_transform
            .set(Arc::new(Adapter(steps)));
    }
}

fn apply(
    steps: &Steps,
    position: [i32; 3],
    data: &mut Compound,
    rotation: Rotation,
    mirror: Mirror,
) {
    let Some(Tag::String(id)) = data.get(&"id".into()) else {
        return;
    };
    let Some(step) = steps.get(id) else {
        return;
    };
    let id = id.to_string_lossy();
    let transform = mod_api::StructureTransform::new(
        match rotation {
            Rotation::None => mod_api::StructureRotation::None,
            Rotation::Clockwise90 => mod_api::StructureRotation::Clockwise90,
            Rotation::Clockwise180 => mod_api::StructureRotation::Clockwise180,
            Rotation::Counterclockwise90 => mod_api::StructureRotation::Counterclockwise90,
        },
        match mirror {
            Mirror::None => mod_api::StructureMirror::None,
            Mirror::LeftRight => mod_api::StructureMirror::LeftRight,
            Mirror::FrontBack => mod_api::StructureMirror::FrontBack,
        },
    );
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        step.transform(data, transform)
    })) {
        Ok(Ok(mut result)) => {
            // A transform owns payload fields, never the saved entity's identity.
            for key in ["id", "x", "y", "z"] {
                let key = JavaString::from(key);
                if let Some(original) = data.get(&key) {
                    result.insert(key, original.clone());
                }
            }
            let tag = Tag::Compound(result);
            match horizon_core::nbt::write_network(&tag, horizon_core::nbt::Limits::default()) {
                Ok(_) => {
                    let Tag::Compound(result) = tag else {
                        unreachable!()
                    };
                    *data = result;
                }
                Err(error) => log::warn!(
                    "block entity {id} at {position:?} refused structure transform output: {error}"
                ),
            }
        }
        Ok(Err(error)) => {
            log::warn!("block entity {id} at {position:?} refused structure transform: {error}")
        }
        Err(_) => log::warn!(
            "block entity {id} at {position:?} panicked during structure transform; keeping original data"
        ),
    }
}
