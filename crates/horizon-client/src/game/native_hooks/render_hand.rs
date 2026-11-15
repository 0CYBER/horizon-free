use super::*;
use crate::game::Dim;
use horizon_client::mesher::Vertex;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Before one first-person hand of this window's player is drawn
    /// (`RenderHandEvent`, client): a hand that holds an item or a map, the
    /// empty main hand's arm, and the empty off hand, where vanilla draws
    /// nothing. Adjust `render`'s pose, draw in the hand with `draw`, or
    /// cancel the default geometry; what the mods drew stays. Equip progress
    /// includes the item's swap scale. `draw`'s pose stack starts in the
    /// hands' view space, in blocks, +x right, +y up, -z into the screen,
    /// under the hand sway and before vanilla's per-item transforms; both
    /// hands share it, and `render.hand` says which one is asked. It is lit
    /// with the light at the eyes; add geometry with `native_draw_cube`,
    /// `native_draw_quad`, `native_draw_block` and `native_draw_item`. It
    /// joins the hand's own vertices, ahead of vanilla's: the hand's field of
    /// view, the view bob, a shader pack's hand pass and `native_hand_drawn`
    /// see it as they see vanilla's hand. Unlike NeoForge, the pose stack is
    /// `draw`'s own: turning it moves what you draw, `render`'s pose moves
    /// the default geometry, and player render state is not exposed.
    pub(in crate::game) fn native_render_hand(
        &self,
        render: &mut HandRender<'_>,
        draw: &mut BlockEntityDraw,
        canceled: &mut bool,
    ) {
        self.fire_render_hand(render, draw, canceled);
    }
}

impl Game<'_> {
    /// Asks the mods about one first-person hand (`native_render_hand`) and
    /// says whether they left its default geometry out. They draw in the
    /// hands' view space `to_world`; what they drew is appended to the hand's
    /// vertices `out`, cancelled or not, in a run of its own with no rendered
    /// item id, as the arm's.
    pub(in crate::game) fn render_hand_cancelled(
        &self,
        render: &mut HandRender<'_>,
        to_world: glam::Mat4,
        light: u32,
        out: &mut Vec<Vertex>,
        runs: &mut Vec<(u32, i32)>,
    ) -> bool {
        let start = out.len();
        let eye = self.eye_position().unwrap_or_default();
        let mut draw = BlockEntityDraw::posed(
            render.partial,
            self.world.game_time,
            light,
            eye.map(|c| c.floor() as i32),
            to_world,
            horizon_client::block_entity_render::diffuse_lights(self.dim == Dim::Nether),
            std::mem::take(out),
        )
        .animated(self.animation_time());
        let mut canceled = false;
        self.native_render_hand(render, &mut draw, &mut canceled);
        *out = draw.into_vertices();
        if out.len() > start {
            runs.push((start as u32, 0));
        }
        canceled
    }
}
