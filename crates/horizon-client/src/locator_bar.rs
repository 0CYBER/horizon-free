//! The locator bar: `ClientWaypointManager` (the waypoints the server
//! tracks for this player, from `ClientboundTrackedWaypointPacket`),
//! `WaypointStyleManager` (`waypoint_style/*.json`: near and far distances
//! and the dot sprites between them) and `LocatorBar`, the contextual bar
//! that draws a dot per waypoint within 60 degrees of the view, with an
//! arrow when it is above or below the screen.
use crate::hud_layout::{Blend, GuiDraw};
use horizon_assets::animation::{AnimationClip, AnimationMetadata};
use horizon_core::geometry::trig;
use horizon_protocol::packets::play::{
    TrackedWaypoint, WaypointId, WaypointTarget, waypoint_operation,
};
use std::{borrow::Cow, collections::HashMap, fs, path::Path};

/// `WaypointStyle`: the sprites (under `hud/locator_bar_dot/`) from near to far.
#[derive(Clone, Debug, PartialEq)]
pub struct WaypointStyle {
    pub near_distance: i32,
    pub far_distance: i32,
    pub sprites: Vec<String>,
}
impl WaypointStyle {
    /// `WaypointStyleManager.MISSING`.
    fn missing() -> Self {
        Self {
            near_distance: 0,
            far_distance: 1,
            sprites: vec!["hud/locator_bar_dot/missingno".into()],
        }
    }
    /// `WaypointStyle.CODEC` and `validate`: None when the file is invalid.
    pub fn parse(json: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(json).ok()?;
        let distance = |key: &str, default: i32| match v.get(key) {
            None => Some(default),
            Some(d) => d
                .as_i64()
                .filter(|d| (0..=60_000_000).contains(d))
                .map(|d| d as i32),
        };
        let near_distance = distance("near_distance", 128)?;
        let far_distance = distance("far_distance", 332)?;
        let sprites = v
            .get("sprites")?
            .as_array()?
            .iter()
            .map(|s| {
                let id = s.as_str()?;
                let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
                // Sprites of the gui atlas are named by path (minecraft only).
                (namespace == "minecraft").then(|| format!("hud/locator_bar_dot/{path}"))
            })
            .collect::<Option<Vec<_>>>()?;
        (!sprites.is_empty() && near_distance > 0 && near_distance < far_distance).then_some(Self {
            near_distance,
            far_distance,
            sprites,
        })
    }
    /// `WaypointStyle.sprite`.
    pub fn sprite(&self, distance: f32) -> &str {
        let (first, last) = (&self.sprites[0], &self.sprites[self.sprites.len() - 1]);
        if distance < self.near_distance as f32 {
            return first;
        }
        if distance >= self.far_distance as f32 {
            return last;
        }
        match self.sprites.len() {
            1 => first,
            3 => &self.sprites[1],
            n => {
                let t = (distance - self.near_distance as f32)
                    / (self.far_distance - self.near_distance) as f32;
                // Mth.lerpInt(t, 1, n - 1).
                &self.sprites[(1 + (t * (n as f32 - 2.0)).floor() as i32) as usize]
            }
        }
    }
}

