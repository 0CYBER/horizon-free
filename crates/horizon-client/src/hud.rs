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
#[derive(Clone, Copy)]
struct Glyph {
    rect: Rect,
    advance: f32,
}
pub struct HudAtlas {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub sprites: HashMap<String, Rect>,
    glyphs: HashMap<char, Glyph>,
    white: Rect,
}
fn decode(path: &Path) -> Result<(u32, u32, Vec<u8>)> {
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
    let Ok(entries) = fs::read_dir(dir) else { return };
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
    pub fn build(root: &Path) -> Result<Self> {
        let textures = root.join("assets/minecraft/textures");
        let mut files = Vec::new();
        walk(&textures.join("gui"), &mut files);
        walk(&textures.join("item"), &mut files);
        walk(&textures.join("block"), &mut files);
        walk(&textures.join("font"), &mut files);
        walk(&textures.join("environment"), &mut files);
        walk(&textures.join("misc"), &mut files);
        walk(&textures.join("entity"), &mut files);
        walk(&textures.join("particle"), &mut files);
        files.retain(|p| {
            let s = p.to_string_lossy();
            !s.contains("title/background") && !s.contains("realms") && !s.contains("presets")
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
            (w <= 512 && h <= 512).then_some((name, w, h, pixels))
        };
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get());
        let chunk = files.len().div_ceil(threads).max(1);
        let mut images: Vec<_> = std::thread::scope(|scope| {
            let handles: Vec<_> = files
                .chunks(chunk)
                .map(|part| scope.spawn(move || part.iter().filter_map(decode_one).collect::<Vec<_>>()))
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        });
        images.push(("horizon/white".into(), 4, 4, vec![255; 64]));
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
        let height = (y + row + 1).next_power_of_two();
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
        let mut atlas = Self {
            pixels,
            width,
            height,
            sprites,
            glyphs: HashMap::new(),
            white,
        };
        atlas.load_font(root)?;
        Ok(atlas)
    }
    fn load_font(&mut self, root: &Path) -> Result<()> {
        let text = fs::read_to_string(root.join("assets/minecraft/font/include/default.json"))?;
        let value: serde_json::Value = serde_json::from_str(&text)?;
        for provider in value["providers"].as_array().into_iter().flatten() {
            if provider["type"] != "bitmap" {
                continue;
            }
            let file = provider["file"].as_str().unwrap_or("");
            let name = file
                .trim_start_matches("minecraft:")
                .trim_end_matches(".png")
                .to_owned();
            let Some(rect) = self.sprites.get(&name).copied() else {
                continue;
            };
            let rows: Vec<Vec<char>> = provider["chars"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| r.as_str().map(|s| s.chars().collect()))
                .collect();
            if rows.is_empty() {
                continue;
            }
            let columns = rows[0].len() as u32;
            let cell_w = rect.w / columns;
            let cell_h = rect.h / rows.len() as u32;
            let height = provider["height"].as_f64().unwrap_or(8.0) as f32;
            let scale = height / cell_h as f32;
            let (ax, ay) = (
                (rect.u0 * self.width as f32) as u32,
                (rect.v0 * self.height as f32) as u32,
            );
            for (r, row) in rows.iter().enumerate() {
                for (c, ch) in row.iter().enumerate() {
                    if *ch == '\0' || self.glyphs.contains_key(ch) {
                        continue;
                    }
                    let (cx, cy) = (ax + c as u32 * cell_w, ay + r as u32 * cell_h);
                    // Glyph width: rightmost column containing an opaque pixel.
                    let mut used = 0;
                    for x in (0..cell_w).rev() {
                        if (0..cell_h).any(|y| {
                            self.pixels[(((cy + y) * self.width + cx + x) * 4 + 3) as usize] != 0
                        }) {
                            used = x + 1;
                            break;
                        }
                    }
                    self.glyphs.insert(
                        *ch,
                        Glyph {
                            rect: Rect {
                                u0: cx as f32 / self.width as f32,
                                v0: cy as f32 / self.height as f32,
                                u1: (cx + cell_w) as f32 / self.width as f32,
                                v1: (cy + cell_h) as f32 / self.height as f32,
                                w: cell_w,
                                h: cell_h,
                            },
                            advance: (used as f32 * scale + 0.5).floor() + 1.0,
                        },
                    );
                }
            }
        }
        self.glyphs.insert(
            ' ',
            Glyph {
                rect: self.white,
                advance: 4.0,
            },
        );
        Ok(())
    }
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HudVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: u32,
}
/// One frame of overlay geometry in GUI pixels.
pub struct HudBatch {
    pub vertices: Vec<HudVertex>,
    pub scale: f32,
    pub width: f32,
    pub height: f32,
}
pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    u32::from(a) << 24 | u32::from(b) << 16 | u32::from(g) << 8 | u32::from(r)
}
impl HudBatch {
    /// Vanilla's automatic GUI scale: the largest scale keeping >= 320x240 GUI pixels.
    pub fn new(window: [u32; 2]) -> Self {
        let mut scale = 1;
        while window[0] / (scale + 1) >= 320 && window[1] / (scale + 1) >= 240 {
            scale += 1;
        }
        Self {
            vertices: Vec::new(),
            scale: scale as f32,
            width: (window[0] as f32 / scale as f32).ceil(),
            height: (window[1] as f32 / scale as f32).ceil(),
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
    pub fn sprite_scaled(&mut self, atlas: &HudAtlas, name: &str, x: f32, y: f32, w: f32, h: f32, color: u32) {
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
        text.chars()
            .map(|c| atlas.glyphs.get(&c).map_or(6.0, |g| g.advance))
            .sum()
    }
    pub fn text(&mut self, atlas: &HudAtlas, text: &str, x: f32, y: f32, color: u32, shadow: bool) -> f32 {
        if shadow {
            let c = color.to_le_bytes();
            let dark = rgba(c[0] / 4, c[1] / 4, c[2] / 4, c[3]);
            self.text(atlas, text, x + 1.0, y + 1.0, dark, false);
        }
        let mut cursor = x;
        for ch in text.chars() {
            let Some(g) = atlas.glyphs.get(&ch).copied() else {
                cursor += 6.0;
                continue;
            };
            if ch != ' ' {
                self.rect_uv(cursor, y, 8.0, 8.0, g.rect, color);
            }
            cursor += g.advance;
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
