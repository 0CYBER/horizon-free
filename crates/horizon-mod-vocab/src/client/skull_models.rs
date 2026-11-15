//! Skull models a native mod registers for its own skull types (NeoForge's
//! `EntityRenderersEvent.CreateSkullModels`).

/// A skull type a native mod registers once at start: the `kind` a `minecraft:head` item model
/// names, the model layer it is drawn with, and its default texture (`SKIN_BY_TYPE`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
// exhaustive: a mod builds it with a literal, as `registerSkullModel` takes a type and its model
#[derive(horizon_mod_api_macros::Wire)]
pub struct SkullModel {
    /// The skull type, `<namespace>:<path>` in a namespace other than vanilla's, like
    /// `mymod:goblin`: the `kind` of a `minecraft:head` item model.
    pub kind: String,
    /// The model layer the head is baked from: one the mod registered in `layer_definitions`
    /// (`mymod:goblin_head#main`) or vanilla's (`minecraft:zombie_head#main`). Its root's `head`
    /// part is posed as vanilla's skull heads are.
    pub layer: String,
    /// The entity texture drawn when the item model names none, as `BlockEntityDrawContext::
    /// model_part` takes a sprite (`entity/zombie/zombie`, a mod's `mymod:entity/goblin`).
    pub texture: String,
}
