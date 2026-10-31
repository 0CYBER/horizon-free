//! Vanilla's post-effect chains (`PostChainConfig`, `PostChain`, `PostPass`,
//! `GameRenderer.preparePostEffects` / `applyPostEffects`): a `post_effect/*.json`
//! names internal targets and passes (a vertex and a fragment shader, inputs read
//! from targets, the target written, std140 uniform blocks) that run over the
//! frame drawn so far, before the HUD.
//!
//! The JSON is read at run time from the active resource root, so a resource pack
//! can replace or add chains. The shaders the passes name are Horizon's own
//! stand-ins (`post_chain.wgsl`) for the vanilla `core/screenquad`, `post/rotscale`,
//! `post/invert`, `post/blit`, `post/bits`, `post/color_convolve`, `post/box_blur`
//! and `post/spiderclip`; a chain that names any other shader, a texture or depth
//! input, or a target the frame has no equivalent of fails like a chain that does
//! not compile (`GameRenderer.failedPostEffects`), once, and is skipped.
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// `PostChain.MAIN_TARGET_ID`.
const MAIN: &str = "minecraft:main";
/// `PostPass`'s uniform buffers are 128 bytes.
const BLOCK_BYTES: usize = 128;

/// `ResourceLocation.withDefaultNamespace` on an id read from a chain.
fn normalise(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

/// One uniform of a block (`UniformValue`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Uniform {
    Int(i32),
    IVec3([i32; 3]),
    Float(f32),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    Matrix([f32; 16]),
}

