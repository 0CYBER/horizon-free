//! 2D overlay: one atlas holding GUI sprites, the bitmap font, item and block
//! textures, and a batch of textured quads in GUI pixels (vanilla's automatic
//! GUI scale is applied when drawing).
use crate::Result;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};
use wgpu::util::DeviceExt;

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
    pub w: u32,
    pub h: u32,
}
pub struct HudAtlas {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub sprites: HashMap<String, Rect>,
    /// The default font (bitmap sheets in this atlas, unifont rasterized on demand).
    pub font: crate::font::Font,
    white: Rect,
}
/// Atlas rows reserved for glyphs rasterized at run time.
pub const GLYPH_ROWS: u32 = 512;
/// 128x128 atlas slots for map textures.
pub const MAP_SLOTS: usize = 16;
pub fn decode(path: &Path) -> Result<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::BufReader::new(fs::File::open(path)?));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut bytes = vec![0; reader.output_buffer_size().ok_or("image too large")?];
    let info = reader.next_frame(&mut bytes)?;
    let channels = info.color_type.samples();
    let mut rgba = Vec::with_capacity((info.width * info.height * 4) as usize);
    for p in bytes[..info.buffer_size()].chunks_exact(channels) {
        rgba.extend(match channels {
            4 => [p[0], p[1], p[2], p[3]],
            3 => [p[0], p[1], p[2], 255],
            2 => [p[0], p[0], p[0], p[1]],
            _ => [p[0], p[0], p[0], 255],
        });
    }
    Ok((info.width, info.height, rgba))
}
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "png") {
            out.push(path);
        }
    }
}
impl HudAtlas {
    /// A font glyph's atlas cell (8x8 GUI pixels) and advance.
    pub fn glyph(&self, ch: char) -> Option<(Rect, f32)> {
        let g = self.font.glyph(ch);
        g.rect.map(|rect| (rect, g.advance))
    }
    /// A font glyph's atlas cell (none for blank glyphs such as the space) and advance.
    pub fn glyph_cell(&self, ch: char) -> (Option<Rect>, f32) {
        let g = self.font.glyph(ch);
        (g.rect, g.advance)
    }
    pub fn build(root: &Path) -> Result<Self> {
        let textures = root.join("assets/minecraft/textures");
        let mut files = Vec::new();
        walk(&textures.join("gui"), &mut files);
        walk(&textures.join("item"), &mut files);
        walk(&textures.join("block"), &mut files);
        walk(&textures.join("font"), &mut files);
        walk(&textures.join("environment"), &mut files);
        walk(&textures.join("misc"), &mut files);
        walk(&textures.join("painting"), &mut files);
        walk(&textures.join("entity"), &mut files);
        walk(&textures.join("particle"), &mut files);
        walk(&textures.join("mob_effect"), &mut files);
        walk(&textures.join("map"), &mut files);
        files.retain(|p| {
            let s = p.to_string_lossy();
            (!s.contains("title/background") || s.contains("panorama_"))
                && !s.contains("realms")
                && !s.contains("presets")
        });
        let decode_one = |path: &PathBuf| -> Option<(String, u32, u32, Vec<u8>)> {
            let name = path
                .strip_prefix(&textures)
                .ok()?
                .with_extension("")
                .to_string_lossy()
                .replace('\\', "/");
            let (w, mut h, pixels) = decode(path).ok()?;
            let is_frame_strip = (name.starts_with("block/") || name.starts_with("item/")) && h > w;
            let pixels = if is_frame_strip {
                h = w;
                pixels[..(w * w * 4) as usize].to_vec()
            } else {
                pixels
            };
            // Title panorama faces are downsampled to 256 for the shared atlas.
            if name.contains("panorama_") && w >= 512 && w == h {
                let step = (w / 256) as usize;
                let mut small = Vec::with_capacity(256 * 256 * 4);
                for y in 0..256usize {
                    for x in 0..256usize {
                        let i = ((y * step) * w as usize + x * step) * 4;
                        small.extend_from_slice(&pixels[i..i + 4]);
                    }
                }
                return Some((name, 256, 256, small));
            }
            (w <= 1024 && (h <= 512 || name.starts_with("font/"))).then_some((name, w, h, pixels))
        };
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get());
        let chunk = files.len().div_ceil(threads).max(1);
        let mut images: Vec<_> = std::thread::scope(|scope| {
            let handles: Vec<_> = files
                .chunks(chunk)
                .map(|part| {
                    scope.spawn(move || part.iter().filter_map(decode_one).collect::<Vec<_>>())
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap_or_default())
                .collect()
        });
        images.push(("horizon/white".into(), 4, 4, vec![255; 64]));
        // Map textures (game/maps.rs) are written into these slots at run time.
        for i in 0..MAP_SLOTS {
            images.push((format!("horizon/map{i}"), 128, 128, vec![0; 128 * 128 * 4]));
        }
        images.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)));
        let width = 4096u32;
        let mut placements = Vec::with_capacity(images.len());
        let (mut x, mut y, mut row) = (0u32, 0u32, 0u32);
        for (_, w, h, _) in &images {
            if x + w + 1 > width {
                x = 0;
                y += row + 1;
                row = 0;
            }
            placements.push((x, y));
            x += w + 1;
            row = row.max(*h);
        }
        let glyph_region = y + row + 1;
        let height = (glyph_region + GLYPH_ROWS).next_power_of_two();
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let mut sprites = HashMap::new();
        for ((name, w, h, data), (px, py)) in images.iter().zip(placements) {
            for row in 0..*h {
                let src = &data[(row * w * 4) as usize..((row + 1) * w * 4) as usize];
                let dst = (((py + row) * width + px) * 4) as usize;
                pixels[dst..dst + (w * 4) as usize].copy_from_slice(src);
            }
            sprites.insert(
                name.clone(),
                Rect {
                    u0: px as f32 / width as f32,
                    v0: py as f32 / height as f32,
                    u1: (px + w) as f32 / width as f32,
                    v1: (py + h) as f32 / height as f32,
                    w: *w,
                    h: *h,
                },
            );
        }
        let white = sprites["horizon/white"];
        let font = crate::font::Font::load(
            root,
            &sprites,
            &pixels,
            (width, height),
            (glyph_region, height - glyph_region),
            white,
        );
        Ok(Self {
            pixels,
            width,
            height,
            sprites,
            font,
            white,
        })
    }
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HudVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: u32,
}
/// The GUI Scale option (0 = auto).
pub static GUI_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// One frame of overlay geometry in GUI pixels.
pub struct HudBatch {
    pub vertices: Vec<HudVertex>,
    pub scale: f32,
    pub width: f32,
    pub height: f32,
    /// Glyph pixels to upload into the atlas before drawing.
    pub patches: Vec<crate::font::Patch>,
}
pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    u32::from(a) << 24 | u32::from(b) << 16 | u32::from(g) << 8 | u32::from(r)
}
impl HudBatch {
    /// Window.calculateScale: the largest scale keeping >= 320x240 GUI pixels,
    /// capped by the GUI Scale option (0 = auto).
    pub fn new(window: [u32; 2]) -> Self {
        let limit = GUI_SCALE.load(std::sync::atomic::Ordering::Relaxed);
        let mut scale = 1;
        while (limit == 0 || scale < limit)
            && window[0] / (scale + 1) >= 320
            && window[1] / (scale + 1) >= 240
        {
            scale += 1;
        }
        Self {
            vertices: Vec::new(),
            scale: scale as f32,
            width: (window[0] as f32 / scale as f32).ceil(),
            height: (window[1] as f32 / scale as f32).ceil(),
            patches: Vec::new(),
        }
    }
    pub fn quad(&mut self, p: [[f32; 2]; 4], uv: [[f32; 2]; 4], color: u32) {
        for i in [0, 1, 2, 0, 2, 3] {
            self.vertices.push(HudVertex {
                pos: p[i],
                uv: uv[i],
                color,
            });
        }
    }
    pub fn rect_uv(&mut self, x: f32, y: f32, w: f32, h: f32, r: Rect, color: u32) {
        self.quad(
            [[x, y], [x + w, y], [x + w, y + h], [x, y + h]],
            [[r.u0, r.v0], [r.u1, r.v0], [r.u1, r.v1], [r.u0, r.v1]],
            color,
        );
    }
    pub fn sprite(&mut self, atlas: &HudAtlas, name: &str, x: f32, y: f32) {
        if let Some(r) = atlas.sprites.get(name).copied() {
            self.rect_uv(x, y, r.w as f32, r.h as f32, r, u32::MAX);
        }
    }
    pub fn sprite_scaled(
        &mut self,
        atlas: &HudAtlas,
        name: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: u32,
    ) {
        if let Some(r) = atlas.sprites.get(name).copied() {
            self.rect_uv(x, y, w, h, r, color);
        }
    }
    /// Draw part of a sprite, `fraction` of its width from the left.
    pub fn sprite_part(&mut self, atlas: &HudAtlas, name: &str, x: f32, y: f32, fraction: f32) {
        if let Some(mut r) = atlas.sprites.get(name).copied() {
            let w = r.w as f32 * fraction;
            r.u1 = r.u0 + (r.u1 - r.u0) * fraction;
            self.rect_uv(x, y, w, r.h as f32, r, u32::MAX);
        }
    }
    /// Clip the quads drawn since `start` to `[x0, x1) x [y0, y1)` (the
    /// GUI scissor): axis-aligned quads are cut with their UVs, others are
    /// kept or dropped by their centre.
    /// Scale the vertices pushed since `start` about `(cx, cy)` (a GUI pose
    /// matrix of translate, scale, translate back).
    pub fn scale_since(&mut self, start: usize, cx: f32, cy: f32, sx: f32, sy: f32) {
        for v in &mut self.vertices[start..] {
            v.pos[0] = cx + (v.pos[0] - cx) * sx;
            v.pos[1] = cy + (v.pos[1] - cy) * sy;
        }
    }
    pub fn clip_since(&mut self, start: usize, x0: f32, y0: f32, x1: f32, y1: f32) {
        let quads: Vec<[HudVertex; 6]> = self.vertices[start..].as_chunks::<6>().0.to_vec();
        self.vertices.truncate(start);
        for q in quads {
            let corners = [q[0], q[1], q[2], q[5]];
            let (mut x_lo, mut x_hi) = (f32::INFINITY, f32::NEG_INFINITY);
            let (mut y_lo, mut y_hi) = (f32::INFINITY, f32::NEG_INFINITY);
            for v in &corners {
                x_lo = x_lo.min(v.pos[0]);
                x_hi = x_hi.max(v.pos[0]);
                y_lo = y_lo.min(v.pos[1]);
                y_hi = y_hi.max(v.pos[1]);
            }
            // An axis-aligned rectangle, its corners in any order.
            let find = |x: f32, y: f32| corners.iter().find(|v| v.pos == [x, y]).copied();
            let rect = (
                find(x_lo, y_lo),
                find(x_hi, y_lo),
                find(x_hi, y_hi),
                find(x_lo, y_hi),
            );
            let (Some(tl), Some(tr), Some(br), Some(bl)) = rect else {
                let cx = corners.iter().map(|v| v.pos[0]).sum::<f32>() / 4.0;
                let cy = corners.iter().map(|v| v.pos[1]).sum::<f32>() / 4.0;
                if cx >= x0 && cx < x1 && cy >= y0 && cy < y1 {
                    self.vertices.extend_from_slice(&q);
                }
                continue;
            };
            let (cx0, cy0) = (x_lo.max(x0), y_lo.max(y0));
            let (cx1, cy1) = (x_hi.min(x1), y_hi.min(y1));
            if cx0 >= cx1 || cy0 >= cy1 {
                continue;
            }
            let t = |p: f32, lo: f32, hi: f32| if hi == lo { 0.0 } else { (p - lo) / (hi - lo) };
            let uv = |x: f32, y: f32| {
                let (s, r) = (t(x, x_lo, x_hi), t(y, y_lo, y_hi));
                std::array::from_fn(|i| {
                    let top = tl.uv[i] + (tr.uv[i] - tl.uv[i]) * s;
                    let bottom = bl.uv[i] + (br.uv[i] - bl.uv[i]) * s;
                    top + (bottom - top) * r
                })
            };
            self.quad(
                [[cx0, cy0], [cx1, cy0], [cx1, cy1], [cx0, cy1]],
                [uv(cx0, cy0), uv(cx1, cy0), uv(cx1, cy1), uv(cx0, cy1)],
                tl.color,
            );
        }
    }
    /// `blit` of the top-left `w x h` texels of a `tw x th` texture.
    pub fn sprite_region(
        &mut self,
        atlas: &HudAtlas,
        name: &str,
        [x, y, w, h]: [f32; 4],
        tw: f32,
        th: f32,
    ) {
        if let Some(mut r) = atlas.sprites.get(name).copied() {
            r.u1 = r.u0 + (r.u1 - r.u0) * w / tw;
            r.v1 = r.v0 + (r.v1 - r.v0) * h / th;
            self.rect_uv(x, y, w, h, r, u32::MAX);
        }
    }
    pub fn fill(&mut self, atlas: &HudAtlas, x: f32, y: f32, w: f32, h: f32, color: u32) {
        let mut r = atlas.white;
        let (cu, cv) = ((r.u0 + r.u1) / 2.0, (r.v0 + r.v1) / 2.0);
        r.u0 = cu;
        r.u1 = cu;
        r.v0 = cv;
        r.v1 = cv;
        self.rect_uv(x, y, w, h, r, color);
    }
    pub fn text_width(atlas: &HudAtlas, text: &str) -> f32 {
        atlas.font.width(text)
    }
    /// Draw plain text (legacy `§` codes are honoured); returns the advance.
    pub fn text(
        &mut self,
        atlas: &HudAtlas,
        text: &str,
        x: f32,
        y: f32,
        color: u32,
        shadow: bool,
    ) -> f32 {
        let t = if text.contains('§') {
            crate::text::Text::legacy(text)
        } else {
            crate::text::Text::literal(text)
        };
        self.text_styled(atlas, &t, x, y, color, shadow)
    }
    /// `text_styled` under a pose: GUI positions scaled by `scale`, then
    /// moved to `origin` (Matrix3x2fStack translate + scale).
    pub fn text_posed(
        &mut self,
        atlas: &HudAtlas,
        text: &crate::text::Text,
        at: [f32; 2],
        color: u32,
        scale: f32,
        origin: [f32; 2],
    ) -> f32 {
        let start = self.vertices.len();
        let advance = self.text_styled(atlas, text, at[0], at[1], color, true);
        for v in &mut self.vertices[start..] {
            v.pos = [origin[0] + v.pos[0] * scale, origin[1] + v.pos[1] * scale];
        }
        advance
    }
    /// Font.drawInternal: styled glyphs, shadows first, with bold, italic,
    /// underline and strikethrough. `color` is the default colour and alpha.
    pub fn text_styled(
        &mut self,
        atlas: &HudAtlas,
        text: &crate::text::Text,
        x: f32,
        y: f32,
        color: u32,
        shadow: bool,
    ) -> f32 {
        let font = &atlas.font;
        let alpha = (color >> 24) as u8;
        let mut glyphs = Vec::new();
        let mut cursor = x;
        for (i, (ch, style)) in text.chars().enumerate() {
            let g = font.glyph(ch);
            let c = style.color.map_or(color, |rgb| {
                rgba((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, alpha)
            });
            let bold = style.is_bold();
            let advance = g.advance + if bold { g.offset } else { 0.0 };
            glyphs.push((
                cursor,
                g,
                c,
                bold,
                style.is_italic(),
                style.underlined == Some(true),
                style.strikethrough == Some(true),
                i == 0,
                advance,
            ));
            cursor += advance;
        }
        self.patches.extend(font.take_patches());
        let white = font.white();
        let passes: &[bool] = if shadow { &[true, false] } else { &[false] };
        for &is_shadow in passes {
            for &(gx, g, c, bold, italic, underline, strike, first, advance) in &glyphs {
                let (dx, col) = if is_shadow {
                    let b = c.to_le_bytes();
                    (
                        g.offset,
                        rgba(
                            (f32::from(b[0]) * 0.25) as u8,
                            (f32::from(b[1]) * 0.25) as u8,
                            (f32::from(b[2]) * 0.25) as u8,
                            b[3],
                        ),
                    )
                } else {
                    (0.0, c)
                };
                let (px, py) = (gx + dx, y + dx);
                let shear = |v: f32| if italic { 1.0 - 0.25 * v } else { 0.0 };
                let thick = if bold { 0.1 } else { 0.0 };
                let draw = |batch: &mut Self, ox: f32| match g.rect {
                    Some(r) => {
                        let (x0, x1, y0, y1) = (
                            px + ox + g.left,
                            px + ox + g.right,
                            py + g.top,
                            py + g.bottom,
                        );
                        batch.quad(
                            [
                                [x0 + shear(g.top) - thick, y0 - thick],
                                [x1 + shear(g.top) + thick, y0 - thick],
                                [x1 + shear(g.bottom) + thick, y1 + thick],
                                [x0 + shear(g.bottom) - thick, y1 + thick],
                            ],
                            [[r.u0, r.v0], [r.u1, r.v0], [r.u1, r.v1], [r.u0, r.v1]],
                            col,
                        );
                    }
                    None if g.right > 0.0 => {
                        // SpecialGlyphs.MISSING: a 5x8 outline.
                        let (bx, by) = (px + ox, py);
                        for (fx, fy, fw, fh) in [
                            (0.0, 0.0, 5.0, 1.0),
                            (0.0, 7.0, 5.0, 1.0),
                            (0.0, 1.0, 1.0, 6.0),
                            (4.0, 1.0, 1.0, 6.0),
                        ] {
                            batch.rect_uv(bx + fx, by + fy, fw, fh, white, col);
                        }
                    }
                    None => {}
                };
                draw(self, 0.0);
                if bold {
                    draw(self, g.offset);
                }
                let x0 = if first { px - 1.0 } else { px };
                if strike {
                    self.rect_uv(x0, py + 3.5, px + advance - x0, 1.0, white, col);
                }
                if underline {
                    self.rect_uv(x0, py + 8.0, px + advance - x0, 1.0, white, col);
                }
            }
        }
        cursor - x
    }
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct HudUniform {
    size: [f32; 4],
}
pub struct HudRenderer {
    /// The shared atlas texture, for run-time glyph uploads.
    pub atlas_texture: Option<wgpu::Texture>,
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    buffer: wgpu::Buffer,
    capacity: u64,
}
impl HudRenderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        view: &wgpu::TextureView,
    ) -> Result<Self> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon HUD"),
            source: wgpu::ShaderSource::Wgsl(include_str!("hud.wgsl").into()),
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("HUD"),
            ..Default::default()
        });
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("HUD size"),
            contents: bytemuck::bytes_of(&HudUniform { size: [1.0; 4] }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("HUD bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("HUD resources"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("HUD layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let attributes = wgpu::vertex_attr_array![0=>Float32x2,1=>Float32x2,2=>Unorm8x4];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("HUD"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<HudVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let capacity = 1 << 16;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("HUD vertices"),
            size: capacity * std::mem::size_of::<HudVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            atlas_texture: None,
            pipeline,
            group,
            uniform,
            buffer,
            capacity,
        })
    }
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        batch: &HudBatch,
        size: [u32; 2],
    ) {
        if let Some(texture) = &self.atlas_texture {
            for p in &batch.patches {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: p.x,
                            y: p.y,
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &p.rgba,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(p.width * 4),
                        rows_per_image: None,
                    },
                    wgpu::Extent3d {
                        width: p.width,
                        height: p.height,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
        if batch.vertices.is_empty() {
            return;
        }
        let count = batch.vertices.len() as u64;
        if count > self.capacity {
            self.capacity = count.next_power_of_two();
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("HUD vertices"),
                size: self.capacity * std::mem::size_of::<HudVertex>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&batch.vertices));
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&HudUniform {
                size: [size[0] as f32, size[1] as f32, batch.scale, 0.0],
            }),
        );
        let attachments = [Some(wgpu::RenderPassColorAttachment {
            view: target,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("HUD"),
            color_attachments: &attachments,
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        pass.draw(0..count as u32, 0..1);
    }
}
