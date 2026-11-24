//! Full-screen camera effects drawn between the world and the HUD:
//! `ScreenEffectRenderer` (in-wall block texture, underwater overlay, fire)
//! and the full-screen `Hud` camera overlays laid out by
//! `hud_overlays::camera_overlays` (vignette, pumpkin blur, powder-snow
//! outline, nether portal swirl). Positions are clip-space quads; textures
//! come from the shared 2D atlas and may repeat. A mod's overlay quads arrive in
//! camera space ([`CameraQuads`]) and leave here in clip space too.
use crate::{
    effects::EffectInstance,
    hud::{Rect, TextureFlags},
    hud_layout::GuiDraw,
};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    /// `GUI_TEXTURED` / screen effects: alpha blending.
    Alpha,
    /// `RenderPipelines.VIGNETTE`: `dst * (1 - src)`.
    Vignette,
}
/// One clip-space vertex of a screen quad.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ScreenVertex {
    /// Clip-space position: `w` is 1 but for a mod's camera-space quads, which it keeps
    /// in perspective.
    pub pos: [f32; 4],
    /// Texture coordinates in sprite units (repeat outside 0..1).
    pub uv: [f32; 2],
    /// Atlas rectangle (u0, v0, u1, v1) of the sprite.
    pub rect: [f32; 4],
    pub color: [f32; 4],
}
/// Linear filtering (`TextureMetadataSection.blur`), else nearest.
pub const SAMPLE_BLUR: u32 = 1;
/// Clamp-to-edge addressing (`TextureMetadataSection.clamp`), else repeat.
pub const SAMPLE_CLAMP: u32 = 2;
/// The sampling bits of a standalone texture's flags.
pub fn sampling(flags: TextureFlags) -> u32 {
    let bit = |on: bool, bit: u32| if on { bit } else { 0 };
    bit(flags.blur, SAMPLE_BLUR) | bit(flags.clamp, SAMPLE_CLAMP)
}
#[derive(Clone, Debug)]
pub struct ScreenQuad {
    pub vertices: [ScreenVertex; 4],
    pub blend: Blend,
    /// How the sprite is sampled: [`SAMPLE_BLUR`] and [`SAMPLE_CLAMP`].
    pub sampling: u32,
}
/// What `block_screen_effect_draw` draws with: quads in camera space (`+x` right, `+y` up, the
/// camera at the origin looking down `-z`), under `GameRenderer.render3dHud`'s projection.
pub struct CameraQuads {
    /// `camera::hand_projection`, column-major.
    projection: [f32; 16],
    quads: Vec<ScreenQuad>,
}
impl CameraQuads {
    pub fn new(projection: [f32; 16]) -> Self {
        Self {
            projection,
            quads: Vec::new(),
        }
    }
    /// Adds a quad, alpha-blended, of atlas rectangle `rect` at `uv` in sprite units; false,
    /// with nothing added, for a coordinate that is not finite.
    pub fn push(
        &mut self,
        corners: [[f32; 3]; 4],
        uv: [[f32; 2]; 4],
        rect: [f32; 4],
        argb: u32,
        sampling: u32,
    ) -> bool {
        if !corners.as_flattened().iter().all(|c| c.is_finite())
            || !uv.as_flattened().iter().all(|c| c.is_finite())
        {
            return false;
        }
        let m = self.projection;
        let clip = |[x, y, z]: [f32; 3]| {
            std::array::from_fn(|i| m[i] * x + m[4 + i] * y + m[8 + i] * z + m[12 + i])
        };
        self.quads.push(ScreenQuad {
            vertices: std::array::from_fn(|i| ScreenVertex {
                pos: clip(corners[i]),
                uv: uv[i],
                rect,
                color: rgba(argb),
            }),
            blend: Blend::Alpha,
            sampling,
        });
        true
    }
    /// The quads added since the last call.
    pub fn take(&mut self) -> Vec<ScreenQuad> {
        std::mem::take(&mut self.quads)
    }
}
/// `0xAARRGGBB` as a vertex colour.
fn rgba(argb: u32) -> [f32; 4] {
    let [a, r, g, b] = argb.to_be_bytes();
    [r, g, b, a].map(|v| f32::from(v) / 255.0)
}

