use super::Game;
use horizon_client::{dynamic_light, gizmos};

/// Non-particle requests survive a late particle clear. Select the combined nearest
/// lights only after particle extraction, so removed particles cannot displace them.
#[derive(Default)]
pub(super) struct FrameVisuals {
    item_lights: Vec<mod_api::ColoredLight>,
    mod_lights: Vec<mod_api::ColoredLight>,
}

impl FrameVisuals {
    pub(super) fn finish(
        self,
        game: &Game<'_>,
        camera: glam::DVec3,
    ) -> Vec<dynamic_light::Colored> {
        let particle_lights = game
            .renderer
            .as_ref()
            .and_then(|r| r.particle_lights())
            .map_or_else(Vec::new, |entries| {
                game.particles
                    .lights(entries, game.clock.camera_fraction() as f32)
            });
        dynamic_light::select_colored(
            self.mod_lights
                .into_iter()
                .map(|l| (l.at, l.rgb, l.radius, l.intensity))
                .chain(particle_lights)
                .chain(
                    self.item_lights
                        .into_iter()
                        .map(|l| (l.at, l.rgb, l.radius, l.intensity)),
                ),
            camera,
        )
    }
}

fn rim_target_valid(
    loaded: bool,
    state: horizon_world::registry::BlockStateId,
    air: horizon_world::registry::BlockStateId,
) -> bool {
    loaded && state != air
}

impl Game<'_> {
    /// Frame-local requests never persist after a mod is removed or a world changes.
    pub(super) fn native_frame_visuals(
        &self,
        alpha: f64,
        lines: &mut Vec<gizmos::Line>,
    ) -> FrameVisuals {
        let mut visuals = FrameVisuals::default();
        // Held and dropped items whose block the pack's `[[lights]]` colour shine in that colour,
        // as far as their dynamic light reaches.
        if let Some(renderer) = self.renderer.as_ref().filter(|r| r.has_light_colors()) {
            for (source, item) in self.dynamic_light_sources(alpha) {
                let Some(color) = item.and_then(|i| renderer.light_color(i)) else {
                    continue;
                };
                let top = color[0].max(color[1]).max(color[2]);
                if source.level == 0 || top <= 0.0 {
                    continue;
                }
                let [r, g, b] = color.map(|c| (c / top * 255.0).round() as u32);
                visuals.item_lights.push(mod_api::ColoredLight::new(
                    source.at.to_array(),
                    r << 16 | g << 8 | b,
                    f32::from(source.level),
                    1.0,
                ));
            }
        }
        let Some(player) = self.native_player() else {
            return visuals;
        };
        let mut rims = Vec::new();
        let mut lights = Vec::new();
        self.native_visuals(&mut rims, &mut lights);
        let close = |at: [f64; 3]| {
            at.iter().all(|v| v.is_finite())
                && (at[0] - player.x).powi(2)
                    + (at[1] - player.y).powi(2)
                    + (at[2] - player.z).powi(2)
                    <= 1024.0
        };
        let mut edges = 0;
        for rim in rims.into_iter().take(8) {
            let pos = rim.pos;
            let state = self.world.block(pos[0], pos[1], pos[2]);
            if edges >= 256
                || !rim.width.is_finite()
                || !(0.5..=4.0).contains(&rim.width)
                || !close(pos.map(|v| f64::from(v) + 0.5))
                || !rim_target_valid(
                    self.world
                        .chunks
                        .contains_key(&(pos[0].div_euclid(16), pos[2].div_euclid(16))),
                    state,
                    self.world.air,
                )
            {
                continue;
            }
            let facts = self.res.facts().get(state);
            let offset = if facts.outline_offset {
                self.pack
                    .context
                    .tables
                    .offset(state, pos[0], pos[2])
                    .unwrap_or([0.0; 3])
            } else {
                [0.0; 3]
            };
            let origin = pos.map(f64::from);
            let custom = self
                .collision
                .cached_shapes()
                .context_outline_edges(state, self.outline_context_kind());
            let box_shape = custom
                .is_none()
                .then(|| self.context_outline(pos, state, true, &self.be_anims))
                .flatten();
            let cube = horizon_client::blocks::cube_edges();
            let segments = if let Some(edges) = custom {
                edges
            } else if box_shape.is_some() {
                cube.as_slice()
            } else {
                facts.outline_edges.as_ref()
            };
            for edge in segments.iter().take(256 - edges) {
                let point = |end: usize| {
                    std::array::from_fn(|i| {
                        let v = edge[i + 3 * end];
                        if let Some(b) = box_shape {
                            origin[i] + b[i] + v * (b[i + 3] - b[i])
                        } else {
                            origin[i] + offset[i] + v
                        }
                    })
                };
                lines.push(gizmos::Line {
                    start: point(0),
                    end: point(1),
                    color: rim.argb,
                    width: rim.width,
                });
                edges += 1;
            }
        }
        lights.truncate(8);
        lights.retain(|l| {
            close(l.at)
                && l.rgb <= 0xffffff
                && l.radius.is_finite()
                && (0.25..=16.0).contains(&l.radius)
                && l.intensity.is_finite()
                && (0.0..=1.0).contains(&l.intensity)
                && self.world.chunks.contains_key(&(
                    (l.at[0].floor() as i32).div_euclid(16),
                    (l.at[2].floor() as i32).div_euclid(16),
                ))
        });
        visuals.mod_lights = lights;
        visuals
    }
}