/// An animated GUI sprite (`.png.mcmeta` animation, ticked with the GUI
/// atlas): its image and frame sizes and the frame sequence.
#[derive(Clone, Debug)]
pub struct SpriteFrames {
    image: [i32; 2],
    frame: [i32; 2],
    clip: AnimationClip,
}
impl SpriteFrames {
    /// None for a still sprite or invalid metadata.
    fn load(png: &Path) -> Option<Self> {
        let text = fs::read_to_string(png.with_extension("png.mcmeta")).ok()?;
        let meta: HashMap<String, Box<serde_json::value::RawValue>> =
            serde_json::from_str(&text).ok()?;
        let animation = AnimationMetadata::from_json(meta.get("animation")?.get()).ok()?;
        // The PNG header: width and height at bytes 16..24 of IHDR.
        let mut header = [0u8; 24];
        std::io::Read::read_exact(&mut fs::File::open(png).ok()?, &mut header).ok()?;
        let dim = |at: usize| i32::from_be_bytes(header[at..at + 4].try_into().unwrap_or_default());
        let image = [dim(16), dim(20)];
        let frame = animation.frame_size(image[0], image[1]);
        if frame[0] <= 0 || frame[1] <= 0 || image[0] % frame[0] != 0 || image[1] % frame[1] != 0 {
            return None;
        }
        let count = (image[0] / frame[0]) * (image[1] / frame[1]);
        let clip = animation.clip(count as u32).ok()??;
        Some(Self { image, frame, clip })
    }
    /// The `region` (image width, height, u, v) of the frame shown at `tick`.
    fn region(&self, tick: u64) -> [i32; 4] {
        let index = self.clip.sample(tick).frame as i32;
        let columns = self.image[0] / self.frame[0];
        [
            self.image[0],
            self.image[1],
            index % columns * self.frame[0],
            index / columns * self.frame[1],
        ]
    }
}

/// `WaypointStyleManager`: every `assets/<namespace>/waypoint_style/*.json`,
/// plus the animations of the bar's own sprites (the arrows blink).
#[derive(Clone, Debug, Default)]
pub struct WaypointStyles(
    HashMap<String, WaypointStyle>,
    HashMap<String, SpriteFrames>,
);
impl WaypointStyles {
    pub fn load(root: &Path) -> Self {
        let mut frames = HashMap::new();
        let hud = root.join("assets/minecraft/textures/gui/sprites/hud");
        let dots = fs::read_dir(hud.join("locator_bar_dot"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path());
        let arrows = ["locator_bar_arrow_up", "locator_bar_arrow_down"]
            .map(|a| hud.join(format!("{a}.png")));
        for png in arrows.into_iter().chain(dots) {
            if png.extension().is_none_or(|e| e != "png") {
                continue;
            }
            let Some(f) = SpriteFrames::load(&png) else {
                continue;
            };
            let name = png
                .strip_prefix(hud.parent().unwrap_or(&hud))
                .map(|r| r.with_extension("").to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            frames.insert(name, f);
        }
        let mut out = HashMap::new();
        for ns in fs::read_dir(root.join("assets"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let namespace = ns.file_name().to_string_lossy().into_owned();
            for f in fs::read_dir(ns.path().join("waypoint_style"))
                .into_iter()
                .flatten()
                .flatten()
            {
                let path = f.path();
                if path.extension().is_none_or(|e| e != "json") {
                    continue;
                }
                let name = path.file_stem().unwrap_or_default().to_string_lossy();
                if let Some(style) = fs::read_to_string(&path)
                    .ok()
                    .and_then(|t| WaypointStyle::parse(&t))
                {
                    out.insert(format!("{namespace}:{name}"), style);
                } else {
                    log::warn!("Horizon: invalid waypoint style {}", path.display());
                }
            }
        }
        Self(out, frames)
    }
    /// The sprite at `tick`, with its animation frame when it has one.
    fn sprite(&self, name: &str, x: i32, y: i32, w: i32, h: i32, color: u32, tick: u64) -> GuiDraw {
        let mut d = sprite(name, x, y, w, h, color);
        if let (Some(f), GuiDraw::Sprite { region, .. }) = (self.1.get(name), &mut d) {
            *region = Some(f.region(tick));
        }
        d
    }
    /// `get`: the style, or the missing one.
    pub fn get(&self, id: &str) -> Cow<'_, WaypointStyle> {
        let id = if id.contains(':') {
            Cow::Borrowed(id)
        } else {
            Cow::Owned(format!("minecraft:{id}"))
        };
        self.0
            .get(&*id)
            .map_or_else(|| Cow::Owned(WaypointStyle::missing()), Cow::Borrowed)
    }
}

/// `ClientWaypointManager`: the tracked waypoints by identifier.
#[derive(Clone, Debug, Default)]
pub struct ClientWaypoints {
    waypoints: Vec<TrackedWaypoint>,
}
impl ClientWaypoints {
    /// `ClientboundTrackedWaypointPacket.apply`.
    pub fn apply(&mut self, packet: &TrackedWaypoint) {
        let at = self.waypoints.iter().position(|w| w.id == packet.id);
        match packet.operation {
            waypoint_operation::TRACK => match at {
                Some(i) => self.waypoints[i] = packet.clone(),
                None => self.waypoints.push(packet.clone()),
            },
            waypoint_operation::UNTRACK => {
                if let Some(i) = at {
                    self.waypoints.remove(i);
                }
            }
            // `TrackedWaypoint.update`: only the same kind of waypoint moves.
            _ => {
                if let Some(w) = at.map(|i| &mut self.waypoints[i]) {
                    use WaypointTarget as T;
                    if matches!(
                        (w.target, packet.target),
                        (T::Block(_), T::Block(_))
                            | (T::Chunk(_), T::Chunk(_))
                            | (T::Azimuth(_), T::Azimuth(_))
                    ) {
                        w.target = packet.target;
                    }
                }
            }
        }
    }
    pub fn has_waypoints(&self) -> bool {
        !self.waypoints.is_empty()
    }
    pub fn clear(&mut self) {
        self.waypoints.clear();
    }
}

/// What the locator bar needs of the camera and the world this frame.
pub struct View<'a> {
    pub gui_width: i32,
    pub gui_height: i32,
    /// The camera entity's UUID, its block position and its position.
    pub camera_uuid: u128,
    pub camera_entity_pos: [f64; 3],
    /// `Camera.position()` and `yRot()`, `xRot()`, `getFov()` in degrees.
    pub camera_pos: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub fov: f32,
    /// `Camera.getViewRotationProjectionMatrix`, applied to a point
    /// relative to the camera: normalised device coordinates.
    pub project: &'a dyn Fn([f64; 3]) -> [f64; 3],
    /// An entity with this UUID in the level: (block position, eye position).
    pub entity: &'a dyn Fn(u128) -> Option<([i32; 3], [f64; 3])>,
    pub styles: &'a WaypointStyles,
    /// Client ticks, for animated sprites.
    pub tick: u64,
}

/// `TrackedWaypoint.PitchDirection`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pitch {
    None,
    Up,
    Down,
}