impl Uniform {
    /// std140 (alignment, size) in bytes: `Std140SizeCalculator`.
    fn layout(&self) -> (usize, usize) {
        match self {
            Uniform::Int(_) | Uniform::Float(_) => (4, 4),
            Uniform::Vec2(_) => (8, 8),
            Uniform::Vec3(_) | Uniform::IVec3(_) => (16, 12),
            Uniform::Vec4(_) => (16, 16),
            Uniform::Matrix(_) => (16, 64),
        }
    }

    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Uniform::Int(v) => out.extend_from_slice(&v.to_le_bytes()),
            Uniform::IVec3(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
            Uniform::Float(v) => out.extend_from_slice(&v.to_le_bytes()),
            Uniform::Vec2(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
            Uniform::Vec3(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
            Uniform::Vec4(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
            Uniform::Matrix(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
        }
    }
}

/// A uniform block in std140 order, as `PostPass` builds it with `Std140Builder`.
pub fn pack_block(values: &[Uniform]) -> Result<[u8; BLOCK_BYTES], String> {
    let mut bytes = Vec::new();
    for value in values {
        let (align, _) = value.layout();
        while bytes.len() % align != 0 {
            bytes.push(0);
        }
        value.write(&mut bytes);
    }
    if bytes.len() > BLOCK_BYTES {
        return Err(format!("a uniform block of {} bytes", bytes.len()));
    }
    let mut block = [0u8; BLOCK_BYTES];
    block[..bytes.len()].copy_from_slice(&bytes);
    Ok(block)
}

/// `PostChainConfig.InternalTarget`.
#[derive(Clone, Debug, PartialEq)]
pub struct InternalTarget {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub persistent: bool,
    /// `clear_color` as straight RGBA.
    pub clear: [f32; 4],
}

/// `PostChainConfig.TargetInput` (the only input kind Horizon draws).
#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    pub sampler: String,
    pub target: String,
    pub bilinear: bool,
}

/// `PostChainConfig.Pass`.
#[derive(Clone, Debug, PartialEq)]
pub struct PassConfig {
    pub vertex: String,
    pub fragment: String,
    pub inputs: Vec<Input>,
    pub output: String,
    pub uniforms: BTreeMap<String, Vec<Uniform>>,
}

/// `PostChainConfig`.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub targets: BTreeMap<String, InternalTarget>,
    pub passes: Vec<PassConfig>,
}

fn floats<const N: usize>(v: &Value) -> Result<[f32; N], String> {
    let a = v
        .as_array()
        .filter(|a| a.len() == N)
        .ok_or("a vector of the wrong length")?;
    let mut out = [0.0; N];
    for (o, x) in out.iter_mut().zip(a) {
        *o = x.as_f64().ok_or("a vector entry that is not a number")? as f32;
    }
    Ok(out)
}

fn uniform(v: &Value) -> Result<Uniform, String> {
    let kind = v["type"].as_str().ok_or("a uniform without a type")?;
    let value = &v["value"];
    Ok(match kind {
        "int" => Uniform::Int(value.as_i64().ok_or("a bad int")? as i32),
        "float" => Uniform::Float(value.as_f64().ok_or("a bad float")? as f32),
        "vec2" => Uniform::Vec2(floats(value)?),
        "vec3" => Uniform::Vec3(floats(value)?),
        "vec4" => Uniform::Vec4(floats(value)?),
        "ivec3" => {
            let a = value
                .as_array()
                .filter(|a| a.len() == 3)
                .ok_or("a bad ivec3")?;
            let mut out = [0; 3];
            for (o, x) in out.iter_mut().zip(a) {
                *o = x.as_i64().ok_or("a bad ivec3")? as i32;
            }
            Uniform::IVec3(out)
        }
        "matrix4x4" => {
            let flat: Vec<f32> = match value.as_array() {
                Some(rows) if rows.first().is_some_and(Value::is_array) => rows
                    .iter()
                    .flat_map(|r| r.as_array().into_iter().flatten())
                    .filter_map(|x| x.as_f64().map(|x| x as f32))
                    .collect(),
                Some(flat) => flat
                    .iter()
                    .filter_map(|x| x.as_f64().map(|x| x as f32))
                    .collect(),
                None => Vec::new(),
            };
            Uniform::Matrix(flat.try_into().map_err(|_| "a bad matrix4x4")?)
        }
        other => return Err(format!("unknown uniform type {other}")),
    })
}

impl Config {
    /// `PostChainConfig.CODEC` over the file's text.
    pub fn parse(text: &str) -> Result<Self, String> {
        let json: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let mut targets = BTreeMap::new();
        if let Some(map) = json.get("targets").and_then(Value::as_object) {
            for (id, t) in map {
                let size = |key: &str| -> Result<Option<u32>, String> {
                    match t.get(key) {
                        None => Ok(None),
                        Some(v) => match v.as_u64() {
                            Some(n) if n > 0 => Ok(Some(n as u32)),
                            _ => Err(format!("target {id}: a bad {key}")),
                        },
                    }
                };
                let argb = t.get("clear_color").and_then(Value::as_u64).unwrap_or(0) as u32;
                let [a, r, g, b] = [24, 16, 8, 0].map(|s| ((argb >> s) & 255) as f32 / 255.0);
                targets.insert(
                    normalise(id),
                    InternalTarget {
                        width: size("width")?,
                        height: size("height")?,
                        persistent: t
                            .get("persistent")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        clear: [r, g, b, a],
                    },
                );
            }
        }
        let mut passes = Vec::new();
        for (n, p) in json
            .get("passes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let text = |key: &str| -> Result<String, String> {
                p.get(key)
                    .and_then(Value::as_str)
                    .map(normalise)
                    .ok_or(format!("pass {n}: no {key}"))
            };
            let mut inputs: Vec<Input> = Vec::new();
            for i in p
                .get("inputs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if i.get("location").is_some()
                    || i.get("use_depth_buffer").and_then(Value::as_bool) == Some(true)
                {
                    return Err(format!("pass {n}: texture and depth inputs are not drawn"));
                }
                let sampler = i["sampler_name"]
                    .as_str()
                    .ok_or(format!("pass {n}: an input without sampler_name"))?;
                if inputs.iter().any(|x| x.sampler == sampler) {
                    return Err(format!("Encountered repeated sampler name: {sampler}"));
                }
                inputs.push(Input {
                    sampler: sampler.to_owned(),
                    target: normalise(
                        i["target"]
                            .as_str()
                            .ok_or(format!("pass {n}: an input without a target"))?,
                    ),
                    bilinear: i.get("bilinear").and_then(Value::as_bool).unwrap_or(false),
                });
            }
            let mut uniforms = BTreeMap::new();
            if let Some(map) = p.get("uniforms").and_then(Value::as_object) {
                for (block, list) in map {
                    let list = list
                        .as_array()
                        .ok_or(format!("pass {n}: a bad uniform block"))?;
                    uniforms.insert(
                        block.clone(),
                        list.iter().map(uniform).collect::<Result<Vec<_>, _>>()?,
                    );
                }
            }
            passes.push(PassConfig {
                vertex: text("vertex_shader")?,
                fragment: text("fragment_shader")?,
                inputs,
                output: text("output")?,
                uniforms,
            });
        }
        Ok(Config { targets, passes })
    }

