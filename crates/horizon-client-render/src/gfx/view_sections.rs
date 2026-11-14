//! Each world view's own section list (#3773): the frame keeps the player camera's culling (the
//! caves, occlusion, faces turned away, GPU culling) while views are asked for, and each view
//! walks the resident columns once more with its own frustum, whose near plane is its clip plane
//! (`world_views::ViewCamera`). No occlusion graph and no faces left out: a portal's camera
//! usually sits inside a wall, behind its clip plane, and a mirrored view needs nothing more.
//! Its draws go after the frame's and the shadow camera's, as the shadow pass's go after the
//! frame's, and its translucent sections go back to front from its own camera.

use super::{
    Frame, Frustum, JoinedFrusta, Quads, Renderer, SectionKey, frustum, native_render, resident,
    world_views::ViewCamera,
};
use crate::render_views::{MAX_WORLD_VIEWS, WorldViewRefusalReason, WorldViews};
use glam::DVec3;

/// This frame's world views, asked once (`Renderer::camera_frustum`, or `render` itself when it
/// was not called), each one's sections and where its draws are in the frame's list. The lists
/// keep their room from frame to frame.
pub(super) struct Sections {
    /// The views asked for the frame being built, in the order they draw, and whether `render`
    /// has yet to draw them.
    views: WorldViews,
    asked: bool,
    /// Their frusta, relative to the camera they were asked for, joined to that camera's.
    seen: Option<JoinedFrusta>,
    /// Each view's sections, nearest its camera first, origins relative to the frame's camera.
    visible: [Vec<resident::Visible>; MAX_WORLD_VIEWS],
    /// Each view's draws in `Renderer::draws`: its solid ones from the first, its translucent
    /// ones from the second, to the third.
    pub draws: [[u32; 3]; MAX_WORLD_VIEWS],
}

impl Default for Sections {
    fn default() -> Self {
        Self {
            views: WorldViews::default(),
            asked: false,
            seen: None,
            visible: Default::default(),
            draws: [[0; 3]; MAX_WORLD_VIEWS],
        }
    }
}

impl Sections {
    /// The views mods ask for (`native_render::world_views`) that a device whose textures are at
    /// most `max_side` pixels a side draws, the views seen in a view before it; none, and no
    /// call, while no mod listens.
    fn ask(&mut self, max_side: u32) {
        self.views.begin(max_side);
        if native_render::world_views_hooked() {
            native_render::world_views(&mut self.views);
            self.views.order();
        }
    }
    /// This frame's views, for `draw_world_views`: put back with `put_back`.
    pub fn take_views(&mut self) -> WorldViews {
        std::mem::take(&mut self.views)
    }
    pub fn put_back(&mut self, views: WorldViews) {
        self.views = views;
    }
    /// Every view's sections, the first view's first.
    pub fn sections(&self) -> impl Iterator<Item = &resident::Visible> {
        self.visible.iter().flatten()
    }
    pub fn sections_mut(&mut self) -> impl Iterator<Item = &mut resident::Visible> {
        self.visible.iter_mut().flatten()
    }
    /// `Flat::prepare`'s marks: where the shadow camera's solid and translucent draws begin,
    /// then each view's solid and translucent ones (`usize::MAX` for a view not asked for).
    pub fn marks(&self, shadow: [u32; 2]) -> [usize; super::flat::MARKS] {
        let mut marks = [usize::MAX; super::flat::MARKS];
        marks[0] = shadow[0] as usize;
        marks[1] = shadow[1] as usize;
        let views = self.views.views().len();
        for (k, [solid, translucent, _]) in self.draws[..views].iter().enumerate() {
            marks[2 + 2 * k] = *solid as usize;
            marks[3 + 2 * k] = *translucent as usize;
        }
        marks
    }
    /// Each view's draws, appended to the frame's `draws` and their `pages` (`Sections::draws`):
    /// its sections' solid quads, every face, nearest first, then its translucent sections back
    /// to front from its camera. `instance` is the first view section's instance.
    pub fn push_draws(
        &mut self,
        resident: &resident::Resident<super::SectionGpu>,
        draws: &mut Vec<[u32; 5]>,
        pages: &mut Vec<(u32, Option<u32>)>,
        mut instance: u32,
    ) {
        for (visible, at) in self.visible.iter().zip(&mut self.draws) {
            let solid = draws.len() as u32;
            for (i, v) in visible.iter().enumerate() {
                let s = resident.get(v.slot);
                s.sides.every(Quads::All, |first, quads| {
                    let base = s.solid_base() + first * 4;
                    draws.push([quads * 6, 1, 0, base, instance + i as u32]);
                    pages.push((s.page, None));
                });
            }
            let translucent = draws.len() as u32;
            for (i, v) in visible.iter().enumerate().rev() {
                let s = resident.get(v.slot);
                if let Some((index_page, index_start)) = s.sorted_indices {
                    let (page, first) = s.translucent_at;
                    let i = instance + i as u32;
                    draws.push([s.translucent * 6, 1, index_start as u32, first as u32, i]);
                    pages.push((page, Some(index_page)));
                }
            }
            *at = [solid, translucent, draws.len() as u32];
            instance += visible.len() as u32;
        }
    }
}