/// `UUID.hashCode`.
fn uuid_hash(uuid: u128) -> i32 {
    let hilo = (uuid >> 64) as u64 ^ uuid as u64;
    ((hilo >> 32) as i32) ^ (hilo as i32)
}
/// `String.hashCode`.
fn string_hash(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
}

/// `ARGB.setBrightness`.
pub fn set_brightness(color: u32, brightness: f32) -> u32 {
    let [alpha, r, g, b] = color.to_be_bytes().map(i32::from);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let range = (max - min) as f32;
    let saturation = if max != 0 { range / max as f32 } else { 0.0 };
    let argb =
        |r: i32, g: i32, b: i32| u32::from_be_bytes([alpha as u8, r as u8, g as u8, b as u8]);
    let round = |v: f32| (v * 255.0).round() as i32;
    if saturation == 0.0 {
        let v = round(brightness);
        return argb(v, v, v);
    }
    let (cr, cg, cb) = (
        (max - r) as f32 / range,
        (max - g) as f32 / range,
        (max - b) as f32 / range,
    );
    let mut hue = if r == max {
        cb - cg
    } else if g == max {
        2.0 + cr - cb
    } else {
        4.0 + cg - cr
    } / 6.0;
    if hue < 0.0 {
        hue += 1.0;
    }
    let segment = (hue - hue.floor()) * 6.0;
    let offset = segment - segment.floor();
    let primary = brightness * (1.0 - saturation);
    let secondary = brightness * (1.0 - saturation * offset);
    let tertiary = brightness * (1.0 - saturation * (1.0 - offset));
    let full = round(brightness);
    match segment as i32 {
        0 => argb(full, round(tertiary), round(primary)),
        1 => argb(round(secondary), full, round(primary)),
        2 => argb(round(primary), full, round(tertiary)),
        3 => argb(round(primary), round(secondary), full),
        4 => argb(round(tertiary), round(primary), full),
        5 => argb(full, round(primary), round(secondary)),
        _ => argb(r, g, b),
    }
}

