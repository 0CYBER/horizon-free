//! Custom names: `NameTagItem.interactLivingEntity` and the name tag
//! (`EntityRenderer.submitNameTag`): above the head, facing the camera,
//! on a translucent black background, shown when the name is visible or the
//! mob is under the crosshair within 64 blocks, as its team's name tag
//! visibility allows, in the team's colours, with its score in the
//! objective displayed below names.
use super::{Game, view_vector};
use horizon_client::{mesher::Vertex, save::uuid_text, scoreboard as sb, text::Text};

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
    /// `Entity.belowNameDisplay`: the mob's score in the objective shown below
    /// names, then a space and the objective's name.
    fn below_name_text(&self, id: u32) -> Option<Text> {
        let board = &self.score.board;
        let objective = board.display_objective(sb::SLOT_BELOW_NAME)?;
        let uuid = self.persist.uuids.get(&id)?;
        let score = board.score(&uuid_text(*uuid), &objective.name)?;
        // numberFormatOrDefault(StyledFormat.NO_STYLE).
        let format = score
            .number_format
            .clone()
            .or_else(|| objective.number_format.clone())
            .unwrap_or(sb::NumberFormat::Styled(serde_json::json!({})));
        let mut t = Text::empty();
        t.push(format.format(score.value));
        t.push(Text::literal(" "));
        t.push(objective.display_text());
        Some(t)
    }
    pub(super) fn name_tag_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        let target = self.mob_hit().map(|(i, _)| self.mobs[i].id);
        let forward = glam::DVec3::from_array(view_vector(self.yaw, self.pitch));
        let right = forward.cross(glam::DVec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        let white = self.res.atlas.font.white();
        let viewer = self.player_team();
        for m in &self.mobs {
            let Some(name) = m.custom_name.as_deref() else {
                continue;
            };
            if !m.alive() {
                continue;
            }
            let p = glam::DVec3::new(
                m.previous.x + (m.position.x - m.previous.x) * alpha,
                m.previous.y + (m.position.y - m.previous.y) * alpha + f64::from(m.height()) + 0.5,
                m.previous.z + (m.position.z - m.previous.z) * alpha,
            );
            // The name tag distance (64) is measured to the entity.
            let distance_sqr =
                glam::DVec3::new(m.position.x, m.position.y, m.position.z).distance_squared(camera);
            if distance_sqr >= 64.0 * 64.0 {
                continue;
            }
            let team = self.mob_team(m.id);
            // LivingEntityRenderer.shouldShowName (team rules included), then
            // MobRenderer: a visible name, or the mob under the crosshair.
            let invisible = sb::invisible_to(
                team,
                viewer,
                self.spectator(),
                m.has_effect("minecraft:invisibility"),
            );
            let view = sb::NameView {
                discrete: false,
                distance_sqr,
                invisible_to_viewer: invisible,
                hud_hidden: self.ui.hide_gui,
                is_camera: false,
                is_vehicle: self.mobs.iter().any(|o| o.extra.mount == Some(m.id)),
            };
            if !sb::name_shown(team, viewer, view) || !(m.name_visible || Some(m.id) == target) {
                continue;
            }
            // getDisplayName: the custom name in the team's prefix, suffix and colour.
            let shown = match team {
                Some(t) => t.formatted_name(Text::literal(name)),
                None => Text::literal(name),
            };
            let mut lines = Vec::new();
            // EntityRenderer.extractNameTags: the score shows within the
            // below-name distance (10), under the name.
            let mut lift = 0.0;
            if distance_sqr < 10.0 * 10.0
                && let Some(score) = self.below_name_text(m.id)
            {
                lines.push((score, 0.0));
                lift = f64::from(9.0f32 * 1.15 * 0.025);
            }
            lines.push((shown, lift));
            let l = self.world.light(
                m.position.x.floor() as i32,
                (m.position.y + f64::from(m.height())).floor() as i32,
                m.position.z.floor() as i32,
            );
            let light = (u32::from(l >> 4) * 16) | ((u32::from(l & 15) * 16) << 8);
            let away = forward.as_vec3() * 0.002;
            for (text, lift) in lines {
                let o = (p + glam::DVec3::new(0.0, lift, 0.0) - camera).as_vec3();
                let (r, u) = (right.as_vec3() * 0.025, up.as_vec3() * 0.025);
                // Text space: x right, y down (scale 0.025, -0.025).
                let at = |x: f32, y: f32| o + r * x - u * y;
                let chars: Vec<(char, u32)> = text
                    .spans
                    .iter()
                    .flat_map(|s| {
                        let rgb = s.style.color.unwrap_or(0xFF_FF_FF);
                        // RGBA8, alpha in the top byte.
                        let color = 0xFF00_0000
                            | ((rgb & 0xFF) << 16)
                            | (rgb & 0xFF00)
                            | ((rgb >> 16) & 0xFF);
                        s.text.chars().map(move |c| (c, color))
                    })
                    .collect();
                let width: f32 = chars
                    .iter()
                    .map(|(c, _)| self.res.atlas.glyph(*c).map_or(6.0, |g| g.1))
                    .sum();
                let x0 = -(width / 2.0).trunc();
                let y0 = if text.plain() == "deadmau5" {
                    -10.0
                } else {
                    0.0
                };
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
                            normal: [0; 4],
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
                for (ch, color) in chars {
                    let Some((g, advance)) = self.res.atlas.glyph(ch) else {
                        x += 6.0;
                        continue;
                    };
                    if ch != ' ' {
                        quad(
                            [x, y0, x + 8.0, y0 + 8.0],
                            [g.u0, g.v0, g.u1, g.v1],
                            color,
                            false,
                        );
                    }
                    x += advance;
                }
            }
        }
    }
}
