//! Custom names: `NameTagItem.interactLivingEntity` and the name tag
//! (`EntityRenderer.submitNameTag`): above the head, facing the camera,
//! on a translucent black background, shown when the name is visible or the
//! mob is under the crosshair within 64 blocks.
use super::{Game, view_vector};
use horizon_client::mesher::Vertex;

impl Game<'_> {
    /// A named name tag names the mob under the crosshair.
    pub(super) fn name_mob(&mut self, index: usize) -> bool {
        let Some(name) = self
            .held()
            .filter(|h| &*h.item == "minecraft:name_tag")
            .and_then(|h| h.custom_name().map(str::to_owned))
        else {
            return false;
        };
        let m = &mut self.mobs[index];
        if !m.alive() {
            return false;
        }
        m.custom_name = Some(name);
        m.persistent = true;
        // itemStack.shrink(1), in creative too.
        self.consume_held();
        self.swing_arm();
        true
    }
    pub(super) fn name_tag_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        let target = self.mob_hit().map(|(i, _)| self.mobs[i].id);
        let forward = glam::DVec3::from_array(view_vector(self.yaw, self.pitch));
        let right = forward.cross(glam::DVec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        let white = self.res.atlas.font.white();
        for m in &self.mobs {
            let Some(name) = m.custom_name.as_deref() else {
                continue;
            };
            if !(m.name_visible || Some(m.id) == target) || !m.alive() {
                continue;
            }
            let p = glam::DVec3::new(
                m.previous.x + (m.position.x - m.previous.x) * alpha,
                m.previous.y + (m.position.y - m.previous.y) * alpha + f64::from(m.height()) + 0.5,
                m.previous.z + (m.position.z - m.previous.z) * alpha,
            );
            if p.distance_squared(camera) >= 64.0 * 64.0 {
                continue;
            }
            let o = (p - camera).as_vec3();
            let (r, u) = (right.as_vec3() * 0.025, up.as_vec3() * 0.025);
            // Text space: x right, y down (scale 0.025, -0.025).
            let at = |x: f32, y: f32| o + r * x - u * y;
            let width: f32 = name
                .chars()
                .map(|c| self.res.atlas.glyph(c).map_or(6.0, |g| g.1))
                .sum();
            let x0 = -(width / 2.0).trunc();
            let y0 = if name == "deadmau5" { -10.0 } else { 0.0 };
            let l = self.world.light(
                m.position.x.floor() as i32,
                (m.position.y + f64::from(m.height())).floor() as i32,
                m.position.z.floor() as i32,
            );
            let light = (u32::from(l >> 4) * 16) | ((u32::from(l & 15) * 16) << 8);
            let away = forward.as_vec3() * 0.002;
            // `background`: unlit (vertex colour only) and a hair behind the text.
            let mut quad = |a: [f32; 4], uv: [f32; 4], color: u32, background: bool| {
                let corners = [(a[0], a[3]), (a[2], a[3]), (a[2], a[1]), (a[0], a[1])];
                let uvs = [
                    [uv[0], uv[3]],
                    [uv[2], uv[3]],
                    [uv[2], uv[1]],
                    [uv[0], uv[1]],
                ];
                for ((x, y), uv) in corners.into_iter().zip(uvs) {
                    out.push(Vertex {
                        pos: (at(x, y) + if background { away } else { glam::Vec3::ZERO })
                            .to_array(),
                        uv,
                        layer: if background { 2 } else { 0 },
                        color,
                        light,
                    });
                }
            };
            // Background: Options.getBackgroundOpacity(0.25) black.
            quad(
                [x0 - 1.0, y0 - 1.0, x0 + width, y0 + 8.0],
                [white.u0, white.v0, white.u1, white.v1],
                0x40_00_00_00,
                true,
            );
            let mut x = x0;
            for ch in name.chars() {
                let Some((g, advance)) = self.res.atlas.glyph(ch) else {
                    x += 6.0;
                    continue;
                };
                if ch != ' ' {
                    quad(
                        [x, y0, x + 8.0, y0 + 8.0],
                        [g.u0, g.v0, g.u1, g.v1],
                        0xFFFF_FFFF,
                        false,
                    );
                }
                x += advance;
            }
        }
    }
}