/// The sections of `sections` that `view` sees within `radius` chunks of its own camera, with its
/// own frustum and nothing more, nearest its camera first, into `out`; their origins relative to
/// the frame's camera, `frame_camera`. `far`: the frame's far plane.
fn view_sections<T>(
    sections: &resident::Resident<T>,
    view: &native_render::WorldView,
    frame_camera: DVec3,
    far: f32,
    radius: i32,
    out: &mut Vec<resident::Visible>,
) {
    let own = Frustum {
        planes: frustum(ViewCamera::new(view, view.camera, far).own_view_proj),
    };
    sections.visible(view.camera, radius, &own, |_| true, |_, _| false, out);
    for v in out.iter_mut() {
        v.origin = resident::origin(v.key, sections.min_y(), frame_camera)
            .as_vec3()
            .to_array();
    }
}

impl Renderer {
    /// The last frame's world views' section draws, all of them together (its own:
    /// `section_draws`).
    pub fn world_view_draws(&self) -> usize {
        let draws = &self.world_views.sections.draws;
        draws
            .iter()
            .map(|[solid, _, end]| (end - solid) as usize)
            .sum()
    }
    /// The player's own level this frame, as the game resolved it (`Game::client_own_level`),
    /// which a view naming a level is compared with: set before the views are asked for.
    pub fn set_world_view_own_level(&mut self, own_level: Option<mod_api::LevelRef>) {
        self.world_views.sections.views.set_own_level(own_level);
    }
    /// The frusta `render` will cull with for `camera`: the camera's `frustum` and those of the
    /// world views, which it asks the mods for here, once a frame (`render` draws these).
    pub(super) fn ask_world_views(
        &mut self,
        camera: DVec3,
        far: f32,
        frustum: Frustum,
    ) -> JoinedFrusta {
        let max_side = self.world_view_side;
        let sections = &mut self.world_views.sections;
        sections.ask(max_side);
        sections.asked = true;
        let views = sections.views.views().iter().map(|v| Frustum {
            planes: super::frustum(ViewCamera::new(v, camera, far).view_proj),
        });
        let seen = JoinedFrusta::new(frustum, views);
        sections.seen = Some(seen);
        seen
    }
    /// The last `ask_world_views`' views with the camera's frustum `frustum`.
    pub(super) fn asked_world_views(&self, frustum: Frustum) -> JoinedFrusta {
        let seen = self.world_views.sections.seen;
        seen.map_or(JoinedFrusta::new(frustum, []), |s| s.with_camera(frustum))
    }