    /// `PostChain.getReferencedExternalTargets`.
    pub fn external_targets(&self) -> HashSet<&str> {
        let mut out = HashSet::new();
        for p in &self.passes {
            for id in p.inputs.iter().map(|i| &i.target).chain([&p.output]) {
                if !self.targets.contains_key(id) {
                    out.insert(id.as_str());
                }
            }
        }
        out
    }
}

/// A stand-in for a vanilla vertex shader: its entry point and uniform block.
const VERTEX: [(&str, &str, &str); 2] = [
    ("minecraft:core/screenquad", "vs_quad", ""),
    ("minecraft:post/rotscale", "vs_rotscale", "RotScaleConfig"),
];
/// A stand-in for a vanilla fragment shader: entry point, uniform block and the
/// samplers it reads (`In`, then `Blur`).
const FRAGMENT: [(&str, &str, &str, usize); 6] = [
    ("minecraft:post/invert", "fs_invert", "InvertConfig", 1),
    ("minecraft:post/blit", "fs_blit", "BlitConfig", 1),
    ("minecraft:post/bits", "fs_bits", "BitsConfig", 1),
    (
        "minecraft:post/color_convolve",
        "fs_color_convolve",
        "ColorConfig",
        1,
    ),
    ("minecraft:post/box_blur", "fs_box_blur", "BlurConfig", 1),
    (
        "minecraft:post/spiderclip",
        "fs_spiderclip",
        "SpiderConfig",
        2,
    ),
];
const SAMPLERS: [&str; 2] = ["In", "Blur"];

struct Pass {
    vs: &'static str,
    fs: &'static str,
    /// Per sampler slot: the target read and its filter.
    inputs: [(usize, bool); 2],
    output: usize,
    info: wgpu::Buffer,
    vs_block: wgpu::Buffer,
    fs_block: wgpu::Buffer,
}

/// A loaded chain (`PostChain`): its targets are 0 = the frame, then the internal
/// ones in id order.
struct Chain {
    targets: Vec<(String, InternalTarget)>,
    passes: Vec<Pass>,
    /// Textures for `targets[1..]`, with the size they were made at.
    textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
    size: [u32; 2],
}

/// What a frame asks for.
pub struct Request<'a> {
    /// `GameRenderer.requestedPostEffects`, in order.
    pub effects: &'a [String],
    /// The active resource root (`assets/<namespace>/post_effect/<path>.json`).
    pub root: &'a Path,
    /// `menuBackgroundBlurriness`, the radius of a `box_blur` pass with `Radius` 0.
    pub menu_blur_radius: f32,
}

impl<'a> Request<'a> {
    /// No chains.
    pub fn none() -> Self {
        Request {
            effects: &[],
            root: Path::new(""),
            menu_blur_radius: 0.0,
        }
    }
}

/// Every chain, the pipelines and the frame copy they work on.
pub struct PostEffects {
    layout: wgpu::BindGroupLayout,
    shader: wgpu::ShaderModule,
    pipelines: HashMap<(&'static str, &'static str, wgpu::TextureFormat), wgpu::RenderPipeline>,
    nearest: wgpu::Sampler,
    bilinear: wgpu::Sampler,
    /// `ShaderManager`'s chains by id; `None` when the file is missing.
    chains: HashMap<String, Option<Chain>>,
    /// `GameRenderer.failedPostEffects`.
    failed: HashSet<String>,
    root: PathBuf,
    /// The frame as the chains see `minecraft:main`.
    work: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// The pass that puts the work texture back on the frame.
    blit: Option<Pass>,
}