/// `Mth.wrapDegrees`.
fn wrap_degrees(angle: f32) -> f32 {
    let mut a = angle % 360.0;
    if a >= 180.0 {
        a -= 360.0;
    }
    if a < -180.0 {
        a += 360.0;
    }
    a
}

fn centre(v: [i32; 3]) -> [f64; 3] {
    v.map(|c| f64::from(c) + 0.5)
}

impl View<'_> {
    /// `Vec3iWaypoint.position`: the tracked entity's eyes when it is within
    /// 3 blocks (Manhattan) of the waypoint, else the block's centre.
    fn block_position(&self, w: &TrackedWaypoint, v: [i32; 3]) -> [f64; 3] {
        if let WaypointId::Uuid(uuid) = w.id
            && let Some((block, eyes)) = (self.entity)(uuid)
        {
            let manhattan: i32 = (0..3).map(|i| (block[i] - v[i]).abs()).sum();
            if manhattan <= 3 {
                return eyes;
            }
        }
        centre(v)
    }
    /// `yawAngleToCamera` of a point: `degreesDifference(camera yaw, the
    /// angle of camera - point turned clockwise by 90 degrees)`.
    fn yaw_to(&self, p: [f64; 3]) -> f64 {
        let c = self.camera_pos;
        // Vec3.rotateClockwise90: (-z, y, x).
        let (x, z) = (-(c[2] - p[2]), c[0] - p[0]);
        let angle = trig::atan2(z, x) as f32 * (180.0 / std::f32::consts::PI);
        f64::from(wrap_degrees(angle - self.yaw))
    }
    /// `ChunkPos.getMiddleBlockPosition(y)`'s centre.
    fn chunk_middle(chunk: [i32; 2], y: i32) -> [f64; 3] {
        centre([chunk[0] * 16 + 8, y, chunk[1] * 16 + 8])
    }
    fn yaw_angle(&self, w: &TrackedWaypoint) -> f64 {
        match w.target {
            WaypointTarget::Empty => f64::NAN,
            WaypointTarget::Block(v) => self.yaw_to(self.block_position(w, v)),
            WaypointTarget::Chunk(c) => {
                self.yaw_to(Self::chunk_middle(c, self.camera_pos[1] as i32))
            }
            WaypointTarget::Azimuth(a) => {
                f64::from(wrap_degrees(a * (180.0 / std::f32::consts::PI) - self.yaw))
            }
        }
    }
    /// `GameRenderer.projectHorizonToScreen`.
    fn horizon(&self) -> f64 {
        if self.pitch <= -90.0 {
            return f64::NEG_INFINITY;
        }
        if self.pitch >= 90.0 {
            return f64::INFINITY;
        }
        // Math.tan of the float products, in double.
        let rad = std::f32::consts::PI / 180.0;
        f64::from(self.pitch * rad).tan() / f64::from(self.fov / 2.0 * rad).tan()
    }
    fn pitch_direction(&self, w: &TrackedWaypoint) -> Pitch {
        let by = |y: f64| {
            if y < -1.0 {
                Pitch::Down
            } else if y > 1.0 {
                Pitch::Up
            } else {
                Pitch::None
            }
        };
        match w.target {
            WaypointTarget::Empty => Pitch::None,
            WaypointTarget::Chunk(_) | WaypointTarget::Azimuth(_) => by(self.horizon()),
            WaypointTarget::Block(v) => {
                let p = self.block_position(w, v);
                let c = self.camera_pos;
                let s = (self.project)([p[0] - c[0], p[1] - c[1], p[2] - c[2]]);
                let behind = s[2] > 1.0;
                match by(if behind { -s[1] } else { s[1] }) {
                    Pitch::None if behind && s[1] > 0.0 => Pitch::Up,
                    Pitch::None if behind && s[1] < 0.0 => Pitch::Down,
                    p => p,
                }
            }
        }
    }
    /// `distanceSquared` from the camera entity.
    fn distance_sq(&self, w: &TrackedWaypoint) -> f64 {
        let e = self.camera_entity_pos;
        let d = |p: [f64; 3]| (0..3).map(|i| (p[i] - e[i]).powi(2)).sum::<f64>();
        match w.target {
            WaypointTarget::Block(v) => d(centre(v)),
            WaypointTarget::Chunk(c) => d(Self::chunk_middle(c, e[1].floor() as i32)),
            WaypointTarget::Empty | WaypointTarget::Azimuth(_) => f64::INFINITY,
        }
    }
}

