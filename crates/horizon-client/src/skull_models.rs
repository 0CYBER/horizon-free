//! The skull types native mods register (`create_skull_models`, NeoForge's
//! `EntityRenderersEvent.CreateSkullModels`): each kind's head model, baked once at start, and its
//! default texture (`SkullBlockRenderer.SKIN_BY_TYPE`). A `minecraft:head` item model names a kind.

use crate::{block_entity_render::ModelPart, mobs};
use mod_api::SkullModel;
use std::{collections::HashMap, path::Path, sync::OnceLock};

/// A registered kind's baked head and its default texture.
struct Skull {
    part: ModelPart,
    texture: String,
}

static REGISTERED: OnceLock<HashMap<String, Skull>> = OnceLock::new();

/// Keeps the skull types native mods registered, once at start, after the model layers. A kind
/// that is not `<namespace>:<path>` in a mod's namespace, or registered already, is left out by
/// `horizon_assets::item_model::register_skull_kinds`; one whose layer neither a mod nor vanilla
/// (`entity-models.json` under `vanilla_root`) has, or whose texture is empty, is logged and left
/// out here, before its kind is registered, so no item model can name it. Returns the kinds kept.
pub fn register(models: Vec<SkullModel>, vanilla_root: &Path) -> Vec<String> {
    if REGISTERED.get().is_some() {
        log::warn!("skull models were registered already; these are left out");
        return Vec::new();
    }
    let mut vanilla = None;
    let mut baked = Vec::new();
    for model in models {
        if model.texture.is_empty() {
            log::warn!("skull type {:?} has no texture", model.kind);
            continue;
        }
        let part = match mobs::model_layers::registered(&model.layer) {
            Some(definition) => Some(from_mod_part(&definition.bake_root())),
            None => vanilla
                .get_or_insert_with(|| mobs::EntityModelDocument::load(vanilla_root).ok())
                .as_ref()
                .and_then(|document| document.model(&model.layer))
                .map(|part| from_vanilla_part(&part)),
        };
        let Some(part) = part else {
            log::warn!(
                "skull type {:?} names model layer {:?}, which no mod registered and vanilla lacks",
                model.kind,
                model.layer
            );
            continue;
        };
        baked.push((model.kind, part, model.texture));
    }
    let kept = horizon_assets::item_model::register_skull_kinds(
        baked.iter().map(|(kind, ..)| kind.clone()).collect(),
    );
    // A kind registered twice keeps its first entry, as `register_skull_kinds` does.
    let mut table = HashMap::new();
    for (kind, part, texture) in baked {
        if kept.contains(&kind) {
            table.entry(kind).or_insert(Skull { part, texture });
        }
    }
    // Registration runs once, on the startup thread, so the check above holds.
    let _ = REGISTERED.set(table);
    kept
}

/// The head model of a skull type a native mod registered.
pub fn part(kind: &str) -> Option<&'static ModelPart> {
    REGISTERED.get()?.get(kind).map(|skull| &skull.part)
}

/// The default texture of a skull type a native mod registered.
pub fn texture(kind: &str) -> Option<&'static str> {
    REGISTERED
        .get()?
        .get(kind)
        .map(|skull| skull.texture.as_str())
}

/// Every registered kind with its head model and default texture, which the item renderer takes in
/// when it loads.
pub(crate) fn all() -> impl Iterator<Item = (&'static str, &'static ModelPart, &'static str)> {
    REGISTERED
        .get()
        .into_iter()
        .flatten()
        .map(|(kind, skull)| (kind.as_str(), &skull.part, skull.texture.as_str()))
}

/// A baked mod layer as the special renderers draw it: its pose, its faces in texels with their
/// texture coordinates, and its children in drawing order.
fn from_mod_part(part: &mod_api::ModelPart) -> ModelPart {
    ModelPart {
        pose: [
            part.x,
            part.y,
            part.z,
            part.x_rot,
            part.y_rot,
            part.z_rot,
            part.x_scale,
            part.y_scale,
            part.z_scale,
        ],
        polygons: part
            .polygons
            .iter()
            .map(|polygon| {
                polygon
                    .vertices
                    .map(|v| [v.pos[0], v.pos[1], v.pos[2], v.u, v.v])
            })
            .collect(),
        children: part
            .children()
            .map(|(name, child)| (name.to_owned(), from_mod_part(child)))
            .collect(),
    }
}

/// A vanilla layer from `entity-models.json`, as the special renderers draw it.
fn from_vanilla_part(part: &mobs::Part) -> ModelPart {
    ModelPart {
        pose: part.pose,
        polygons: part
            .polygons
            .iter()
            .map(|polygon| polygon.vertices)
            .collect(),
        children: part
            .children
            .iter()
            .map(|child| (child.name.clone(), from_vanilla_part(child)))
            .collect(),
    }
}