fn resource_path(root: &Path, id: &str) -> PathBuf {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    root.join("assets")
        .join(ns)
        .join("post_effect")
        .join(format!("{path}.json"))
}

impl PostEffects {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Post effects"),
            source: wgpu::ShaderSource::Wgsl(include_str!("post_chain.wgsl").into()),
        });
        let uniform = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Post effect bindings"),
            entries: &[
                uniform(0),
                uniform(1),
                uniform(2),
                texture(3),
                sampler(4),
                texture(5),
                sampler(6),
            ],
        });
        // SamplerCache.getClampToEdge.
        let make = |filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Post effect"),
                mag_filter: filter,
                min_filter: filter,
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                ..Default::default()
            })
        };
        Self {
            layout,
            shader,
            pipelines: HashMap::new(),
            nearest: make(wgpu::FilterMode::Nearest),
            bilinear: make(wgpu::FilterMode::Linear),
            chains: HashMap::new(),
            failed: HashSet::new(),
            root: PathBuf::new(),
            work: None,
            blit: None,
        }
    }

    /// `GameRenderer.reloadPostEffects`-alike: forget chains loaded from another
    /// root, and the failures with them (`shouldResetFailedPostEffects`).
    fn use_root(&mut self, root: &Path) {
        if self.root != root {
            self.root = root.to_owned();
            self.chains.clear();
            self.failed.clear();
        }
    }

    fn pipeline(
        &mut self,
        device: &wgpu::Device,
        key: (&'static str, &'static str),
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        let layout = &self.layout;
        let shader = &self.shader;
        self.pipelines
            .entry((key.0, key.1, format))
            .or_insert_with(|| {
                let pipeline_layout =
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("Post effect"),
                        bind_group_layouts: &[Some(layout)],
                        ..Default::default()
                    });
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("Post effect pass"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: shader,
                        entry_point: Some(key.0),
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: shader,
                        entry_point: Some(key.1),
                        // ColorTargetState.DEFAULT: no blending.
                        targets: &[Some(format.into())],
                        compilation_options: Default::default(),
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                })
            })
    }

    /// `PostChain.load` for the file of `id`: `Ok(None)` when it does not exist.
    fn load(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: &str,
    ) -> Result<Option<Chain>, String> {
        let Ok(text) = std::fs::read_to_string(resource_path(&self.root, id)) else {
            return Ok(None);
        };
        let config = Config::parse(&text)?;
        // isPostEffectValid / LevelTargetBundle.MAIN_TARGETS: the frame is the one
        // external target Horizon has.
        if let Some(bad) = config.external_targets().into_iter().find(|t| *t != MAIN) {
            return Err(format!(
                "Referenced external targets are not available in this context: [{bad}]"
            ));
        }
        let targets: Vec<(String, InternalTarget)> = std::iter::once((
            MAIN.to_owned(),
            InternalTarget {
                width: None,
                height: None,
                persistent: true,
                clear: [0.0; 4],
            },
        ))
        .chain(config.targets.iter().map(|(k, v)| (k.clone(), v.clone())))
        .collect();
        let index = |name: &str| targets.iter().position(|(t, _)| t == name);
        let mut passes = Vec::new();
        for (n, p) in config.passes.iter().enumerate() {
            let &(_, vs, vs_block) = VERTEX
                .iter()
                .find(|v| v.0 == p.vertex)
                .ok_or(format!("pass {n}: no vertex shader {}", p.vertex))?;
            let &(_, fs, fs_block, samplers) = FRAGMENT
                .iter()
                .find(|f| f.0 == p.fragment)
                .ok_or(format!("pass {n}: no fragment shader {}", p.fragment))?;
            let output = index(&p.output).ok_or("a target is missing")?;
            let mut inputs = [(0, false); 2];
            for (slot, name) in SAMPLERS.iter().enumerate().take(samplers) {
                let input = p
                    .inputs
                    .iter()
                    .find(|i| i.sampler == *name)
                    .ok_or(format!("pass {n}: {} needs the sampler {name}", p.fragment))?;
                let target = index(&input.target).ok_or("a target is missing")?;
                if target == output {
                    return Err(format!("pass {n}: reads the target it writes"));
                }
                inputs[slot] = (target, input.bilinear);
            }
            // A shader reading one sampler binds the same texture to the other,
            // never the target it writes.
            if samplers == 1 {
                inputs[1] = inputs[0];
            }
            let block = |name: &str| match p.uniforms.get(name) {
                Some(values) => pack_block(values),
                None if name.is_empty() => Ok([0; BLOCK_BYTES]),
                None => Err(format!("pass {n}: the uniform block {name} is missing")),
            };
            let pass = Pass::new(device, vs, fs, inputs, output);
            queue.write_buffer(&pass.vs_block, 0, &block(vs_block)?);
            queue.write_buffer(&pass.fs_block, 0, &block(fs_block)?);
            passes.push(pass);
        }
        Ok(Some(Chain {
            targets,
            passes,
            textures: Vec::new(),
            size: [0, 0],
        }))
    }

    fn texture(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: [u32; 2],
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Post effect target"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        (texture, view)
    }

    /// Runs the chains `req` names over `frame` (the frame's texture, copied from)
    /// and puts the result on `target` (its view): `GameRenderer.preparePostEffects`
    /// then `applyPostEffects`. `frame` must allow `COPY_SRC`. Returns whether any
    /// chain ran.
    #[allow(clippy::too_many_arguments)]
    pub fn apply(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::Texture,
        target: &wgpu::TextureView,
        size: [u32; 2],
        req: &Request,
    ) -> bool {
        self.use_root(req.root);
        let mut applied = Vec::new();
        for effect in req.effects {
            let id = normalise(effect);
            if self.failed.contains(&id) || applied.contains(&id) {
                continue;
            }
            if !self.chains.contains_key(&id) {
                let loaded = match self.load(device, queue, &id) {
                    Ok(chain) => chain,
                    Err(e) => {
                        log::warn!("Failed to load post effect {id}: {e}");
                        self.failed.insert(id.clone());
                        None
                    }
                };
                self.chains.insert(id.clone(), loaded);
            }
            if matches!(self.chains.get(&id), Some(Some(_))) {
                applied.push(id);
            }
        }
        if applied.is_empty() {
            return false;
        }
        let format = frame.format().remove_srgb_suffix();
        if self.work.as_ref().is_none_or(|(t, _)| {
            t.size().width != size[0] || t.size().height != size[1] || t.format() != format
        }) {
            self.work = Some(Self::texture(device, format, size));
        }
        if self.blit.is_none() {
            let pass = Pass::new(device, "vs_quad", "fs_blit", [(0, false); 2], 0);
            queue.write_buffer(&pass.vs_block, 0, &[0; BLOCK_BYTES]);
            queue.write_buffer(
                &pass.fs_block,
                0,
                &[1.0f32; 32].map(f32::to_le_bytes).concat(),
            );
            self.blit = Some(pass);
        }
        let copy_size = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let work = self.work.take().expect("made above");
        encoder.copy_texture_to_texture(frame.as_image_copy(), work.0.as_image_copy(), copy_size);
        for id in &applied {
            let mut chain = self.chains.remove(id).flatten().expect("loaded above");
            self.run_chain(
                device,
                queue,
                encoder,
                &work.1,
                &mut chain,
                size,
                req.menu_blur_radius,
                format,
            );
            self.chains.insert(id.clone(), Some(chain));
        }
        // The frame, as the chains left it.
        let blit = self.blit.take().expect("made above");
        self.run_pass(
            device,
            queue,
            encoder,
            &blit,
            [(&work.1, size), (&work.1, size)],
            (target, size),
            0.0,
            format,
        );
        self.blit = Some(blit);
        self.work = Some(work);
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn run_chain(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        main: &wgpu::TextureView,
        chain: &mut Chain,
        size: [u32; 2],
        menu_radius: f32,
        format: wgpu::TextureFormat,
    ) {
        let sizes: Vec<[u32; 2]> = chain
            .targets
            .iter()
            .map(|(_, t)| [t.width.unwrap_or(size[0]), t.height.unwrap_or(size[1])])
            .collect();
        // PostChain.addToFrame: internal targets at their size, cleared to their
        // colour; a persistent one only when it is made.
        let remade = chain.size != size || chain.textures.len() + 1 != chain.targets.len();
        if remade {
            chain.size = size;
            chain.textures = sizes[1..]
                .iter()
                .map(|s| Self::texture(device, format, *s))
                .collect();
        }
        for (i, (_, t)) in chain.targets.iter().enumerate().skip(1) {
            if remade || !t.persistent {
                let c = t.clear.map(f64::from);
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Post effect target clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &chain.textures[i - 1].1,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: c[0],
                                g: c[1],
                                b: c[2],
                                a: c[3],
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
            }
        }
        let view = |i: usize| {
            if i == 0 {
                main
            } else {
                &chain.textures[i - 1].1
            }
        };
        for pass in &chain.passes {
            let [a, b] = pass.inputs.map(|(t, _)| (view(t), sizes[t]));
            self.run_pass(
                device,
                queue,
                encoder,
                pass,
                [a, b],
                (view(pass.output), sizes[pass.output]),
                menu_radius,
                format,
            );
        }
    }

    /// `PostPass.addToFrame`'s execution: one fullscreen triangle.
    #[allow(clippy::too_many_arguments)]
    fn run_pass(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pass: &Pass,
        inputs: [(&wgpu::TextureView, [u32; 2]); 2],
        output: (&wgpu::TextureView, [u32; 2]),
        menu_radius: f32,
        format: wgpu::TextureFormat,
    ) {
        // SamplerInfo: OutSize, then each input's size.
        let info = [
            output.1[0] as f32,
            output.1[1] as f32,
            inputs[0].1[0] as f32,
            inputs[0].1[1] as f32,
            inputs[1].1[0] as f32,
            inputs[1].1[1] as f32,
            menu_radius,
            0.0,
        ];
        queue.write_buffer(&pass.info, 0, &info.map(f32::to_le_bytes).concat());
        self.pipeline(device, (pass.vs, pass.fs), format);
        let Some(pipeline) = self.pipelines.get(&(pass.vs, pass.fs, format)) else {
            return;
        };
        let sampler = |bilinear| {
            if bilinear {
                &self.bilinear
            } else {
                &self.nearest
            }
        };
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Post effect pass"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: pass.info.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: pass.vs_block.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: pass.fs_block.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(inputs[0].0),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(sampler(pass.inputs[0].1)),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(inputs[1].0),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(sampler(pass.inputs[1].1)),
                },
            ],
        });
        let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Post effect pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output.0,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        rp.set_pipeline(pipeline);
        rp.set_bind_group(0, &group, &[]);
        rp.draw(0..3, 0..1);
    }
}