#[cfg(test)]
mod tests {
    use super::rim_target_valid;
    use horizon_world::registry::BlockStateId;

    /// A rim and a light a mod builds with their constructors and pushes from `visuals` reach
    /// the frame: the rim as outline edges in its colour and width, the light in its colour.
    #[test]
    #[ignore = "daily"]
    fn a_mod_built_rim_and_light_reach_the_frame() {
        use mod_api::{BlockRim, ClientHost, ColoredLight, Hooks, ModEntry, VisualsEvent};
        use mod_platform::testing::with_mods;

        struct Mod;
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn visuals(&self, _: &dyn ClientHost, event: &mut VisualsEvent<'_>) {
                event.rims.push(BlockRim::new([1, 64, 1], 0xFF00_FF00, 2.0));
                let light = ColoredLight::new([1.5, 65.5, 1.5], 0xFF0000, 4.0, 0.5);
                event.lights.push(light);
            }
        }
        let mods: mod_api::Mods = vec![ModEntry::new("visual", Box::new(Mod))].into();
        with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let mut chunk = horizon_client::chunk::Chunk::empty(0, 0);
                chunk.lit = true;
                game.world.chunks.insert((0, 0), std::sync::Arc::new(chunk));
                game.world.set(1, 64, 1, stone).unwrap();
                let mut player = horizon_client::motion::PlayerMotion::new(
                    game.motion,
                    horizon_core::geometry::Vec3::ZERO,
                )
                .unwrap();
                player.position = horizon_core::geometry::Vec3::new(0.5, 65.0, 0.5);
                game.player = Some(player);

                let mut lines = Vec::new();
                let camera = glam::DVec3::new(0.5, 66.0, 0.5);
                let lights = game
                    .native_frame_visuals(0.0, &mut lines)
                    .finish(game, camera);
                assert_eq!(lines.len(), 12, "a full block's twelve edges");
                assert!(
                    lines
                        .iter()
                        .all(|l| l.color == 0xFF00_FF00 && l.width == 2.0)
                );
                assert_eq!(lights, [[[1.0, -0.5, 1.0, 4.0], [1.0, 0.0, 0.0, 0.5]]]);
            });
        });
    }

    #[test]
    fn ordinary_loaded_blocks_can_have_rims() {
        let air = BlockStateId(0);
        let stone = BlockStateId(1); // No emission requirement.
        assert!(rim_target_valid(true, stone, air));
        assert!(!rim_target_valid(false, stone, air));
        assert!(!rim_target_valid(true, air, air));
    }
}
