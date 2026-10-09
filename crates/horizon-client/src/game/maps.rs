//! Maps (`EmptyMapItem`, `MapItem`, `MapItemSavedData`, `MapRenderer`):
//! creating a map centred on the player's grid cell, revealing it while held
//! (`MapItem.update`: dominant map colour per pixel, height and water-depth
//! shading, the dithered edge), the player marker, `data/map_<id>.dat` and
//! `idcounts.dat`, the 128x128 texture in an atlas slot, the two-handed
//! first-person view and maps in item frames.
use super::Game;
use horizon_client::{
    blocks::{Fluid, MAP_COLORS},
    hud::{MAP_SLOTS, Rect},
    items::ItemStack,
    mesher::Vertex,
};
use horizon_world::nbt::Tag;
use std::collections::HashMap;

/// A decoration: sprite, map position (-128..127, half pixels) and rotation (0..15).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Decoration {
    pub sprite: &'static str,
    pub x: i8,
    pub y: i8,
    pub rot: u8,
}
pub(super) struct MapData {
    pub center: [i32; 2],
    pub scale: u8,
    pub dimension: String,
    pub locked: bool,
    pub tracking: bool,
    pub unlimited: bool,
    pub colors: Vec<u8>,
    pub decorations: Vec<Decoration>,
    /// HoldingPlayer.step for the column sweep.
    step: i32,
    /// Changed since the last save / texture upload.
    dirty: bool,
    texture_dirty: bool,
}
impl MapData {
    /// `MapItemSavedData.createFresh`: centred on the scale's grid cell.
    fn new(x: f64, z: f64, scale: u8, dimension: &str) -> Self {
        let size = 128 * (1 << scale);
        let cell = |v: f64| (v.floor() as i32 + 64).div_euclid(size) * size + size / 2 - 64;
        Self {
            center: [cell(x), cell(z)],
            scale,
            dimension: dimension.to_owned(),
            locked: false,
            tracking: true,
            unlimited: false,
            colors: vec![0; 128 * 128],
            decorations: Vec::new(),
            step: 0,
            dirty: true,
            texture_dirty: true,
        }
    }
    fn update_color(&mut self, x: i32, y: i32, color: u8) -> bool {
        let i = (x + y * 128) as usize;
        if self.colors[i] == color {
            return false;
        }
        self.colors[i] = color;
        self.dirty = true;
        self.texture_dirty = true;
        true
    }
    fn to_nbt(&self) -> Tag {
        let flag = |b: bool| Tag::Byte(i8::from(b));
        Tag::compound([
            (
                "data",
                Tag::compound([
                    ("dimension", Tag::String(self.dimension.as_str().into())),
                    ("xCenter", Tag::Int(self.center[0])),
                    ("zCenter", Tag::Int(self.center[1])),
                    ("scale", Tag::Byte(self.scale as i8)),
                    ("locked", flag(self.locked)),
                    ("trackingPosition", flag(self.tracking)),
                    ("unlimitedTracking", flag(self.unlimited)),
                    ("colors", Tag::ByteArray(self.colors.clone())),
                    ("banners", Tag::List(Vec::new())),
                    ("frames", Tag::List(Vec::new())),
                ]),
            ),
            ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
        ])
    }
    fn from_nbt(tag: &Tag) -> Option<Self> {
        let d = tag.get("data")?;
        let int = |k: &str| match d.get(k) {
            Some(Tag::Int(v)) => Some(*v),
            Some(Tag::Byte(v)) => Some(i32::from(*v)),
            _ => None,
        };
        let dimension = match d.get("dimension") {
            Some(Tag::String(s)) => s.to_string_lossy(),
            _ => "minecraft:overworld".to_owned(),
        };
        let mut colors = match d.get("colors") {
            Some(Tag::ByteArray(c)) => c.clone(),
            _ => Vec::new(),
        };
        colors.resize(128 * 128, 0);
        Some(Self {
            center: [int("xCenter")?, int("zCenter")?],
            scale: int("scale").unwrap_or(0).clamp(0, 4) as u8,
            dimension,
            locked: int("locked").unwrap_or(0) != 0,
            tracking: int("trackingPosition").unwrap_or(1) != 0,
            unlimited: int("unlimitedTracking").unwrap_or(0) != 0,
            colors,
            decorations: Vec::new(),
            step: 0,
            dirty: false,
            texture_dirty: true,
        })
    }
}
#[derive(Default)]
pub(super) struct Maps {
    pub data: HashMap<i32, MapData>,
    /// The highest map id handed out (`idcounts.dat`), loaded on first use.
    last_id: Option<i32>,
    /// (map id, atlas slot), most recently shown last.
    slots: Vec<(i32, usize)>,
    missing: std::collections::HashSet<i32>,
}
/// `MapColor.getColorFromPackedId` as RGBA bytes (NONE is transparent).
fn packed_rgba(packed: u8) -> [u8; 4] {
    let id = usize::from(packed >> 2);
    if id == 0 || id >= MAP_COLORS.len() {
        return [0; 4];
    }
    let modifier = [180u32, 220, 255, 135][usize::from(packed & 3)];
    let col = MAP_COLORS[id];
    let ch = |s: u32| ((col >> s & 255) * modifier / 255) as u8;
    [ch(16), ch(8), ch(0), 255]
}
pub(super) fn map_id(stack: &ItemStack) -> Option<i32> {
    if &*stack.item != "minecraft:filled_map" {
        return None;
    }
    stack.data().and_then(|d| d.map_id)
}
impl Game<'_> {
    fn dimension_id(&self) -> &'static str {
        match self.dim {
            super::Dim::Nether => "minecraft:the_nether",
            super::Dim::End => "minecraft:the_end",
            _ => "minecraft:overworld",
        }
    }
    fn map_path(&self, name: &str) -> std::path::PathBuf {
        self.world_root.join("data").join(name)
    }
    /// The map's data, loaded from `data/map_<id>.dat` on first use.
    pub(super) fn map(&mut self, id: i32) -> Option<&mut MapData> {
        if !self.maps.data.contains_key(&id) && !self.maps.missing.contains(&id) {
            let loaded = horizon_client::save::read_dat(&self.map_path(&format!("map_{id}.dat")))
                .and_then(|t| MapData::from_nbt(&t));
            match loaded {
                Some(m) => {
                    self.maps.data.insert(id, m);
                }
                None => {
                    self.maps.missing.insert(id);
                }
            }
        }
        self.maps.data.get_mut(&id)
    }
    fn next_map_id(&mut self) -> i32 {
        let last = *self.maps.last_id.get_or_insert_with(|| {
            horizon_client::save::read_dat(&self.world_root.join("data/idcounts.dat"))
                .and_then(|t| match t.get("data")?.get("map")? {
                    Tag::Int(v) => Some(*v),
                    _ => None,
                })
                .unwrap_or(-1)
        });
        self.maps.last_id = Some(last + 1);
        last + 1
    }
    /// `EmptyMapItem.use`: a new map centred around the player.
    pub(super) fn use_empty_map(&mut self, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:map" {
            return false;
        }
        let Some(p) = self.player.as_ref().map(|p| p.position) else {
            return false;
        };
        let id = self.next_map_id();
        let data = MapData::new(p.x, p.z, 0, self.dimension_id());
        self.maps.data.insert(id, data);
        let mut filled = ItemStack::new("minecraft:filled_map", 1);
        filled.data_mut().map_id = Some(id);
        if self.creative {
            self.give_or_drop_stack(filled);
        } else if stack.count == 1 {
            self.inventory[self.selected] = Some(filled);
        } else {
            self.consume_held();
            self.give_or_drop_stack(filled);
        }
        let at = [p.x, p.y, p.z];
        self.audio.play(
            "minecraft:ui.cartography_table.take_result",
            Some(at),
            1.0,
            1.0,
        );
        self.swing_arm();
        true
    }
    fn give_or_drop_stack(&mut self, stack: ItemStack) {
        if let Some(rest) = self.give(stack)
            && let Some(eye) = self.eye_position()
        {
            self.spawn_item(rest, [eye[0], eye[1] - 0.3, eye[2]], [0.0, 0.1, 0.0], 40);
        }
    }
    /// Held maps update; every carried map tracks the player marker.
    pub(super) fn tick_maps(&mut self) {
        let Some(p) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        let held: Vec<i32> = [Some(self.selected), Some(super::shield::OFFHAND_SLOT)]
            .into_iter()
            .flatten()
            .filter_map(|i| self.stack_in(i).and_then(map_id))
            .collect();
        let carried: Vec<i32> = self
            .inventory
            .iter()
            .chain(std::iter::once(&self.offhand))
            .flatten()
            .filter_map(map_id)
            .collect();
        let (yaw, dim) = (self.yaw, self.dimension_id());
        for id in carried {
            let Some(m) = self.map(id) else { continue };
            // tickCarriedBy: the player marker while tracking in this dimension.
            m.decorations.clear();
            if m.tracking && m.dimension == dim {
                let s = f64::from(1 << m.scale);
                let dx = ((p.x - f64::from(m.center[0])) / s) as f32;
                let dz = ((p.z - f64::from(m.center[1])) / s) as f32;
                let inside = (-63.0..=63.0).contains(&dx) && (-63.0..=63.0).contains(&dz);
                let clamp = |d: f32| {
                    if d <= -63.0 {
                        -128
                    } else if d >= 63.0 {
                        127
                    } else {
                        (d * 2.0 + 0.5) as i8
                    }
                };
                let y = f64::from(yaw);
                let adjusted = if y < 0.0 { y - 8.0 } else { y + 8.0 };
                let rot = ((adjusted * 16.0 / 360.0) as i32 & 15) as u8;
                let sprite = if inside {
                    Some("map/decorations/player")
                } else if dx.abs() < 320.0 && dz.abs() < 320.0 {
                    Some("map/decorations/player_off_map")
                } else if m.unlimited {
                    Some("map/decorations/player_off_limits")
                } else {
                    None
                };
                if let Some(sprite) = sprite {
                    m.decorations.push(Decoration {
                        sprite,
                        x: clamp(dx),
                        y: clamp(dz),
                        rot,
                    });
                }
            }
        }
        for id in held {
            self.update_map(id);
        }
    }
    /// `MapItem.update` for the player holding map `id`.
    fn update_map(&mut self, id: i32) {
        let dim = self.dimension_id();
        let Some(p) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        let nether = self.dim == super::Dim::Nether;
        let Some(m) = self.maps.data.get_mut(&id) else {
            return;
        };
        if m.locked || m.dimension != dim {
            return;
        }
        let scale = 1i32 << m.scale;
        let [cx, cz] = m.center;
        let px = (p.x - f64::from(cx)).floor() as i32 / scale + 64;
        let pz = (p.z - f64::from(cz)).floor() as i32 / scale + 64;
        let mut radius = 128 / scale;
        if nether {
            radius /= 2;
        }
        m.step += 1;
        let step = m.step;
        let mut changes: Vec<(i32, i32, u8)> = Vec::new();
        let mut consecutive = false;
        for img_x in px - radius + 1..px + radius {
            if (img_x & 15) != (step & 15) && !consecutive {
                continue;
            }
            consecutive = false;
            let mut previous_height = 0.0f64;
            for img_y in pz - radius - 1..pz + radius {
                if !(0..128).contains(&img_x) || !(-1..128).contains(&img_y) {
                    continue;
                }
                let d2 = (img_x - px).pow(2) + (img_y - pz).pow(2);
                let dither_black = d2 > (radius - 2) * (radius - 2);
                let min_x = (cx / scale + img_x - 64) * scale;
                let min_z = (cz / scale + img_y - 64) * scale;
                if !self.world.chunks.contains_key(&(min_x >> 4, min_z >> 4)) {
                    continue;
                }
                let mut counts: Vec<(u8, u32)> = Vec::new();
                let mut add = |c: u8, n: u32| match counts.iter_mut().find(|e| e.0 == c) {
                    Some(e) => e.1 += n,
                    None => counts.push((c, n)),
                };
                let mut water_depth = 0i32;
                let mut height = 0.0f64;
                if nether {
                    let mut noise = min_x.wrapping_add(min_z.wrapping_mul(231_871));
                    noise = noise
                        .wrapping_mul(noise)
                        .wrapping_mul(31_287_121)
                        .wrapping_add(noise.wrapping_mul(11));
                    // Dirt (10) or stone (11) noise under a ceiling.
                    if noise >> 20 & 1 == 0 {
                        add(10, 10);
                    } else {
                        add(11, 100);
                    }
                    height = 100.0;
                } else {
                    for dx in 0..scale {
                        for dz in 0..scale {
                            let (x, z) = (min_x + dx, min_z + dz);
                            let mut y = self.world_surface(x, z);
                            let min_y = super::MIN_Y;
                            let mut color;
                            if y <= min_y {
                                color = 11;
                            } else {
                                loop {
                                    y -= 1;
                                    color = self.res.facts.get(self.world.block(x, y, z)).map_color;
                                    if color != 0 || y <= min_y {
                                        break;
                                    }
                                }
                                let f = self.res.facts.get(self.world.block(x, y, z));
                                if y > min_y && f.fluid != Fluid::None {
                                    let mut solid = y - 1;
                                    loop {
                                        let below =
                                            self.res.facts.get(self.world.block(x, solid, z));
                                        solid -= 1;
                                        water_depth += 1;
                                        if solid <= min_y || below.fluid == Fluid::None {
                                            break;
                                        }
                                    }
                                    // getCorrectStateForFluidBlock: the fluid unless sturdy on top.
                                    if f.sturdy_full >> 1 & 1 == 0 {
                                        color = if f.fluid == Fluid::Lava { 4 } else { 12 };
                                    }
                                }
                            }
                            height += f64::from(y) / f64::from(scale * scale);
                            add(color, 1);
                        }
                    }
                }
                water_depth /= scale * scale;
                // Multisets.copyHighestCountFirst: the most common, first seen on ties.
                let best = counts
                    .iter()
                    .fold(None::<(u8, u32)>, |acc, &(c, n)| match acc {
                        Some((_, bn)) if bn >= n => acc,
                        _ => Some((c, n)),
                    })
                    .map_or(0, |e| e.0);
                let parity = f64::from((img_x + img_y) & 1);
                let brightness = if best == 12 {
                    let diff = f64::from(water_depth) * 0.1 + parity * 0.2;
                    if diff < 0.5 {
                        2
                    } else if diff > 0.9 {
                        0
                    } else {
                        1
                    }
                } else {
                    let diff = (height - previous_height) * 4.0 / f64::from(scale + 4)
                        + (parity - 0.5) * 0.4;
                    if diff > 0.6 {
                        2
                    } else if diff < -0.6 {
                        0
                    } else {
                        1
                    }
                };
                previous_height = height;
                if img_y >= 0 && d2 < radius * radius && (!dither_black || (img_x + img_y) & 1 != 0)
                {
                    changes.push((img_x, img_y, best << 2 | brightness));
                }
            }
            if let Some(m) = self.maps.data.get_mut(&id) {
                for (x, y, c) in changes.drain(..) {
                    consecutive |= m.update_color(x, y, c);
                }
            }
        }
    }
    /// Writes changed maps and the id counter (`MapItemSavedData`, `MapIndex`).
    pub(super) fn save_maps(&mut self) {
        for (id, m) in &mut self.maps.data {
            if m.dirty {
                let path = self.world_root.join("data").join(format!("map_{id}.dat"));
                if horizon_client::save::write_dat(&path, m.to_nbt()).is_ok() {
                    m.dirty = false;
                }
            }
        }
        if let Some(last) = self.maps.last_id {
            let tag = Tag::compound([
                ("data", Tag::compound([("map", Tag::Int(last))])),
                ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
            ]);
            let _ =
                horizon_client::save::write_dat(&self.world_root.join("data/idcounts.dat"), tag);
        }
    }
    /// The atlas slot showing map `id`, its texture uploaded when it changed.
    pub(super) fn map_texture(&mut self, id: i32) -> Option<Rect> {
        self.map(id)?;
        let slots = &mut self.maps.slots;
        if let Some(i) = slots.iter().position(|e| e.0 == id) {
            let e = slots.remove(i);
            slots.push(e);
        } else {
            // A free slot, else the least recently shown one.
            let slot = if slots.len() < MAP_SLOTS {
                slots.len()
            } else {
                slots.remove(0).1
            };
            slots.push((id, slot));
            if let Some(m) = self.maps.data.get_mut(&id) {
                m.texture_dirty = true;
            }
        }
        let slot = self.maps.slots.last()?.1;
        let rect = self
            .res
            .atlas
            .sprites
            .get(&format!("horizon/map{slot}"))
            .copied()?;
        let m = self.maps.data.get_mut(&id)?;
        if m.texture_dirty {
            m.texture_dirty = false;
            let rgba: Vec<u8> = m.colors.iter().flat_map(|&c| packed_rgba(c)).collect();
            let (w, h) = (self.res.atlas.width as f32, self.res.atlas.height as f32);
            self.map_patches.push(horizon_client::font::Patch {
                x: (rect.u0 * w).round() as u32,
                y: (rect.v0 * h).round() as u32,
                width: 128,
                height: 128,
                rgba,
            });
        }
        Some(rect)
    }
    /// `MapRenderer.render`: the map texture and its decorations under `m`
    /// (map pixels 0..128, y down), drawn at depth offsets toward the viewer.
    pub(super) fn map_vertices(
        &mut self,
        id: i32,
        m: glam::Mat4,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        let Some(rect) = self.map_texture(id) else {
            return;
        };
        let quad = |corners: [[f32; 3]; 4], uv: [[f32; 2]; 4], out: &mut Vec<Vertex>| {
            for (c, uv) in corners.into_iter().zip(uv) {
                out.push(Vertex {
                    pos: m.transform_point3(glam::Vec3::from_array(c)).to_array(),
                    uv,
                    layer: 0,
                    color: 0xFFFF_FFFF,
                    light,
                });
            }
        };
        let z = -0.01;
        quad(
            [
                [0.0, 128.0, z],
                [128.0, 128.0, z],
                [128.0, 0.0, z],
                [0.0, 0.0, z],
            ],
            [
                [rect.u0, rect.v1],
                [rect.u1, rect.v1],
                [rect.u1, rect.v0],
                [rect.u0, rect.v0],
            ],
            out,
        );
        let decorations = self
            .maps
            .data
            .get(&id)
            .map(|m| m.decorations.clone())
            .unwrap_or_default();
        for (i, d) in decorations.iter().enumerate() {
            let Some(s) = self.res.atlas.sprites.get(d.sprite).copied() else {
                continue;
            };
            let dm = glam::Mat4::from_translation(glam::Vec3::new(
                f32::from(d.x) / 2.0 + 64.0,
                f32::from(d.y) / 2.0 + 64.0,
                -0.02 - i as f32 * 0.001,
            )) * glam::Mat4::from_rotation_z(
                (f32::from(d.rot) * 360.0 / 16.0).to_radians(),
            ) * glam::Mat4::from_scale(glam::Vec3::splat(4.0))
                * glam::Mat4::from_translation(glam::Vec3::new(-0.125, 0.125, 0.0));
            let p = |x: f32, y: f32| dm.transform_point3(glam::Vec3::new(x, y, 0.0)).to_array();
            let mut q = |corners: [[f32; 3]; 4], uv: [[f32; 2]; 4]| {
                for (c, uv) in corners.into_iter().zip(uv) {
                    out.push(Vertex {
                        pos: m.transform_point3(glam::Vec3::from_array(c)).to_array(),
                        uv,
                        layer: 0,
                        color: 0xFFFF_FFFF,
                        light,
                    });
                }
            };
            q(
                [p(-1.0, 1.0), p(1.0, 1.0), p(1.0, -1.0), p(-1.0, -1.0)],
                [[s.u0, s.v0], [s.u1, s.v0], [s.u1, s.v1], [s.u0, s.v1]],
            );
        }
    }
    /// `renderTwoHandedMap`: the held map (main hand, empty off hand) in view space.
    pub(super) fn held_map_geometry(
        &mut self,
        to_world: glam::Mat4,
        attack: f32,
        inverse_height: f32,
        light: u32,
        out: &mut Vec<Vertex>,
    ) -> bool {
        let Some(id) = self.inventory[self.selected].as_ref().and_then(map_id) else {
            return false;
        };
        if self.offhand.is_some() {
            return false;
        }
        let deg = |d: f32| d.to_radians();
        let sqrt_attack = attack.sqrt();
        let y_swing = -0.2 * (attack * std::f32::consts::PI).sin();
        let z_swing = -0.4 * (sqrt_attack * std::f32::consts::PI).sin();
        // calculateMapTilt
        let tilt = (1.0 - self.pitch / 45.0 + 0.1).clamp(0.0, 1.0);
        let tilt = -(tilt * std::f32::consts::PI).cos() * 0.5 + 0.5;
        let base = to_world
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, -y_swing / 2.0, z_swing))
            * glam::Mat4::from_translation(glam::Vec3::new(
                0.0,
                0.04 + inverse_height * -1.2 + tilt * -0.5,
                -0.72,
            ))
            * glam::Mat4::from_rotation_x(deg(tilt * -85.0));
        let swing = (sqrt_attack * std::f32::consts::PI).sin();
        // renderMap: background (with 7-pixel border) then the map.
        let m = base
            * glam::Mat4::from_rotation_x(deg(swing * 20.0))
            * glam::Mat4::from_scale(glam::Vec3::splat(2.0))
            * glam::Mat4::from_rotation_y(deg(180.0))
            * glam::Mat4::from_rotation_z(deg(180.0))
            * glam::Mat4::from_scale(glam::Vec3::splat(0.38))
            * glam::Mat4::from_translation(glam::Vec3::new(-0.5, -0.5, 0.0))
            * glam::Mat4::from_scale(glam::Vec3::splat(0.0078125));
        let has_data = self.map(id).is_some();
        let background = if has_data {
            "map/map_background_checkerboard"
        } else {
            "map/map_background"
        };
        if let Some(r) = self.res.atlas.sprites.get(background).copied() {
            for (c, uv) in [
                ([-7.0, 135.0, 0.0], [r.u0, r.v1]),
                ([135.0, 135.0, 0.0], [r.u1, r.v1]),
                ([135.0, -7.0, 0.0], [r.u1, r.v0]),
                ([-7.0, -7.0, 0.0], [r.u0, r.v0]),
            ] {
                out.push(Vertex {
                    pos: m.transform_point3(glam::Vec3::from_array(c)).to_array(),
                    uv,
                    layer: 0,
                    color: 0xFFFF_FFFF,
                    light,
                });
            }
        }
        if has_data {
            self.map_vertices(id, m, light, out);
        }
        true
    }
}
