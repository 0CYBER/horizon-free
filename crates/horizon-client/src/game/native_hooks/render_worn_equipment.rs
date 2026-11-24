use super::*;

impl Game<'_> {
    /// What a living entity wears and holds, as it is drawn this frame: once a frame for each mob
    /// and player in view, right after its uncanceled `render_living_pre`, with its render state
    /// and `equipment`, everything shown and untinted, in which mods hide a slot's base, glint or
    /// trim and tint its colour (armour, body, elytra, both hands).
    pub(in crate::game) fn native_render_worn_equipment(
        &self,
        entity: &MobRender<'_>,
        equipment: &mut mod_api::EquipmentRender,
    ) {
        self.fire_render_worn_equipment(entity, equipment);
    }
}
