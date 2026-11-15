//! `ThrownItemRenderer`: a thrown potion, experience bottle or content mod's thrown type drawn as
//! its item, facing the camera; or, for a thrown type a mod registered a renderer for
//! (`register_entity_renderers`), drawn by that renderer instead.
use super::Game;
use horizon_assets::item_model::DisplayContext;
use horizon_client::{
    mesher::Vertex,
    projectiles::{Projectile, ProjectileKind},
};

impl Game<'_> {
    /// Draws the thrown item projectile `a` at `at` (the frame's position; `c` the same relative
    /// to the camera), lit by `packed` as the entity pass packs it: true when it was one and is
    /// drawn.
    pub(super) fn thrown_item_geometry(
        &self,
        a: &Projectile,
        alpha: f64,
        at: [f64; 3],
        c: glam::Vec3,
        packed: u32,
        out: &mut Vec<Vertex>,
    ) -> bool {
        if let ProjectileKind::Thrown(t) = &a.kind
            && let Some(resolved) = super::native_entity_renderers::thrown_renderer(t)
        {
            let partial = alpha as f32;
            let entity: mod_api::EntityRenderState = mod_api::__private::built::EntityRenderState {
                id: a.id,
                partial,
                age: a.age as f32 + partial,
                yaw: a.previous_yaw + partial * (a.yaw - a.previous_yaw),
                pitch: a.previous_pitch + partial * (a.pitch - a.previous_pitch),
                living: None,
            }
            .into();
            let block = at.map(|x| x.floor() as i32);
            if self.mod_entity_renderer_geometry(
                &resolved,
                &entity,
                c.as_dvec3(),
                block,
                packed,
                out,
            ) {
                return true;
            }
        }
        let Some(stack) = a.drawn_item() else {
            return false;
        };
        let owner = super::item_draw::owner_at(at, 0.0, a.kind.id());
        let state = self.resolve_item(&stack, DisplayContext::Ground, Some(&owner), 0);
        let pose =
            glam::Mat4::from_translation(c) * glam::Mat4::from_quat(self.camera_orientation());
        self.submit_item_world(&state, pose, packed, out);
        true
    }
}