/// `LocatorBar.extractBackground`: the nine-sliced sprite and its rectangle.
pub fn background(gui_width: i32, gui_height: i32) -> (&'static str, [i32; 4]) {
    (
        "gui/sprites/hud/locator_bar_background",
        [gui_width / 2 - 91, gui_height - 32 + 3, 182, 5],
    )
}

fn sprite(name: &str, x: i32, y: i32, w: i32, h: i32, color: u32) -> GuiDraw {
    GuiDraw::Sprite {
        sprite: Cow::Owned(name.to_owned()),
        x,
        y,
        w,
        h,
        region: None,
        color,
        blend: Blend::Textured,
    }
}

impl ClientWaypoints {
    /// `LocatorBar.extractRenderState`: the farthest waypoint first.
    pub fn draw(&self, view: &View) -> Vec<GuiDraw> {
        let top = view.gui_height - 32 + 3;
        let mut sorted: Vec<(f64, &TrackedWaypoint)> = self
            .waypoints
            .iter()
            .map(|w| (view.distance_sq(w), w))
            .collect();
        sorted.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut out = Vec::new();
        for (distance_sq, w) in sorted {
            if w.id == WaypointId::Uuid(view.camera_uuid) {
                continue;
            }
            let angle = view.yaw_angle(w);
            // `!(angle <= -60) && !(angle > 60)`: NaN draws, at the centre.
            if angle <= -60.0 || angle > 60.0 {
                continue;
            }
            let middle = ((view.gui_width - 9) as f32 / 2.0).ceil() as i32;
            let style = view.styles.get(&w.style);
            let distance = (distance_sq as f32).sqrt();
            let color = w.color.map_or_else(
                || {
                    let hash = match &w.id {
                        WaypointId::Uuid(u) => uuid_hash(*u),
                        WaypointId::Name(n) => string_hash(n),
                    };
                    set_brightness(0xFF00_0000 | hash as u32, 0.9)
                },
                |c| c as u32,
            );
            let dot = (angle * 173.0 / 2.0 / 60.0).floor() as i32;
            out.push(view.styles.sprite(
                style.sprite(distance),
                middle + dot,
                top - 2,
                9,
                9,
                color,
                view.tick,
            ));
            let arrow = match view.pitch_direction(w) {
                Pitch::None => continue,
                Pitch::Down => (6, "hud/locator_bar_arrow_down"),
                Pitch::Up => (-6, "hud/locator_bar_arrow_up"),
            };
            out.push(view.styles.sprite(
                arrow.1,
                middle + dot + 1,
                top + arrow.0,
                7,
                5,
                u32::MAX,
                view.tick,
            ));
        }
        out
    }
}