/// What the camera sees this frame.
pub struct ScreenInputs<'a> {
    pub sprites: &'a HashMap<String, Rect>,
    /// `.png.mcmeta` sampling of the standalone textures among `sprites`.
    pub flags: &'a HashMap<String, TextureFlags>,
    pub aspect: f32,
    pub partial: f32,
    /// Particle sprite of the view-blocking block around the eye.
    pub block_overlay: Option<String>,
    /// Eye in water: (lightmap brightness at the eye, yaw, pitch).
    pub water: Option<(f32, f32, f32)>,
    pub on_fire: bool,
    /// How far the fire overlay moves down, in its own heights (PvP & HUD's lower fire; 0 is
    /// vanilla's place).
    pub fire_lower: f32,
    /// `Hud.extractCameraOverlays` draws: the full-screen blits become
    /// screen quads (the spyglass, its fills and the nausea swirl do not).
    pub gui_overlays: &'a [GuiDraw],
    /// What mods drew in `block_screen_effect_draw` before the block overlay, the water
    /// overlay and the fire overlay, in that order, each shown or not.
    pub custom: [&'a [ScreenQuad]; 3],
}

/// Effect state advanced every tick.
#[derive(Clone, Copy, Debug)]
pub struct ScreenEffects {
    /// `Hud.vignetteBrightness`.
    pub vignette_brightness: f32,
    pub portal: f32,
    pub o_portal: f32,
    /// Nausea's `MobEffectInstance.BlendState` factor and the previous tick's.
    pub nausea: f32,
    pub o_nausea: f32,
    /// `LocalPlayer.spinningEffectTime` and `spinningEffectSpeed` (degrees).
    pub spinning_time: f32,
    pub spinning_speed: f32,
}
impl Default for ScreenEffects {
    fn default() -> Self {
        Self {
            vignette_brightness: 1.0,
            portal: 0.0,
            o_portal: 0.0,
            nausea: 0.0,
            o_nausea: 0.0,
            spinning_time: 0.0,
            spinning_speed: 0.0,
        }
    }
}
/// `Lightmap.getBrightness`.
pub fn lightmap_brightness(ambient_light: f32, level: u8) -> f32 {
    let v = f32::from(level) / 15.0;
    let curved = v / (4.0 - 3.0 * v);
    curved + (1.0 - curved) * ambient_light
}
/// Clip-space scale of the hand projection (fov 70) at view depth 0.5.
fn hand_scale(aspect: f32) -> [f32; 2] {
    let half = 0.5 * (35.0f32).to_radians().tan();
    [1.0 / (half * aspect), 1.0 / half]
}
impl ScreenEffects {
    /// `Gui.tick` -> `Hud.updateVignetteBrightness`: eases every client tick, even while
    /// `/tick freeze` pauses the level.
    pub fn tick_vignette(&mut self, eye_brightness: f32) {
        let target = (1.0 - eye_brightness).clamp(0.0, 1.0);
        self.vignette_brightness += (target - self.vignette_brightness) * 0.01;
    }
    /// `LocalPlayer.handlePortalTransitionEffect`: runs only while the level ticks.
    pub fn tick_portal(&mut self, inside_portal: bool) {
        self.o_portal = self.portal;
        let step = if inside_portal {
            0.0125
        } else if self.portal > 0.0 {
            -0.05
        } else {
            0.0
        };
        self.portal = (self.portal + step).clamp(0.0, 1.0);
    }
    /// `Hud.updateVignetteBrightness` and `LocalPlayer.handlePortalTransitionEffect`.
    pub fn tick(&mut self, eye_brightness: f32, inside_portal: bool) {
        self.tick_vignette(eye_brightness);
        self.tick_portal(inside_portal);
    }
    /// `LocalPlayer.tickSpinningEffect` and Nausea's blend. `nausea` is `None`
    /// without the effect, else whether it is past its blend-out point
    /// (`!endsWithin(60)`, `MobEffects.NAUSEA`).
    pub fn tick_spinning(&mut self, nausea: Option<bool>) {
        self.o_nausea = self.nausea;
        match nausea {
            // BlendState.tick: fades in over 150 ticks and out over 20.
            Some(active) => {
                let (target, duration) = if active { (1.0, 150.0) } else { (0.0, 20.0) };
                self.nausea += (target - self.nausea).clamp(-1.0 / duration, 1.0 / duration);
            }
            // The blend state belongs to the instance, so a new one starts at 0.
            None => self.nausea = 0.0,
        }
        if self.portal > 0.0 || self.nausea > 0.0 {
            self.spinning_speed =
                (self.portal * 20.0 + self.nausea * 7.0) / (self.portal + self.nausea);
            self.spinning_time += self.spinning_speed;
        } else {
            self.spinning_speed = 0.0;
        }
    }
    #[horizon_mod_api_macros::replaceable]
    pub fn build(&self, input: &ScreenInputs) -> Vec<ScreenQuad> {
        let mut out = Vec::new();
        let sprite = |name: &str| input.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        // Standalone textures (`ReloadableTexture.setSampler`) follow their
        // metadata; atlas sprites (block overlay, fire, portal) sample
        // nearest with repeat.
        let texture_sampling = |name: &str| input.flags.get(name).copied().map_or(0, sampling);
        let scale = hand_scale(input.aspect);
        // Screen-effect quads live at view depth 0.5 in the hand projection.
        let hand_quad = |corners: [[f32; 2]; 4],
                         uv: [[f32; 2]; 4],
                         rect: [f32; 4],
                         color: [f32; 4],
                         sampling: u32| ScreenQuad {
            vertices: std::array::from_fn(|i| ScreenVertex {
                pos: [corners[i][0] * scale[0], corners[i][1] * scale[1], 0.0, 1.0],
                uv: uv[i],
                rect,
                color,
            }),
            blend: Blend::Alpha,
            sampling,
        };
        let full = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
        // ScreenEffectRenderer.submitBlockSprite: u1/v1 at (-1, -1), colour 0xFF191919.
        out.extend_from_slice(input.custom[0]);
        if let Some(name) = &input.block_overlay
            && let Some(rect) = sprite(name)
        {
            let c = 0x19 as f32 / 255.0;
            out.push(hand_quad(
                full,
                [[1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]],
                rect,
                [c, c, c, 1.0],
                0,
            ));
        }
        out.extend_from_slice(input.custom[1]);
        if let Some((brightness, yaw, pitch)) = input.water
            && let Some(rect) = sprite("misc/underwater")
        {
            let (u, v) = (-yaw / 64.0, pitch / 64.0);
            let uv = [[u + 4.0, v + 4.0], [u, v + 4.0], [u, v], [u + 4.0, v]];
            out.push(hand_quad(
                full,
                uv,
                rect,
                [brightness, brightness, brightness, 0.1],
                texture_sampling("misc/underwater"),
            ));
        }
        out.extend_from_slice(input.custom[2]);
        if input.on_fire
            && let Some(rect) = sprite("block/fire_1")
        {
            for (dx, angle) in [(0.24f32, -10.0f32), (-0.24, 10.0)] {
                let (s, c) = angle.to_radians().sin_cos();
                // rotateY about the quad then translate; z = -0.5 feeds into x.
                let corner = |x: f32, y: f32| [x * c + -0.5 * s + dx, y - 0.3 - input.fire_lower];
                let corners = [
                    corner(-0.5, -0.5),
                    corner(0.5, -0.5),
                    corner(0.5, 0.5),
                    corner(-0.5, 0.5),
                ];
                let alpha = 0xE5 as f32 / 255.0;
                out.push(hand_quad(
                    corners,
                    [[1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]],
                    rect,
                    [1.0, 1.0, 1.0, alpha],
                    0,
                ));
            }
        }
        // Hud camera overlays: GUI-space full-screen blits (v grows downwards).
        let gui = |rect: [f32; 4], color: [f32; 4], blend: Blend, sampling: u32| ScreenQuad {
            vertices: std::array::from_fn(|i| {
                let p = [[-1.0f32, 1.0], [1.0, 1.0], [1.0, -1.0], [-1.0, -1.0]][i];
                ScreenVertex {
                    pos: [p[0], p[1], 0.0, 1.0],
                    uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]][i],
                    rect,
                    color,
                }
            }),
            blend,
            sampling,
        };
        for d in input.gui_overlays {
            match d {
                GuiDraw::Blit {
                    texture,
                    blend,
                    x: 0,
                    y: 0,
                    color,
                    ..
                } if *blend != crate::hud_layout::Blend::Nausea
                    && texture != "misc/spyglass_scope" =>
                {
                    let blend = if *blend == crate::hud_layout::Blend::Vignette {
                        Blend::Vignette
                    } else {
                        Blend::Alpha
                    };
                    // The vignette darkens by texture × colour: black (in
                    // daylight its brightness fades to 0 within half a
                    // minute) leaves every pixel as it was.
                    if blend == Blend::Vignette && color & 0x00ff_ffff == 0 {
                        continue;
                    }
                    if let Some(rect) = sprite(texture) {
                        out.push(gui(rect, rgba(*color), blend, texture_sampling(texture)));
                    }
                }
                GuiDraw::AtlasSprite {
                    sprite: name,
                    color,
                    ..
                } => {
                    if let Some(rect) = sprite(name) {
                        out.push(gui(rect, rgba(*color), Blend::Alpha, 0));
                    }
                }
                _ => {}
            }
        }
        out
    }
}