    /// This frame's views (those `camera_frustum` asked for it, else asked now; on the title
    /// or a loading screen none, those asked refused) and each one's sections, a walk over the
    /// resident columns each within `radius` chunks of its camera; those the camera has not
    /// sorted are sorted now.
    pub(super) fn walk_world_views(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &Frame,
        radius: i32,
    ) {
        let sections = &mut self.world_views.sections;
        if frame.title && sections.asked {
            sections
                .views
                .refuse_all(WorldViewRefusalReason::TitleScreen);
        } else if frame.title {
            sections.views.begin(self.world_view_side);
        } else if !sections.asked {
            sections.ask(self.world_view_side);
        }
        sections.asked = false;
        let views = sections.views.views();
        for (k, out) in sections.visible.iter_mut().enumerate() {
            out.clear();
            if let Some(view) = views.get(k) {
                let (camera, far) = (frame.camera, frame.depth_far);
                view_sections(&self.sections, view, camera, far, radius, out);
            }
        }
        let visible = std::mem::take(&mut self.world_views.sections.visible);
        for v in visible.iter().flatten() {
            self.sort_unseen(encoder, v.slot, v.key, frame.camera);
        }
        self.world_views.sections.visible = visible;
    }

    /// Sorts the translucent quads of the section in `slot` at `key` for `camera` if they never
    /// were: one the camera has not seen, which a shadow camera or a world view draws, in their
    /// last sorted order otherwise.
    pub(super) fn sort_unseen(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        slot: u32,
        key: SectionKey,
        camera: DVec3,
    ) {
        let s = self.sections.get_mut(slot);
        let Some((index_page, index_start)) = s.sorted_indices.filter(|_| !s.sort.is_sorted())
        else {
            return;
        };
        let origin = [key.0 * 16, self.layout.min_y() + key.1 * 16, key.2 * 16];
        if let Some(indices) =
            s.sort
                .update(camera.to_array(), origin, &mut self.translucent_sorter)
        {
            self.uploads.buffer(
                encoder,
                self.sorted_indices.buffer(index_page),
                index_start * 4,
                bytemuck::cast_slice(indices),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A camera at z 60 looking north (-z) along x 8, y 104 (section y 10).
    fn north() -> native_render::WorldView {
        let at = DVec3::new(8.0, 104.0, 60.0);
        native_render::WorldView::new(1, at, 180.0, 0.0, 70.0, [200, 120])
    }
    /// What `view` lists of `keys`, the frame's camera at `frame_camera`.
    fn walk(
        view: native_render::WorldView,
        keys: &[SectionKey],
        frame_camera: DVec3,
    ) -> Vec<resident::Visible> {
        let mut sections = resident::Resident::default();
        for &key in keys {
            sections.insert(key, ());
        }
        let mut out = Vec::new();
        view_sections(&sections, &view, frame_camera, 1000.0, 8, &mut out);
        out
    }

    /// A view's list holds only what its own frustum takes in: of sections ahead at z 32..48 and
    /// 16..32, one behind its camera (z 64..80) and one off to its side, the two ahead; with a
    /// clip plane at z 30 facing away from it, not the one wholly behind the plane (z 32..48).
    #[test]
    fn a_view_lists_the_sections_its_own_frustum_sees() {
        let keys = [(0, 10, 2), (0, 10, 1), (0, 10, 4), (-4, 10, 3)];
        let listed = |view| -> Vec<SectionKey> {
            walk(view, &keys, DVec3::ZERO)
                .iter()
                .map(|v| v.key)
                .collect()
        };
        assert_eq!(listed(north()), [(0, 10, 2), (0, 10, 1)]);
        let clipped = north().with_clip_plane([0.0, 0.0, -1.0, 30.0]);
        assert_eq!(listed(clipped), [(0, 10, 1)]);
    }

    /// A view's sections go nearest its own camera first (its translucent draws back to front
    /// from it), not the frame camera's, and their origins are relative to the frame's camera.
    #[test]
    fn a_views_sections_go_by_its_own_camera() {
        let frame_camera = DVec3::new(8.0, 104.0, -100.0);
        let out = walk(north(), &[(0, 10, 1), (0, 10, 2)], frame_camera);
        let keys: Vec<SectionKey> = out.iter().map(|v| v.key).collect();
        assert_eq!(keys, [(0, 10, 2), (0, 10, 1)]);
        assert_eq!(out[0].origin, [-8.0, -8.0, 132.0]);
    }
}