/// `Hud.ContextualInfo`: which bar sits above the hotbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextualBar {
    Empty,
    Experience,
    Locator,
    JumpableVehicle,
}
impl ContextualBar {
    /// `Hud.nextContextualInfoState`: `recent_experience` is
    /// `willPrioritizeExperienceInfo` (experience changed in the last 100
    /// ticks), `jumping` is `willPrioritizeJumpInfo`.
    pub fn next(
        waypoints: bool,
        jumpable_vehicle: bool,
        jumping: bool,
        has_experience: bool,
        recent_experience: bool,
    ) -> Self {
        if waypoints {
            if jumpable_vehicle && jumping {
                Self::JumpableVehicle
            } else if has_experience && recent_experience {
                Self::Experience
            } else {
                Self::Locator
            }
        } else if jumpable_vehicle {
            Self::JumpableVehicle
        } else if has_experience {
            Self::Experience
        } else {
            Self::Empty
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracked(operation: i32, id: u128, target: WaypointTarget) -> TrackedWaypoint {
        TrackedWaypoint {
            operation,
            id: WaypointId::Uuid(id),
            style: "minecraft:default".into(),
            color: None,
            target,
        }
    }

    #[test]
    fn styles_pick_sprites_by_distance() {
        let default = WaypointStyle::parse(
            r#"{"sprites":["minecraft:default_0","minecraft:default_1","minecraft:default_2","minecraft:default_3"]}"#,
        )
        .unwrap();
        assert_eq!((default.near_distance, default.far_distance), (128, 332));
        assert_eq!(default.sprite(10.0), "hud/locator_bar_dot/default_0");
        assert_eq!(default.sprite(128.0), "hud/locator_bar_dot/default_1");
        // lerpInt((d - 128) / 204, 1, 3): halfway is the third sprite.
        assert_eq!(default.sprite(229.0), "hud/locator_bar_dot/default_1");
        assert_eq!(default.sprite(230.0), "hud/locator_bar_dot/default_2");
        assert_eq!(default.sprite(332.0), "hud/locator_bar_dot/default_3");
        let bowtie = WaypointStyle::parse(
            r#"{"near_distance":64,"sprites":["minecraft:bowtie","minecraft:default_0","minecraft:default_1","minecraft:default_2","minecraft:default_3"]}"#,
        )
        .unwrap();
        assert_eq!(bowtie.sprite(63.0), "hud/locator_bar_dot/bowtie");
        // lerpInt((d - 64) / 268, 1, 4).
        assert_eq!(bowtie.sprite(64.0), "hud/locator_bar_dot/default_0");
        assert_eq!(bowtie.sprite(200.0), "hud/locator_bar_dot/default_1");
        assert_eq!(bowtie.sprite(300.0), "hud/locator_bar_dot/default_2");
        // Three sprites: the middle one between near and far.
        let three = WaypointStyle::parse(r#"{"sprites":["a","b","c"]}"#).unwrap();
        assert_eq!(three.sprite(200.0), "hud/locator_bar_dot/b");
        // validate: sprites, near > 0, far > near.
        assert!(WaypointStyle::parse(r#"{"sprites":[]}"#).is_none());
        assert!(WaypointStyle::parse(r#"{"near_distance":0,"sprites":["a"]}"#).is_none());
        assert!(WaypointStyle::parse(r#"{"near_distance":400,"sprites":["a"]}"#).is_none());
        assert_eq!(
            WaypointStyles::default().get("minecraft:none").sprite(5.0),
            "hud/locator_bar_dot/missingno"
        );
    }

    #[test]
    fn packets_track_update_and_untrack() {
        let mut w = ClientWaypoints::default();
        w.apply(&tracked(
            waypoint_operation::TRACK,
            1,
            WaypointTarget::Block([1, 2, 3]),
        ));
        assert!(w.has_waypoints());
        w.apply(&tracked(
            waypoint_operation::UPDATE,
            1,
            WaypointTarget::Block([4, 5, 6]),
        ));
        assert_eq!(w.waypoints[0].target, WaypointTarget::Block([4, 5, 6]));
        // Another kind of waypoint doesn't update it.
        w.apply(&tracked(
            waypoint_operation::UPDATE,
            1,
            WaypointTarget::Azimuth(1.0),
        ));
        assert_eq!(w.waypoints[0].target, WaypointTarget::Block([4, 5, 6]));
        w.apply(&tracked(
            waypoint_operation::UNTRACK,
            1,
            WaypointTarget::Empty,
        ));
        assert!(!w.has_waypoints());
    }

    #[test]
    fn brightness_like_argb() {
        // Pure red at 0.9: Math.round(0.9 * 255) = 230.
        assert_eq!(set_brightness(0xFFFF_0000, 0.9), 0xFFE6_0000);
        assert_eq!(set_brightness(0xFF80_8080, 0.9), 0xFFE6_E6E6);
        assert_eq!(uuid_hash(0x0000_0001_0000_0000_0000_0000_0000_0002), 3);
        assert_eq!(string_hash("ab"), 97 * 31 + 98);
    }

    #[test]
    fn arrows_blink_through_their_frames() {
        // locator_bar_arrow_up.png.mcmeta: 7x10, two 5-pixel frames.
        let meta = r#"{"frames":[{"index":0,"time":10},{"index":1,"time":4}],"height":5}"#;
        let animation = AnimationMetadata::from_json(meta).unwrap();
        let frames = SpriteFrames {
            image: [7, 10],
            frame: animation.frame_size(7, 10),
            clip: animation.clip(2).unwrap().unwrap(),
        };
        assert_eq!(frames.region(0), [7, 10, 0, 0]);
        assert_eq!(frames.region(9), [7, 10, 0, 0]);
        assert_eq!(frames.region(10), [7, 10, 0, 5]);
        assert_eq!(frames.region(14), [7, 10, 0, 0]);
    }

    #[test]
    fn dots_sit_on_the_bar_by_angle() {
        let styles = WaypointStyles(
            [(
                "minecraft:default".to_owned(),
                WaypointStyle {
                    near_distance: 128,
                    far_distance: 332,
                    sprites: vec!["hud/locator_bar_dot/default_0".into()],
                },
            )]
            .into(),
            HashMap::new(),
        );
        let project = |p: [f64; 3]| [p[0], p[1], 0.5];
        let entity = |_: u128| None;
        let view = View {
            gui_width: 427,
            gui_height: 240,
            camera_uuid: 9,
            camera_entity_pos: [0.5, 64.0, 0.5],
            camera_pos: [0.5, 65.62, 0.5],
            yaw: 0.0,
            pitch: 0.0,
            fov: 70.0,
            project: &project,
            entity: &entity,
            styles: &styles,
            tick: 0,
        };
        let mut w = ClientWaypoints::default();
        // Yaw 0 looks towards +z: a waypoint straight ahead sits in the middle.
        w.apply(&tracked(
            waypoint_operation::TRACK,
            1,
            WaypointTarget::Block([0, 65, 40]),
        ));
        // One behind the camera and the camera's own waypoint are not drawn.
        w.apply(&tracked(
            waypoint_operation::TRACK,
            2,
            WaypointTarget::Block([0, 65, -40]),
        ));
        w.apply(&tracked(
            waypoint_operation::TRACK,
            9,
            WaypointTarget::Block([0, 65, 40]),
        ));
        let draws = w.draw(&view);
        assert_eq!(draws.len(), 1);
        let GuiDraw::Sprite { x, y, .. } = draws[0] else {
            panic!("{draws:?}");
        };
        assert_eq!((x, y), (209, 209));
        // 30 degrees to the right: floor(30 * 173 / 120) = 43 pixels.
        let east = ClientWaypoints {
            waypoints: vec![tracked(
                waypoint_operation::TRACK,
                3,
                WaypointTarget::Azimuth(30f32.to_radians()),
            )],
        };
        let GuiDraw::Sprite { x, .. } = east.draw(&view)[0] else {
            panic!();
        };
        assert_eq!(x, 209 + 43);
    }

    #[test]
    fn contextual_bar_priorities() {
        use ContextualBar as B;
        assert_eq!(B::next(true, false, false, true, false), B::Locator);
        assert_eq!(B::next(true, false, false, true, true), B::Experience);
        assert_eq!(B::next(true, true, true, true, true), B::JumpableVehicle);
        assert_eq!(B::next(false, false, false, true, true), B::Experience);
        assert_eq!(B::next(false, false, false, false, false), B::Empty);
        // Creative players see the bar too.
        assert_eq!(B::next(true, false, false, false, false), B::Locator);
    }
}