/// The nausea effect's blend-out advance: over its last `NAUSEA_BLEND_OUT`
/// ticks it has no effect (`MobEffects.NAUSEA`, `endsWithin(60)`).
const NAUSEA_BLEND_OUT: i32 = 60;

/// `MobEffectInstance.BlendState.hasEffect`: still past the blend-out point,
/// so the effect counts as on (`!endsWithin(60)` for `MobEffects.NAUSEA`).
pub fn nausea_has_effect(e: &EffectInstance) -> bool {
    !e.ends_within(NAUSEA_BLEND_OUT)
}

/// A new nausea instance's `BlendState`: from nothing, or, when the server
/// asked for no blending, at once (`MobEffectInstance.skipBlending`'s
/// `BlendState.setImmediate`).
pub fn nausea_blend_added(e: &EffectInstance, blend: bool) -> (f32, f32) {
    let factor = if blend || !nausea_has_effect(e) {
        0.0
    } else {
        1.0
    };
    (factor, factor)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lightmap_brightness_curve() {
        assert_eq!(lightmap_brightness(0.0, 15), 1.0);
        assert_eq!(lightmap_brightness(0.0, 0), 0.0);
        assert!((lightmap_brightness(0.1, 0) - 0.1).abs() < 1e-6);
    }
    #[test]
    fn portal_intensity_ramps() {
        let mut e = ScreenEffects::default();
        for _ in 0..80 {
            e.tick(1.0, true);
        }
        assert!((e.portal - 1.0).abs() < 1e-5);
        e.tick(1.0, false);
        assert!((e.portal - 0.95).abs() < 1e-5);
        // Vignette fades towards 1 - brightness at 1% per tick.
        assert!(e.vignette_brightness < 1.0);
    }
    #[test]
    fn overlays_need_their_sprites() {
        let mut sprites = HashMap::new();
        sprites.insert(
            "misc/vignette".to_owned(),
            Rect {
                u0: 0.0,
                v0: 0.0,
                u1: 1.0,
                v1: 1.0,
                w: 256,
                h: 256,
            },
        );
        let e = ScreenEffects::default();
        let flags = HashMap::new();
        let input = ScreenInputs {
            sprites: &sprites,
            flags: &flags,
            aspect: 1.6,
            partial: 0.0,
            block_overlay: None,
            water: Some((1.0, 0.0, 0.0)),
            on_fire: false,
            fire_lower: 0.0,
            gui_overlays: &[crate::hud_overlays::vignette(320, 240, 1.0, 0.0)],
            custom: [&[]; 3],
        };
        let quads = e.build(&input);
        assert_eq!(quads.len(), 1);
        assert_eq!(quads[0].blend, Blend::Vignette);
        assert_eq!(quads[0].sampling, 0);
        // A black vignette (faded out in daylight) changes no pixel: no quad.
        // Below 1/255 the colour's channels are 0; a border warning tints it.
        for (brightness, warning, drawn) in [(0.003, 0.0, 0), (0.004, 0.0, 1), (0.0, 0.5, 1)] {
            let overlays = [crate::hud_overlays::vignette(320, 240, brightness, warning)];
            let input = ScreenInputs {
                block_overlay: None,
                gui_overlays: &overlays,
                ..input
            };
            assert_eq!(e.build(&input).len(), drawn, "{brightness} {warning}");
        }
    }
    #[test]
    fn standalone_textures_sample_by_their_metadata() {
        let rect = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 16,
            h: 16,
        };
        let sprites: HashMap<_, _> = ["misc/vignette", "misc/underwater", "block/stone"]
            .map(|name| (name.to_owned(), rect))
            .into();
        let blur = TextureFlags {
            blur: true,
            clamp: false,
        };
        let clamp = TextureFlags {
            blur: false,
            clamp: true,
        };
        let flags: HashMap<_, _> = [
            ("misc/vignette".to_owned(), blur),
            ("misc/underwater".to_owned(), clamp),
            // Atlas sprites ignore the texture section.
            ("block/stone".to_owned(), blur),
        ]
        .into();
        let quads = ScreenEffects::default().build(&ScreenInputs {
            sprites: &sprites,
            flags: &flags,
            aspect: 1.6,
            partial: 0.0,
            block_overlay: Some("block/stone".to_owned()),
            water: Some((1.0, 0.0, 0.0)),
            on_fire: false,
            fire_lower: 0.0,
            gui_overlays: &[crate::hud_overlays::vignette(320, 240, 1.0, 0.0)],
            custom: [&[]; 3],
        });
        let sampling: Vec<_> = quads.iter().map(|q| q.sampling).collect();
        assert_eq!(sampling, [0, SAMPLE_CLAMP, SAMPLE_BLUR]);
    }
    /// A camera-space corner under a 90 degree square projection is at `(x, y)` with `w` its
    /// depth, whatever the depth, so a deeper corner of the same quad draws nearer the centre.
    #[test]
    fn camera_quads_keep_their_depth_for_perspective() {
        let mut quads = CameraQuads::new(crate::camera::hand_projection(90.0, 1.0, 1.0, 100.0));
        let corners = [
            [-0.5, -0.5, -0.5],
            [0.5, -0.5, -1.0],
            [0.5, 0.5, -1.0],
            [-0.5, 0.5, -0.5],
        ];
        let uv = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        let rect = [0.25, 0.25, 0.5, 0.5];
        assert!(quads.push(corners, uv, rect, 0x80FF0000, 0));
        let drawn = quads.take();
        assert!(quads.take().is_empty());
        let [near, far, ..] = drawn[0].vertices;
        assert!((near.pos[0] + 0.5).abs() < 1e-5 && (near.pos[3] - 0.5).abs() < 1e-6);
        assert!((far.pos[0] - 0.5).abs() < 1e-5 && (far.pos[3] - 1.0).abs() < 1e-6);
        assert_eq!(near.rect, rect);
        assert_eq!(near.color, [1.0, 0.0, 0.0, 128.0 / 255.0]);
    }
    #[test]
    fn camera_quads_refuse_what_is_not_finite() {
        let mut quads = CameraQuads::new(crate::camera::hand_projection(70.0, 2.0, 1.0, 100.0));
        let corners = [[0.0, 0.0, -1.0]; 4];
        assert!(!quads.push([[f32::NAN; 3]; 4], [[0.0; 2]; 4], [0.0; 4], 0, 0));
        assert!(!quads.push(corners, [[f32::INFINITY; 2]; 4], [0.0; 4], 0, 0));
        assert!(quads.take().is_empty());
    }
    /// What a mod drew goes right before the overlay it belongs to, shown or not.
    #[test]
    fn custom_quads_go_before_their_overlays() {
        let rect = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 16,
            h: 16,
        };
        let sprites: HashMap<_, _> = ["block/stone", "block/fire_1"]
            .map(|name| (name.to_owned(), rect))
            .into();
        let mut quads = CameraQuads::new(crate::camera::hand_projection(70.0, 2.0, 1.0, 100.0));
        let mut custom = |argb: u32| {
            quads.push([[0.0, 0.0, -1.0]; 4], [[0.0; 2]; 4], [0.0; 4], argb, 0);
            quads.take()
        };
        let (block, water, fire) = (custom(0xFF010000), custom(0xFF020000), custom(0xFF030000));
        let flags = HashMap::new();
        let drawn = ScreenEffects::default().build(&ScreenInputs {
            sprites: &sprites,
            flags: &flags,
            aspect: 2.0,
            partial: 0.0,
            block_overlay: Some("block/stone".to_owned()),
            water: None,
            on_fire: true,
            fire_lower: 0.0,
            gui_overlays: &[],
            custom: [&block, &water, &fire],
        });
        let reds: Vec<_> = drawn
            .iter()
            .map(|quad| (quad.vertices[0].color[0] * 255.0).round() as u8)
            .collect();
        // Block's own overlay is grey 0x19, the fire's two quads white.
        assert_eq!(reds, [1, 0x19, 2, 3, 255, 255]);
    }
    #[test]
    fn mcmeta_texture_section() {
        let parse = TextureFlags::from_mcmeta;
        assert_eq!(
            parse(r#"{"texture":{"blur":true}}"#),
            Some(TextureFlags {
                blur: true,
                clamp: false
            })
        );
        assert_eq!(
            parse(r#"{"texture":{"clamp":true,"mipmap_strategy":"mean"}}"#),
            Some(TextureFlags {
                blur: false,
                clamp: true
            })
        );
        // Defaults (false, false) or no section: the default sampler.
        assert_eq!(parse(r#"{"texture":{"blur":false}}"#), None);
        assert_eq!(parse(r#"{"animation":{}}"#), None);
        assert_eq!(parse("not json"), None);
    }
    #[test]
    fn tick_vignette_eases_independently_of_portal_tick() {
        let mut fx = ScreenEffects {
            vignette_brightness: 1.0,
            portal: 0.25,
            o_portal: 0.2,
            ..ScreenEffects::default()
        };
        fx.tick_vignette(1.0);
        assert!((fx.vignette_brightness - 0.99).abs() < 1e-6);
        assert_eq!(fx.portal, 0.25);
        assert_eq!(fx.o_portal, 0.2);
    }
}
