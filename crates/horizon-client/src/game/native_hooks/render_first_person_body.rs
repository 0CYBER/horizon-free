use super::*;

impl Game<'_> {
    /// Decide whether to admit the local player's body to the world entity pass.
    pub(in crate::game) fn native_render_first_person_body(
        &self,
        player: &mod_api::EntityRenderState,
        riding: bool,
        body: &mut mod_api::FirstPersonBody,
    ) {
        self.fire_render_first_person_body(player, riding, body);
    }
}