impl Pass {
    fn new(
        device: &wgpu::Device,
        vs: &'static str,
        fs: &'static str,
        inputs: [(usize, bool); 2],
        output: usize,
    ) -> Self {
        let buffer = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Pass {
            vs,
            fs,
            inputs,
            output,
            info: buffer("Post effect sampler info", 32),
            vs_block: buffer("Post effect vertex uniforms", BLOCK_BYTES as u64),
            fs_block: buffer("Post effect fragment uniforms", BLOCK_BYTES as u64),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INVERT_THEN_BLIT: &str = r#"{
        "targets": { "swap": {} },
        "passes": [
            { "vertex_shader": "minecraft:core/screenquad",
              "fragment_shader": "minecraft:post/invert",
              "inputs": [ { "sampler_name": "In", "target": "minecraft:main" } ],
              "output": "swap",
              "uniforms": { "InvertConfig": [ { "name": "InverseAmount", "type": "float", "value": 1.0 } ] } },
            { "vertex_shader": "minecraft:core/screenquad",
              "fragment_shader": "minecraft:post/blit",
              "inputs": [ { "sampler_name": "In", "target": "swap", "bilinear": true } ],
              "output": "minecraft:main",
              "uniforms": { "BlitConfig": [ { "name": "ColorModulate", "type": "vec4", "value": [1.0, 0.0, 0.5, 1.0] } ] } }
        ]
    }"#;

    #[test]
    fn shaders_validate() {
        let module = naga::front::wgsl::parse_str(include_str!("post_chain.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    fn a_chain_is_read_like_postchainconfig() {
        let config = Config::parse(INVERT_THEN_BLIT).unwrap();
        // Ids without a namespace are minecraft's.
        assert_eq!(
            config.targets.keys().collect::<Vec<_>>(),
            ["minecraft:swap"]
        );
        assert_eq!(config.passes.len(), 2);
        assert_eq!(config.passes[0].output, "minecraft:swap");
        assert_eq!(
            config.passes[1].inputs,
            [Input {
                sampler: "In".into(),
                target: "minecraft:swap".into(),
                bilinear: true
            }]
        );
        assert_eq!(
            config.passes[1].uniforms["BlitConfig"],
            [Uniform::Vec4([1.0, 0.0, 0.5, 1.0])]
        );
        // Only the frame is outside the chain.
        assert_eq!(config.external_targets(), HashSet::from(["minecraft:main"]));
    }

    #[test]
    fn target_options_and_clear_colour() {
        let config = Config::parse(
            r#"{"targets":{"a":{"width":16,"height":8,"persistent":true,"clear_color":2180972544}}}"#,
        )
        .unwrap();
        let t = &config.targets["minecraft:a"];
        assert_eq!((t.width, t.height, t.persistent), (Some(16), Some(8), true));
        // 0x81FF0000: alpha 129, red 255.
        assert_eq!(t.clear, [1.0, 0.0, 0.0, 129.0 / 255.0]);
        assert!(Config::parse(r#"{"targets":{"a":{"width":0}}}"#).is_err());
    }

    #[test]
    fn inputs_are_checked() {
        let pass = |inputs: &str| {
            format!(
                r#"{{"passes":[{{"vertex_shader":"a","fragment_shader":"b","output":"o","inputs":{inputs}}}]}}"#
            )
        };
        // Repeated sampler names are refused.
        assert!(
            Config::parse(&pass(
                r#"[{"sampler_name":"In","target":"x"},{"sampler_name":"In","target":"y"}]"#
            ))
            .is_err()
        );
        // Texture and depth inputs are not drawn.
        assert!(
            Config::parse(&pass(
                r#"[{"sampler_name":"In","location":"noise","width":4,"height":4}]"#
            ))
            .is_err()
        );
        assert!(
            Config::parse(&pass(
                r#"[{"sampler_name":"In","target":"x","use_depth_buffer":true}]"#
            ))
            .is_err()
        );
    }

    #[test]
    fn blocks_are_packed_as_std140() {
        // BlurConfig { vec2 BlurDir; float Radius }.
        let block = pack_block(&[Uniform::Vec2([1.0, 0.0]), Uniform::Float(7.0)]).unwrap();
        let words: Vec<f32> = block
            .chunks(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        assert_eq!(words[..3], [1.0, 0.0, 7.0]);
        // ColorConfig { vec3, vec3, vec3 }: each starts on 16 bytes.
        let block = pack_block(&[
            Uniform::Vec3([1.0, 2.0, 3.0]),
            Uniform::Vec3([4.0, 5.0, 6.0]),
        ])
        .unwrap();
        let words: Vec<f32> = block
            .chunks(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        assert_eq!(words[..7], [1.0, 2.0, 3.0, 0.0, 4.0, 5.0, 6.0]);
        // RotScaleConfig { vec2, vec2, float }.
        let block = pack_block(&[
            Uniform::Vec2([1.0, 2.0]),
            Uniform::Vec2([3.0, 4.0]),
            Uniform::Float(5.0),
        ])
        .unwrap();
        let words: Vec<f32> = block
            .chunks(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        assert_eq!(words[..5], [1.0, 2.0, 3.0, 4.0, 5.0]);
        assert!(pack_block(&[Uniform::Matrix([0.0; 16]); 3]).is_err());
    }

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        pollster::block_on(adapter.request_device(&Default::default())).ok()
    }

    fn scratch(name: &str, json: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("horizon-post-chain-{}-{name}", std::process::id()));
        let dir = root.join("assets/minecraft/post_effect");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{name}.json")), json).unwrap();
        root
    }

    fn run(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        effects: &mut PostEffects,
        root: &Path,
        names: &[String],
    ) -> (bool, [u8; 4]) {
        let frame = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 8,
                height: 4,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = frame.create_view(&Default::default());
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = device.create_command_encoder(&Default::default());
        // The frame drawn so far: (0.2, 0.4, 0.6).
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.2,
                        g: 0.4,
                        b: 0.6,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        let ran = effects.apply(
            device,
            queue,
            &mut encoder,
            &frame,
            &view,
            [8, 4],
            &Request {
                effects: names,
                root,
                menu_blur_radius: 5.0,
            },
        );
        let buffer = crate::gfx::copy_frame(device, &mut encoder, &frame, [8, 4]);
        queue.submit([encoder.finish()]);
        if let Some(error) = pollster::block_on(scope.pop()) {
            panic!("{error}");
        }
        buffer.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = buffer.get_mapped_range(..).unwrap();
        (ran, [data[0], data[1], data[2], data[3]])
    }

    /// The chain runs over the frame and the result lands on it: invert into
    /// `swap`, then a blit modulated by (1, 0, 0.5, 1) back onto the frame.
    #[test]
    fn a_chain_draws_onto_the_frame() {
        let Some((device, queue)) = device() else {
            return;
        };
        let root = scratch("invert_then_blit", INVERT_THEN_BLIT);
        let mut effects = PostEffects::new(&device);
        let names = ["minecraft:invert_then_blit".to_owned()];
        let (ran, px) = run(&device, &queue, &mut effects, &root, &names);
        assert!(ran);
        // (0.8, 0.6, 0.4) * (1, 0, 0.5): 204, 0, 51.
        let near = |a: u8, b: u8| a.abs_diff(b) <= 2;
        assert!(
            near(px[0], 204) && px[1] == 0 && near(px[2], 51) && px[3] == 255,
            "{px:?}"
        );
        // A second frame is the same (the chain and its targets are reused).
        let (_, again) = run(&device, &queue, &mut effects, &root, &names);
        assert_eq!(again, px);
        // Missing chains and a chain that does not load change nothing.
        let (ran, px) = run(
            &device,
            &queue,
            &mut effects,
            &root,
            &["minecraft:nothing".to_owned()],
        );
        assert!(!ran);
        assert!(
            near(px[0], 51) && near(px[1], 102) && near(px[2], 153),
            "{px:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_chain_naming_an_unknown_shader_fails_once() {
        let Some((device, queue)) = device() else {
            return;
        };
        let root = scratch(
            "unknown",
            r#"{"passes":[{"vertex_shader":"minecraft:core/screenquad","fragment_shader":"minecraft:post/nope",
                "inputs":[{"sampler_name":"In","target":"minecraft:main"}],"output":"minecraft:main"}]}"#,
        );
        let mut effects = PostEffects::new(&device);
        let names = ["minecraft:unknown".to_owned()];
        assert!(!run(&device, &queue, &mut effects, &root, &names).0);
        assert!(effects.failed.contains("minecraft:unknown"));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The chains the prepared vanilla resources hold all load (skipped without
    /// them): none names a shader Horizon lacks.
    #[test]
    fn vanilla_chains_load() {
        let Some((device, queue)) = device() else {
            return;
        };
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3");
        if !root
            .join("assets/minecraft/post_effect/invert.json")
            .exists()
        {
            return;
        }
        let mut effects = PostEffects::new(&device);
        effects.use_root(&root);
        for name in ["blur", "creeper", "invert", "spider"] {
            let id = format!("minecraft:{name}");
            assert!(
                effects.load(&device, &queue, &id).unwrap().is_some(),
                "{id}"
            );
        }
        // entity_outline needs targets and shaders the world pass has of its own.
        assert!(
            effects
                .load(&device, &queue, "minecraft:entity_outline")
                .is_err()
        );
    }
}
