//! Shared nearest-sampled sprite array for block and item textures.
use crate::{
    Result,
    resource_source::{ResourceRoot, ResourceSource},
};
use horizon_assets::animation::{AnimationClip, AnimationMetadata};
#[cfg(test)]
use std::path::Path;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone)]
enum AnimationFrames {
    Raw(Arc<Vec<Vec<u8>>>),
    Packed(Arc<PackedFrames>),
}
struct PackedFrames {
    frame_lens: Vec<usize>,
    compressed: Vec<u8>,
    /// Decompressed lazily if [`SpriteAnimation::pixels`] is ever called after
    /// [`SpriteAnimation::pack`] (for example, on renderer rebuild after device loss).
    unpacked: std::sync::OnceLock<Vec<Vec<u8>>>,
}
impl PackedFrames {
    fn compress(frames: &[Vec<u8>]) -> Self {
        let frame_lens: Vec<usize> = frames.iter().map(Vec::len).collect();
        let total: usize = frame_lens.iter().sum();
        let mut flat = Vec::with_capacity(total);
        for frame in frames {
            flat.extend_from_slice(frame);
        }
        let mut compressed = lz4_flex::compress_prepend_size(&flat);
        compressed.shrink_to_fit();
        Self {
            frame_lens,
            compressed,
            unpacked: std::sync::OnceLock::new(),
        }
    }
    fn frames(&self) -> &[Vec<u8>] {
        self.unpacked.get_or_init(|| {
            let expected: usize = self.frame_lens.iter().sum();
            let raw = lz4_flex::decompress_size_prepended(&self.compressed)
                .ok()
                .filter(|b| b.len() == expected)
                .unwrap_or_else(|| vec![0; expected]);
            let mut out = Vec::with_capacity(self.frame_lens.len());
            let mut offset = 0;
            for &len in &self.frame_lens {
                out.push(raw[offset..offset + len].to_vec());
                offset += len;
            }
            out
        })
    }
}
impl AnimationFrames {
    fn len(&self) -> usize {
        match self {
            Self::Raw(frames) => frames.len(),
            Self::Packed(packed) => packed.frame_lens.len(),
        }
    }
    fn pixels(&self, frame: usize) -> &[u8] {
        match self {
            Self::Raw(frames) => &frames[frame],
            Self::Packed(packed) => &packed.frames()[frame],
        }
    }
    fn is_packed(&self) -> bool {
        matches!(self, Self::Packed(_))
    }
    fn ram_bytes(&self) -> usize {
        match self {
            Self::Raw(frames) => frames.iter().map(Vec::capacity).sum(),
            Self::Packed(packed) => {
                let base = packed.compressed.capacity()
                    + packed.frame_lens.capacity() * std::mem::size_of::<usize>();
                let unpacked = packed
                    .unpacked
                    .get()
                    .map_or(0, |v| v.iter().map(Vec::capacity).sum());
                base + unpacked
            }
        }
    }
    #[cfg(test)]
    fn raw_arc(&self) -> Option<&Arc<Vec<Vec<u8>>>> {
        match self {
            Self::Raw(frames) => Some(frames),
            Self::Packed(_) => None,
        }
    }
}

#[derive(Clone)]
pub struct SpriteAnimation {
    pub layer: u32,
    frames: AnimationFrames,
    clip: Arc<AnimationClip>,
    materials: [Option<Arc<Vec<Vec<u8>>>>; 2],
    /// The frames' width and height when they are not square; `None` for square frames. Frames
    /// stay as decoded: [`SpriteAnimation::frame_at`] makes a frame's square at any size.
    shape: Option<[u32; 2]>,
}
/// What an animated sprite shows: `frame`, mixed `millis` thousandths of the
/// way towards `next` (0 for a single frame).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameBlend {
    pub frame: usize,
    pub next: usize,
    pub millis: u32,
}
impl SpriteAnimation {
    pub(crate) fn has_materials(&self) -> bool {
        self.materials.iter().any(Option::is_some)
    }
    /// Material categories follow the selected colour frame, without colour interpolation.
    #[cfg(test)]
    pub(crate) fn material_pixels(&self, kind: usize, tick: u64) -> Option<&[u8]> {
        let frames = self.materials[kind].as_ref()?;
        Some(&frames[self.frame(tick) % frames.len()])
    }
    pub(crate) fn material_frames(&self, kind: usize) -> Option<&[Vec<u8>]> {
        self.materials[kind].as_deref().map(Vec::as_slice)
    }
    /// Polytone's `mode` on this animation, when it has one.
    pub fn mode(&self) -> Option<&horizon_assets::animation::ClipMode> {
        self.clip.mode()
    }
    pub fn frame(&self, tick: u64) -> usize {
        self.clip.sample(tick).frame as usize
    }
    /// `SpriteContents.AnimationState.drawToAtlas`: an animation with
    /// `interpolate` mixes the frame with the next one by
    /// `(int)(subFrame / time * 1000)` thousandths; others show the frame.
    pub fn blend(&self, tick: u64) -> FrameBlend {
        self.blend_sample(self.clip.sample(tick))
    }
    /// [`Self::blend`], or what a Polytone `mode` (time of day, weather, an expression) picks
    /// from `inputs` for an animation that has one.
    pub fn blend_with(
        &self,
        tick: u64,
        inputs: Option<&horizon_assets::animation::ModeInputs>,
    ) -> FrameBlend {
        match inputs.and_then(|i| self.clip.sample_mode(i)) {
            Some(sample) => self.blend_sample(sample),
            None => self.blend(tick),
        }
    }
    fn blend_sample(&self, sample: horizon_assets::animation::AnimationSample) -> FrameBlend {
        let frame = sample.frame as usize;
        let next = sample.next_frame as usize;
        if self.clip.interpolates() && sample.interpolation_millis > 0 && next != frame {
            FrameBlend {
                frame,
                next,
                millis: sample.interpolation_millis,
            }
        } else {
            FrameBlend {
                frame,
                next: frame,
                millis: 0,
            }
        }
    }
    /// A frame nearest-resampled to `width` x `height`. A frame that is not square yet reads as
    /// its square (of its longer side) would: the texels the layer will hold.
    pub fn resized_pixels(&self, frame: usize, width: u32, height: u32) -> Vec<u8> {
        let source = self.pixels(frame);
        let [fw, fh] = self
            .shape
            .unwrap_or_else(|| [(source.len() / 4).isqrt() as u32; 2]);
        if width == 0 || height == 0 || fw == 0 || fh == 0 {
            return Vec::new();
        }
        if fw == fh {
            if width == fw && height == fh {
                return source.to_vec();
            }
            if width == height {
                let mut out = Vec::with_capacity(width as usize * height as usize * 4);
                resample_into(source, fw, width, &mut out);
                return out;
            }
        }
        let side = u64::from(fw.max(fh));
        let pick = |i: u32, extent: u32, along: u32| {
            (u64::from(i) * side / u64::from(extent) * u64::from(along) / side) as usize
        };
        let col_map: smallvec::SmallVec<[usize; 64]> =
            (0..width).map(|x| pick(x, width, fw)).collect();
        let fw_usize = fw as usize;
        let mut out = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            let row = pick(y, height, fh) * fw_usize;
            for &sx in &col_map {
                let offset = (row + sx) * 4;
                out.extend_from_slice(&source[offset..offset + 4]);
            }
        }
        out
    }
    pub fn pixels(&self, frame: usize) -> &[u8] {
        self.frames.pixels(frame)
    }
    /// A frame as its layer shows it at `side` x `side`: its square of its longer side,
    /// nearest-resampled to `side` (the texels squaring then `resample` picked).
    pub fn frame_at(&self, frame: usize, side: u32) -> Vec<u8> {
        let source = self.pixels(frame);
        let [w, h] = self
            .shape
            .unwrap_or_else(|| [(source.len() / 4).isqrt() as u32; 2]);
        region_layer(source, w, [0, 0, w, h], side)
    }
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }
    /// Compress CPU frame buffers with LZ4 once the GPU holds the animation frames.
    /// Subsequent [`Self::pixels`] calls transparently decompress on demand.
    pub fn pack(&mut self) {
        if let AnimationFrames::Raw(frames) = &self.frames {
            self.frames = AnimationFrames::Packed(Arc::new(PackedFrames::compress(frames)));
        }
    }
    /// Whether this animation's CPU frame buffers are LZ4-packed.
    pub fn is_packed(&self) -> bool {
        self.frames.is_packed()
    }
    /// Heap bytes currently retained by this animation's colour frames.
    pub fn frame_ram_bytes(&self) -> usize {
        self.frames.ram_bytes()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct StillKey {
    frame: u64,
    pbr: u64,
    side: u32,
    settings: [u32; 2],
    flags: [bool; 4],
}

#[derive(Default)]
pub struct Sprites {
    source: ResourceSource,
    /// Edge length of every array layer and the layers' current frames, set
    /// by [`Sprites::finish`].
    pub size: u32,
    /// Smallest native layer before array-size resampling.
    pub minimum_size: u32,
    pub pixels: Vec<u8>,
    pub layers: BTreeMap<String, u32>,
    pub animations: Vec<SpriteAnimation>,
    pub opaque: Vec<bool>,
    pub translucent: Vec<bool>,
    /// Each layer's `MipmapGenerator` settings.
    pub mips: Vec<crate::mipmap::Settings>,
    /// Each layer's frames at the texture's own resolution, its longer side first, and the
    /// frame shown first. Square unless `shapes` names the layer.
    native: Vec<(u32, Arc<Vec<Vec<u8>>>, usize)>,
    /// The width and height of the frames of each layer whose frames are not square: they are
    /// squared once, at the layers' size, by [`Sprites::finish`].
    shapes: std::collections::HashMap<u32, [u32; 2]>,
    /// Each layer's LabPBR maps (`_n` normals, `_s` specular) beside its texture: their
    /// frames at their own resolution (square).
    pbr: Vec<[Option<(u32, Arc<Vec<Vec<u8>>>)>; 2]>,
    /// The layers' normal and specular maps at `pbr_size`, layer after layer, set by
    /// [`Sprites::finish`]: empty when no texture has either.
    pub normals: Vec<u8>,
    pub specular: Vec<u8>,
    pub pbr_size: u32,
    /// Still textures by their pixels, their LabPBR maps and mip settings: mods copy vanilla's
    /// textures under their own names, and one layer serves them all (the GPU's array may
    /// hold as few as 2048).
    still: std::collections::HashMap<StillKey, u32>,
    /// Names a connected-texture rule matches by name (`matchTiles`, `connectTiles`): each keeps a
    /// layer of its own ([`Sprites::keep_apart`]).
    apart: std::collections::HashSet<String>,
    /// Decoded textures kept on disk between launches ([`Sprites::with_disk_cache`]).
    disk: Option<crate::sprite_cache::SpriteCache>,
    /// Textures [`Sprites::prefetch`] decoded ahead of their insert, by sprite name or
    /// resource path.
    ready: std::collections::HashMap<String, crate::sprite_cache::Decoded>,
    /// Textures past [`max_layers`] that the missing texture stands in for
    /// ([`Sprites::plan_within_capacity`]).
    dropped: std::collections::HashSet<String>,
    /// The one missing texture layer those share.
    overflow_layer: Option<u32>,
}
/// The most block sprite layers the renderer draws from: its arrays
/// (`gfx::SPRITE_ARRAYS`) of the layers known before the GPU is (`gfx::sprite_split`,
/// 2048 unless `HORIZON_SPRITE_SPLIT` asks for fewer). A GPU that holds fewer per array
/// still refuses more, as before.
pub fn max_layers() -> usize {
    crate::gfx::SPRITE_ARRAYS as usize * crate::gfx::sprite_split() as usize
}
/// Layers [`Sprites::plan_within_capacity`] keeps free for what later loaders add
/// (connected-texture tiles, run-time sprites).
const LATE_LAYERS: usize = 512;
/// Edge length of vanilla's block textures: the smallest array layer.
pub const SPRITE_SIZE: u32 = 16;
/// The largest sprite array (all layers of the top mip level) when the settings leave it to
/// Horizon: resource packs with bigger textures are sampled down to fit.
pub const DEFAULT_TEXTURE_BUDGET: u64 = 256 << 20;
/// The budget in bytes the next resource load keeps to (`set_texture_budget`).
static TEXTURE_BUDGET: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(DEFAULT_TEXTURE_BUDGET);
/// The block textures' budget from the `textureBudgetMiB` setting (0 for the default), for
/// resources loaded from now on.
pub fn set_texture_budget(mib: u32) {
    let bytes = match mib {
        0 => DEFAULT_TEXTURE_BUDGET,
        mib => u64::from(mib) << 20,
    };
    TEXTURE_BUDGET.store(bytes, std::sync::atomic::Ordering::Relaxed);
}
/// Whether resource loads read the resource packs' LabPBR maps (`set_labpbr_maps`).
static LABPBR_MAPS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Whether resource loads from now on read the resource packs' LabPBR maps beside their block
/// textures. Native material stages and terrain shader packs read them; without a pack they cost
/// neither loading time nor video memory.
pub fn set_labpbr_maps(on: bool) {
    LABPBR_MAPS.store(on, std::sync::atomic::Ordering::Relaxed);
}
/// Whether resource loads read the LabPBR maps (`set_labpbr_maps`).
pub fn labpbr_maps() -> bool {
    LABPBR_MAPS.load(std::sync::atomic::Ordering::Relaxed)
}
/// Budget for either complete normal or specular image set: a quarter of the block textures'
/// budget, so raising `textureBudgetMiB` sharpens a pack's LabPBR maps along with its colours.
fn map_bytes(texture_budget: u64) -> u64 {
    texture_budget / 4
}
/// [`map_bytes`] for the budget the next resource load keeps to.
fn max_map_bytes() -> u64 {
    map_bytes(TEXTURE_BUDGET.load(std::sync::atomic::Ordering::Relaxed))
}
/// Fewer layers than any game has (vanilla's are about 1300): what a map is kept at while
/// loading assumes at least these many share the array.
const LAYERS_AT_LEAST: u64 = 1024;
/// A layer's normal map where its texture has none: flat, unoccluded, at full height.
pub const FLAT_NORMAL: [u8; 4] = [128, 128, 255, 255];
/// A layer's specular map where its texture has none: rough, not metal, not glowing.
pub const NO_SPECULAR: [u8; 4] = [0, 0, 0, 0];
/// The largest power-of-two side from `start` down (at least `SPRITE_SIZE`, at most a full
/// atlas side, whatever the budget) at which `layers` square RGBA layers take at most `bytes`.
fn side_within(start: u32, layers: u64, bytes: u64) -> u32 {
    let mut size = start
        .clamp(SPRITE_SIZE, crate::hud::MAX_ATLAS_SIZE)
        .next_power_of_two();
    let take = |size: u32| u128::from(layers) * u128::from(size).pow(2) * 4;
    while size > SPRITE_SIZE && take(size) > u128::from(bytes) {
        size /= 2;
    }
    size
}
/// The most texels of a non-square frame's square [`hash_square`] walks.
const SQUARE_HASHED: u64 = 1 << 24;
/// Hashes the square of its longer side a `w` x `h` RGBA frame becomes, row by row, without
/// making it: still textures share a layer when their squares are equal, as when frames were
/// squared at decoding. A square frame is its own square. A non-square frame whose square has
/// more than [`SQUARE_HASHED`] texels hashes its own texels and shape instead: walking its square
/// would cost more than the frame, and a different shape gives the same square only for
/// degenerate content (a uniform colour, rows repeating as squaring repeats them).
fn hash_square(frame: &[u8], [w, h]: [u32; 2], hasher: &mut impl std::hash::Hasher) {
    let side = u64::from(w.max(h));
    if w != h && side * side > SQUARE_HASHED {
        hasher.write_u8(1);
        hasher.write(frame);
        hasher.write_u32(w);
        hasher.write_u32(h);
        return;
    }
    hasher.write_u8(0);
    hasher.write_u64(side);
    if w == h {
        let row_bytes = w as usize * 4;
        for r in 0..w as usize {
            hasher.write(&frame[r * row_bytes..(r + 1) * row_bytes]);
        }
        return;
    }
    let mut row = Vec::with_capacity(side as usize * 4);
    for r in 0..side {
        let source_row = (r * u64::from(h) / side) as usize * w as usize;
        row.clear();
        for c in 0..side {
            let at = (source_row + (c * u64::from(w) / side) as usize) * 4;
            row.extend_from_slice(&frame[at..at + 4]);
        }
        hasher.write(&row);
    }
}
/// Stream the `size` x `size` layer of the `w` x `h` region at `x, y` of RGBA `pixels` (`stride`
/// pixels wide) directly into `out`.
fn region_layer_into(
    pixels: &[u8],
    stride: u32,
    [x, y, w, h]: [u32; 4],
    size: u32,
    out: &mut Vec<u8>,
) {
    if size == 0 || w == 0 || h == 0 {
        return;
    }
    if w == h {
        if x == 0 && y == 0 && stride == w {
            let frame_bytes = w as usize * h as usize * 4;
            resample_into(&pixels[..frame_bytes], w, size, out);
            return;
        }
        if w == size {
            let row_bytes = w as usize * 4;
            let stride_bytes = stride as usize * 4;
            let x_bytes = x as usize * 4;
            for row in 0..h as usize {
                let start = (y as usize + row) * stride_bytes + x_bytes;
                out.extend_from_slice(&pixels[start..start + row_bytes]);
            }
            return;
        }
    }
    let side = u64::from(w.max(h));
    let pick = |i: u32, extent: u32| {
        (u64::from(i) * side / u64::from(size) * u64::from(extent) / side) as usize
    };
    let col_map: smallvec::SmallVec<[usize; 64]> = (0..size)
        .map(|column| x as usize + pick(column, w))
        .collect();
    let stride_usize = stride as usize;
    for row in 0..size {
        let row_base = (y as usize + pick(row, h)) * stride_usize;
        for &col in &col_map {
            let at = (row_base + col) * 4;
            out.extend_from_slice(&pixels[at..at + 4]);
        }
    }
}
/// The `size` x `size` layer of the `w` x `h` region at `x, y` of RGBA `pixels`, `stride` pixels
/// wide. Its texels are those of the region nearest-resampled to a square of its longer side and
/// that square nearest-resampled to `size` (`resample`), picked in one pass, so the square of
/// the longer side is never made.
fn region_layer(pixels: &[u8], stride: u32, [x, y, w, h]: [u32; 4], size: u32) -> Vec<u8> {
    if w == h && w == size && x == 0 && y == 0 && stride == w {
        return pixels[..w as usize * h as usize * 4].to_vec();
    }
    let mut out = Vec::with_capacity(size as usize * size as usize * 4);
    region_layer_into(pixels, stride, [x, y, w, h], size, &mut out);
    out
}
/// Reduce data maps by selecting whole texels: material IDs and sentinel bytes cannot be averaged.
#[allow(dead_code)]
fn shrink_map(frame: Vec<u8>, side: u32, max: u32) -> (u32, Vec<u8>) {
    if side <= max || !side.is_power_of_two() || !max.is_power_of_two() {
        return (side, frame);
    }
    (max, resample(&frame, side, max))
}
/// Nearest-resample a square RGBA frame directly into `out`.
fn resample_into(frame: &[u8], from: u32, to: u32, out: &mut Vec<u8>) {
    if from == to {
        out.extend_from_slice(frame);
        return;
    }
    if from == 0 || to == 0 {
        return;
    }
    let from_usize = from as usize;
    let to_usize = to as usize;
    if to > from && to.is_multiple_of(from) {
        let scale = to_usize / from_usize;
        let row_bytes = to_usize * 4;
        for src_row in 0..from_usize {
            let row_start = out.len();
            let base = src_row * from_usize * 4;
            for src_col in 0..from_usize {
                let px = &frame[base + src_col * 4..base + src_col * 4 + 4];
                for _ in 0..scale {
                    out.extend_from_slice(px);
                }
            }
            for _ in 1..scale {
                out.extend_from_within(row_start..row_start + row_bytes);
            }
        }
        return;
    }
    if from > to && from.is_multiple_of(to) {
        let step = from_usize / to_usize;
        for row in 0..to_usize {
            let row_base = row * step * from_usize;
            for column in 0..to_usize {
                let offset = (row_base + column * step) * 4;
                out.extend_from_slice(&frame[offset..offset + 4]);
            }
        }
        return;
    }
    let col_map: smallvec::SmallVec<[usize; 64]> = (0..to)
        .map(|column| (u64::from(column) * u64::from(from) / u64::from(to)) as usize)
        .collect();
    for row in 0..to {
        let row_base = (u64::from(row) * u64::from(from) / u64::from(to)) as usize * from_usize;
        for &src_col in &col_map {
            let offset = (row_base + src_col) * 4;
            out.extend_from_slice(&frame[offset..offset + 4]);
        }
    }
}
/// Nearest-resample a square RGBA frame to another square size.
pub(crate) fn resample(frame: &[u8], from: u32, to: u32) -> Vec<u8> {
    if from == to {
        return frame.to_vec();
    }
    let mut out = Vec::with_capacity(to as usize * to as usize * 4);
    resample_into(frame, from, to, &mut out);
    out
}
impl Sprites {
    /// A sprite atlas retains the source used for this resource load.
    pub fn new(source: ResourceSource) -> Self {
        Self {
            source,
            ..Self::default()
        }
    }
    /// Sprites that reuse textures decoded by an earlier launch, kept in a
    /// cache file in `root`.
    pub fn with_disk_cache(root: &ResourceRoot) -> Self {
        Self {
            source: root.source().clone(),
            disk: crate::sprite_cache::SpriteCache::open(root),
            ..Self::default()
        }
    }
    /// Keeps each of `names` on a layer of its own, never shared with an identical texture: the
    /// connected-texture rules that name them find their faces by layer, so a texture sharing it
    /// would be matched too (a tile a rule chains onto, and a model's transparent placeholder
    /// with the same pixels). Before the textures that bear these names are inserted.
    pub fn keep_apart(&mut self, names: impl IntoIterator<Item = String>) {
        self.apart.extend(names);
    }
    /// Write what this run decoded to the cache file.
    pub fn save_disk_cache(&mut self) {
        if let Some(disk) = &mut self.disk {
            disk.save();
        }
    }
    /// How many array layers the textures take.
    pub fn count(&self) -> usize {
        self.native.len()
    }
    /// `SpriteContents.computeTransparency(u0, v0, u1, v1)` is NONE: every texel under
    /// the quad's UV box is fully opaque in every frame, so the quad needs no alpha test.
    pub fn region_opaque(&self, layer: u32, uv: &[[f32; 2]; 4]) -> bool {
        if self.opaque[layer as usize] {
            return true;
        }
        let (side, frames, _) = &self.native[layer as usize];
        let [width, height] = self.shapes.get(&layer).copied().unwrap_or([*side; 2]);
        let side = *side as usize;
        let (width, height) = (width as usize, height as usize);
        let range = |axis: usize| {
            let lo = uv.iter().map(|p| p[axis]).fold(f32::INFINITY, f32::min);
            let hi = uv.iter().map(|p| p[axis]).fold(f32::NEG_INFINITY, f32::max);
            let clamp = |v: f32| (v.max(0.0) as usize).min(side);
            clamp((lo * side as f32).floor())..clamp((hi * side as f32).ceil())
        };
        // The frame's texels under the square's range: squaring repeats each, in order, so the
        // range's ends map to the ends of a contiguous run, and each texel is read once.
        let source = |r: std::ops::Range<usize>, extent: usize| match r.is_empty() {
            true => 0..0,
            false => {
                // In `u64`: a side past 65536 overflows these products in a 32-bit `usize`.
                let map = |i: usize| (i as u64 * extent as u64 / side as u64) as usize;
                map(r.start)..map(r.end - 1) + 1
            }
        };
        let (columns, rows) = (source(range(0), width), source(range(1), height));
        frames.iter().all(|frame| {
            rows.clone().all(|y| {
                columns
                    .clone()
                    .all(|x| frame[(y * width + x) * 4 + 3] == 255)
            })
        })
    }
    /// Lay every layer out at one size: the largest texture's (resource packs
    /// may ship 32x, 64x... textures, like vanilla's stitched atlas), rounded
    /// up to a power of two and limited to the texture budget (`set_texture_budget`). Smaller
    /// textures are scaled up by whole pixels, so UVs stay per layer.
    #[horizon_mod_api_macros::replaceable]
    pub fn finish(&mut self) {
        self.lay_out(true);
    }
    /// [`Sprites::finish`] for a server, which draws nothing: the layers'
    /// size, but neither their pixels nor the animations' frames.
    pub fn finish_without_pixels(&mut self) {
        self.lay_out(false);
    }
    fn lay_out(&mut self, pixels: bool) {
        // Prefetched textures nothing inserted.
        self.ready = Default::default();
        let layers = self.native.len().max(1) as u64;
        let largest = self.native.iter().map(|n| n.0).max().unwrap_or(SPRITE_SIZE);
        self.minimum_size = self.native.iter().map(|n| n.0).min().unwrap_or(SPRITE_SIZE);
        let budget = TEXTURE_BUDGET.load(std::sync::atomic::Ordering::Relaxed);
        let size = side_within(largest, layers, budget);
        if pixels {
            log::info!(
                "Horizon: block sprites: {layers} layers at {size} px (largest {largest} px, budget {} MiB)",
                budget >> 20
            );
        }
        if !pixels {
            self.pixels = Vec::new();
            self.animations = Vec::new();
            self.pbr = Vec::new();
            self.size = size;
            return;
        }
        let layer_count = self.native.len();
        let layer_bytes = size as usize * size as usize * 4;
        self.pixels.clear();
        self.pixels.reserve_exact(layer_count * layer_bytes);
        // Only the frame each layer shows first is made at the layers' size, streamed directly
        // into `self.pixels` without intermediate per-layer allocations. Animations keep their
        // frames as decoded (shared, not copied); the animator makes them at its own edge and
        // charges what it builds first (`gfx::animate`).
        for (layer, (from, frames, initial)) in self.native.iter().enumerate() {
            let frame = &frames[*initial];
            match self.shapes.get(&(layer as u32)) {
                Some(&[w, h]) => region_layer_into(frame, w, [0, 0, w, h], size, &mut self.pixels),
                None if *from == size => self.pixels.extend_from_slice(frame),
                None => resample_into(frame, *from, size, &mut self.pixels),
            }
        }
        for animation in &mut self.animations {
            let layer = animation.layer;
            animation.frames = AnimationFrames::Raw(Arc::clone(&self.native[layer as usize].1));
            animation.shape = self.shapes.get(&layer).copied();
        }
        self.size = size;
        self.normals.clear();
        self.specular.clear();
        let largest_map = self.pbr.iter().flatten().flatten().map(|m| m.0).max();
        self.pbr_size = largest_map.map_or(SPRITE_SIZE, |l| {
            side_within(l.min(size), layers, max_map_bytes())
        });
        if largest_map.is_some() {
            let pbr_size = self.pbr_size;
            log::info!("Horizon: LabPBR maps at {pbr_size} px, {layers} layers");
            // Only each layer's initial frame is made at the shared size here, streamed directly
            // into pre-reserved buffers. Animations share the companion frames as stored without
            // moving `self.pbr` out, so repeated layouts keep every map aligned with its layer.
            let map_layer_bytes = pbr_size as usize * pbr_size as usize * 4;
            self.normals.reserve_exact(self.pbr.len() * map_layer_bytes);
            self.specular
                .reserve_exact(self.pbr.len() * map_layer_bytes);
            for (layer, maps) in self.pbr.iter().enumerate() {
                for (map, (out, none)) in maps.iter().zip([
                    (&mut self.normals, FLAT_NORMAL),
                    (&mut self.specular, NO_SPECULAR),
                ]) {
                    match map {
                        Some((from, frames)) => {
                            let initial = self.native[layer].2 % frames.len();
                            resample_into(&frames[initial], *from, pbr_size, out);
                        }
                        None => {
                            let start = out.len();
                            out.resize(start + map_layer_bytes, 0);
                            if none != [0, 0, 0, 0] {
                                for px in out[start..].as_chunks_mut::<4>().0 {
                                    *px = none;
                                }
                            }
                        }
                    }
                }
            }
            for animation in &mut self.animations {
                if let Some(maps) = self.pbr.get(animation.layer as usize) {
                    animation.materials = maps
                        .each_ref()
                        .map(|map| map.as_ref().map(|(_, frames)| Arc::clone(frames)));
                }
            }
        } else {
            for animation in &mut self.animations {
                animation.materials = [None, None];
            }
        }
    }
    /// [`Sprites::insert`] without the missing-texture fallback: a file that doesn't load is an
    /// error, and nothing is added.
    pub fn try_insert(&mut self, name: &str) -> Result<u32> {
        match self.layers.get(name) {
            Some(layer) => Ok(*layer),
            None => self.load(name, None),
        }
    }
    /// Load a sprite id or resource file only when its PNG fits the byte limit,
    /// including cached layers. The checked bytes are reused when decoding.
    pub fn try_insert_limited(&mut self, name: &str, max: u64) -> Result<u32> {
        let bytes = self.limited_png(name, max)?;
        match self.layers.get(name) {
            Some(layer) => Ok(*layer),
            None => self.load(name, Some(&bytes)),
        }
    }
    fn limited_png(&self, name: &str, max: u64) -> Result<Vec<u8>> {
        let path = if name.contains(':') {
            crate::atlas_sources::texture_resource(name, "textures")
        } else {
            name.to_owned()
        };
        Ok(self.source.read_limited(&path, max)?)
    }
    /// Like `insert`, but refuses unreadable or oversized PNG data before decoding.
    pub fn insert_limited(&mut self, name: &str, max: u64) -> Result<u32> {
        if self.dropped.contains(name) {
            return Ok(self.overflow_layer());
        }
        let bytes = self.limited_png(name, max)?;
        if let Some(layer) = self.layers.get(name) {
            return Ok(*layer);
        }
        let loaded = self.load(name, Some(&bytes));
        self.or_missing(name, loaded)
    }
    /// The layer of a texture, added on first use. A texture that is missing
    /// or does not load (a resource pack's filter or a broken file) becomes
    /// vanilla's missing texture, as in a stitched atlas.
    pub fn insert(&mut self, name: &str) -> Result<u32> {
        if let Some(layer) = self.layers.get(name) {
            return Ok(*layer);
        }
        if self.dropped.contains(name) {
            return Ok(self.overflow_layer());
        }
        let loaded = self.load(name, None);
        self.or_missing(name, loaded)
    }
    fn or_missing(&mut self, name: &str, loaded: Result<u32>) -> Result<u32> {
        match loaded {
            Ok(layer) => Ok(layer),
            Err(e) => {
                // `MissingTextureAtlasSprite.getLocation()`: vanilla models name the
                // missing texture on purpose (block/air.json's particle), #853.
                if name != "minecraft:missingno" {
                    log::warn!("Horizon: using the missing texture for {name}: {e}");
                }
                // Past the renderer's layers, the shared missing texture.
                let layer = if self.full() {
                    self.overflow_layer()
                } else {
                    self.push_missing()?
                };
                self.layers.insert(name.to_owned(), layer);
                Ok(layer)
            }
        }
    }
    /// A new layer of vanilla's missing texture (`MissingTextureAtlasSprite.generateMissingImage`:
    /// 16x16, magenta and black quarters), under no name.
    fn push_missing(&mut self) -> Result<u32> {
        let frame: Vec<u8> = (0..SPRITE_SIZE * SPRITE_SIZE)
            .flat_map(|i| {
                let (x, y) = (i % SPRITE_SIZE, i / SPRITE_SIZE);
                if (y < SPRITE_SIZE / 2) ^ (x < SPRITE_SIZE / 2) {
                    [248, 0, 248, 255]
                } else {
                    [0, 0, 0, 255]
                }
            })
            .collect();
        let layer = u32::try_from(self.native.len())?;
        self.opaque.push(true);
        self.pbr.push([None, None]);
        self.translucent.push(false);
        self.mips.push(crate::mipmap::Settings::default());
        self.native.push((SPRITE_SIZE, Arc::new(vec![frame]), 0));
        Ok(layer)
    }
    /// Whether one more layer would pass [`max_layers`] (keeping room for the shared missing
    /// texture's own).
    fn full(&self) -> bool {
        self.native.len() + usize::from(self.overflow_layer.is_none()) >= max_layers()
    }
    /// Fit a loader's textures (`names`, about to be inserted) within [`max_layers`], with
    /// room left for later loaders: past it the GPU arrays can't hold them and the game
    /// would not start. The mod namespaces with the most new textures give theirs up first,
    /// in reverse name order, until the rest fit; the missing texture stands in for them.
    /// Vanilla's textures are never given up. A pack that fits is left as it is.
    pub fn plan_within_capacity<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) {
        self.plan_within(names, max_layers().saturating_sub(LATE_LAYERS));
    }
    fn plan_within<'a>(&mut self, names: impl IntoIterator<Item = &'a str>, capacity: usize) {
        let new: std::collections::BTreeSet<&str> = names
            .into_iter()
            .filter(|n| !self.layers.contains_key(*n) && !self.dropped.contains(*n))
            .collect();
        let mut over = (self.native.len() + new.len()).saturating_sub(capacity);
        if over == 0 {
            return;
        }
        // The missing texture's own layer.
        over += usize::from(self.overflow_layer.is_none());
        let mut by_namespace: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for name in &new {
            match name.split_once(':') {
                Some((ns, _)) if ns != "minecraft" => {
                    by_namespace.entry(ns).or_default().push(name)
                }
                _ => {}
            }
        }
        let mut namespaces: Vec<_> = by_namespace.into_iter().collect();
        namespaces.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));
        let mut given_up = Vec::new();
        // Each name keeps a layer, the missing texture's: geometry that finds its sprites by
        // name (`sprite_layers`, a native mod's section geometry) draws the checkerboard
        // instead of leaving the face out.
        let missing = self.overflow_layer();
        for (ns, names) in namespaces {
            if over == 0 {
                break;
            }
            let take = names.len().min(over);
            for name in names.iter().rev().take(take) {
                self.dropped.insert((*name).to_owned());
                self.layers.insert((*name).to_owned(), missing);
            }
            over -= take;
            given_up.push(format!("{ns} ({take})"));
        }
        log::warn!(
            "Horizon: block textures past the {} layers the renderer draws from; \
             the missing texture stands in for {}",
            max_layers(),
            given_up.join(", ")
        );
    }
    /// The missing texture layer the textures past [`max_layers`] share, under no name of its
    /// own.
    fn overflow_layer(&mut self) -> u32 {
        if let Some(layer) = self.overflow_layer {
            return layer;
        }
        let layer = self.push_missing().unwrap_or(0);
        self.overflow_layer = Some(layer);
        layer
    }
    /// Decode on every thread the textures about to be inserted: sprite names, or resource
    /// paths outside `textures/` (as [`Sprites::insert_file`] takes them). A loader that adds
    /// many (the block models, an OptiFine pack's tiles) calls this first and then inserts
    /// them in its own order, so the layers are the same as without it. What is already a
    /// layer or in the disk cache is skipped; what fails to decode is left for the insert,
    /// which reports it.
    pub fn prefetch<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) {
        let names: Vec<&str> = names
            .into_iter()
            .filter(|name| {
                !self.layers.contains_key(*name)
                    && !self.dropped.contains(*name)
                    && !self.ready.contains_key(*name)
                    && self.disk.as_ref().is_none_or(|d| d.get(name).is_none())
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let decoded = crate::par::par_map(&names, |name| {
            if name.contains(':') {
                Self::decode(&self.source, name).ok()
            } else {
                Self::decode_file(&self.source, name, name).ok()
            }
        });
        for (name, decoded) in names.into_iter().zip(decoded) {
            if let Some(decoded) = decoded {
                self.ready.insert(name.to_owned(), decoded);
            }
        }
    }
    fn load(&mut self, name: &str, png: Option<&[u8]>) -> Result<u32> {
        // Custom Item Textures' files are outside `textures/`, which the disk cache's
        // fingerprint covers: like `insert_file`'s, they skip it.
        let cit = name.starts_with("horizon_cit:") || !name.contains(':');
        let disk = self.disk.as_mut().filter(|_| !cit);
        let cached = disk.as_ref().and_then(|d| d.get(name)).cloned();
        let decoded = match cached {
            Some(decoded) => decoded,
            None => {
                let decoded = match self.ready.remove(name) {
                    Some(decoded) => decoded,
                    None => match png {
                        Some(bytes) => {
                            let path = if name.contains(':') {
                                crate::atlas_sources::texture_resource(name, "textures")
                            } else {
                                name.to_owned()
                            };
                            Self::decode_file_data(&self.source, &path, name, Some(bytes))?
                        }
                        None => Self::decode(&self.source, name)?,
                    },
                };
                if let Some(disk) = disk {
                    disk.insert(name, decoded.clone());
                }
                decoded
            }
        };
        self.register(name, decoded)
    }
    /// The layer of a texture outside `textures/` (an OptiFine pack's connected-texture tile),
    /// named by its resource path and added on first use. Unlike [`Sprites::insert`], a texture
    /// that does not load is an error: the caller drops what needed it. Such files bypass the
    /// disk cache, whose fingerprint covers only `textures/`.
    pub fn insert_file(&mut self, resource: &str) -> Result<u32> {
        if let Some(layer) = self.layers.get(resource) {
            return Ok(*layer);
        }
        let decoded = match self.ready.remove(resource) {
            Some(decoded) => decoded,
            None => Self::decode_file(&self.source, resource, resource)?,
        };
        self.register(resource, decoded)
    }
    /// The tiles of the texture `name` (`ns:path`) cut into a `columns` x `rows` grid, row by
    /// row from the top left: a Fusion connecting texture's sheet. Each tile is its own layer
    /// (named `fusion/<ns>/<path>#<tile>`); a fully transparent one is `None`, as Fusion skips
    /// empty tiles. Tiles are square, as wide as a column. An animated sheet's frames are whole
    /// grids one below the other, or with `per_tile` each cell is a column of its tile's frames.
    /// The sheet's own layer, if models already show it, shows its first tile from then on.
    pub fn insert_tiles(
        &mut self,
        name: &str,
        (columns, rows): (u32, u32),
        per_tile: bool,
    ) -> Result<Vec<Option<u32>>> {
        let path = crate::atlas_sources::texture_resource(name, "textures");
        let metadata = match self.source.read_to_string(&format!("{path}.mcmeta")) {
            Ok(text) => Some(crate::packs::first_json::<
                BTreeMap<String, Box<serde_json::value::RawValue>>,
            >(text.as_bytes())?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let animation = metadata
            .as_ref()
            .and_then(|m| m.get("animation"))
            .map(|v| v.get().to_owned());
        // The sheet's `texture` section is each tile's.
        let texture = texture_section(metadata.as_ref());
        let strategy = texture
            .as_ref()
            .and_then(|t| t["mipmap_strategy"].as_str())
            .and_then(crate::mipmap::Strategy::parse)
            .unwrap_or_default();
        let alpha_cutoff_bias = texture
            .as_ref()
            .and_then(|t| t["alpha_cutoff_bias"].as_f64())
            .unwrap_or(0.0) as f32;
        let (width, height, pixels) = read_rgba(&self.source, &path)?;
        let side = width / columns.max(1);
        // How far down the next frame of a tile is, and the next row of tiles.
        let (frame_step, row_step) = match per_tile {
            true => (side, height / rows.max(1)),
            false => (rows * side, side),
        };
        let frames = match per_tile {
            true => row_step / side.max(1),
            false => height / frame_step.max(1),
        };
        let whole = match per_tile {
            true => height % rows.max(1) == 0 && row_step % side.max(1) == 0,
            false => height % frame_step.max(1) == 0,
        };
        if side == 0 || frames == 0 || width % columns.max(1) != 0 || !whole {
            return Err(format!("{name} isn't a {columns} x {rows} grid of square tiles").into());
        }
        let tiles = (0..columns * rows)
            .map(|tile| {
                let frames: Vec<Vec<u8>> = (0..frames)
                    .map(|f| {
                        let (x, y) = (
                            tile % columns * side,
                            tile / columns * row_step + f * frame_step,
                        );
                        (y..y + side)
                            .flat_map(|row| {
                                let start = ((row * width + x) * 4) as usize;
                                pixels[start..start + side as usize * 4].iter().copied()
                            })
                            .collect()
                    })
                    .collect();
                let alphas = || frames.iter().flat_map(|f| f.iter().skip(3).step_by(4));
                if alphas().all(|&a| a == 0) {
                    return Ok(None);
                }
                let decoded = crate::sprite_cache::Decoded {
                    side,
                    frame_size: [side, side],
                    animation: animation.clone().filter(|_| frames.len() > 1),
                    strategy,
                    alpha_cutoff_bias,
                    has_transparent: alphas().any(|&a| a == 0),
                    opaque: alphas().all(|&a| a == 255),
                    translucent: alphas().any(|&a| a != 0 && a != 255),
                    frames,
                };
                let tile_name = format!("fusion/{}#{tile}", name.replace(':', "/"));
                self.register(&tile_name, decoded).map(Some)
            })
            .collect::<Result<Vec<_>>>()?;
        // Where Fusion doesn't draw the sheet (items, other models), the lone tile rather than
        // the whole sheet squeezed square, whose size would set every layer's.
        if let (Some(&own), Some(&first)) = (self.layers.get(name), tiles.iter().flatten().next())
            && Some(own) != self.overflow_layer
        {
            self.show_as(own, first);
        }
        Ok(tiles)
    }
    /// Layer `layer` shows what `source` does, and is no longer the layer of its old pixels.
    fn show_as(&mut self, layer: u32, source: u32) {
        self.still.retain(|_, l| *l != layer);
        let (l, s) = (layer as usize, source as usize);
        self.native[l] = self.native[s].clone();
        match self.shapes.get(&source).copied() {
            Some(shape) => self.shapes.insert(layer, shape),
            None => self.shapes.remove(&layer),
        };
        self.opaque[l] = self.opaque[s];
        self.translucent[l] = self.translucent[s];
        self.mips[l] = self.mips[s];
        self.pbr[l] = self.pbr[s].clone();
        self.animations.retain(|a| a.layer != layer);
        if let Some(animation) = self.animations.iter().find(|a| a.layer == source) {
            let animation = SpriteAnimation {
                layer,
                ..animation.clone()
            };
            self.animations.push(animation);
        }
    }
    /// Read and decode one texture and its `.mcmeta`.
    fn decode(source: &ResourceSource, name: &str) -> Result<crate::sprite_cache::Decoded> {
        if !name.contains(':') {
            return Err("invalid sprite identifier".into());
        }
        // `texture_resource`, so Custom Item Textures' `horizon_cit:` sprites find their file.
        let path = crate::atlas_sources::texture_resource(name, "textures");
        Self::decode_file(source, &path, name)
    }
    fn decode_file(
        source: &ResourceSource,
        path: &str,
        name: &str,
    ) -> Result<crate::sprite_cache::Decoded> {
        Self::decode_file_data(source, path, name, None)
    }
    fn decode_file_data(
        source: &ResourceSource,
        path: &str,
        name: &str,
        png: Option<&[u8]>,
    ) -> Result<crate::sprite_cache::Decoded> {
        let metadata = match source.read_to_string(&format!("{path}.mcmeta")) {
            Ok(text) => Some(crate::packs::first_json::<
                BTreeMap<String, Box<serde_json::value::RawValue>>,
            >(text.as_bytes())?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let animation_text = metadata
            .as_ref()
            .and_then(|v| v.get("animation"))
            .map(|v| v.get().to_owned());
        let animation = animation_text
            .as_deref()
            .map(AnimationMetadata::from_json)
            .transpose()?;
        let (png_width, png_height, pixels) = match png {
            Some(bytes) => decode_rgba(bytes)?,
            None => read_rgba(source, path)?,
        };
        let (width, height) = if let Some(animation) = &animation {
            let [width, height] =
                animation.frame_size(i32::try_from(png_width)?, i32::try_from(png_height)?);
            (u32::try_from(width)?, u32::try_from(height)?)
        } else {
            (png_width, png_height)
        };
        if width == 0
            || height == 0
            || !png_width.is_multiple_of(width)
            || !png_height.is_multiple_of(height)
        {
            return Err(format!("sprite {name} has invalid frame dimensions").into());
        }
        let columns = png_width / width;
        let count = columns * (png_height / height);
        // Layers are square; a frame of another shape is nearest-resampled to
        // its longer side so UVs (normalized per layer) are unchanged. That square is
        // made only at the layers' size (`lay_out`).
        let side = width.max(height);
        let mut has_transparent = false;
        let mut opaque = true;
        let mut translucent = false;
        for p in pixels.as_chunks::<4>().0 {
            let a = p[3];
            if a == 0 {
                has_transparent = true;
                opaque = false;
            } else if a != 255 {
                translucent = true;
                opaque = false;
            }
            if has_transparent && translucent {
                break;
            }
        }
        // The frames keep their own shape: together exactly the decoded image's pixels.
        let frames = if count == 1 {
            vec![pixels]
        } else if columns == 1 {
            let frame_bytes = width as usize * height as usize * 4;
            pixels
                .chunks_exact(frame_bytes)
                .take(count as usize)
                .map(<[u8]>::to_vec)
                .collect()
        } else {
            let mut frames = Vec::with_capacity(count as usize);
            let row_bytes = width as usize * 4;
            let stride = png_width as usize * 4;
            for index in 0..count {
                let x = (index % columns * width) as usize * 4;
                let y = (index / columns * height) as usize;
                let mut frame = Vec::with_capacity(width as usize * height as usize * 4);
                for row in 0..height as usize {
                    let start = (y + row) * stride + x;
                    frame.extend_from_slice(&pixels[start..start + row_bytes]);
                }
                frames.push(frame);
            }
            frames
        };
        let texture = texture_section(metadata.as_ref());
        Ok(crate::sprite_cache::Decoded {
            side,
            frame_size: [width, height],
            frames,
            animation: animation_text,
            strategy: texture
                .as_ref()
                .and_then(|t| t["mipmap_strategy"].as_str())
                .and_then(crate::mipmap::Strategy::parse)
                .unwrap_or_default(),
            alpha_cutoff_bias: texture
                .as_ref()
                .and_then(|t| t["alpha_cutoff_bias"].as_f64())
                .unwrap_or(0.0) as f32,
            has_transparent,
            opaque,
            translucent,
        })
    }
    /// Give a decoded texture its layer (or the one an identical still has).
    #[horizon_mod_api_macros::replaceable]
    fn register(&mut self, name: &str, decoded: crate::sprite_cache::Decoded) -> Result<u32> {
        let crate::sprite_cache::Decoded {
            side,
            frame_size,
            frames,
            animation: animation_text,
            strategy,
            alpha_cutoff_bias,
            has_transparent,
            opaque,
            translucent,
        } = decoded;
        let animation = animation_text
            .as_deref()
            .map(AnimationMetadata::from_json)
            .transpose()?;
        let count = frames.len() as u32;
        let layer = u32::try_from(self.native.len())?;
        let pbr = match name.split_once(':') {
            Some(_) if labpbr_maps() => {
                let path = crate::atlas_sources::texture_resource(name, "textures");
                ["_n", "_s"].map(|suffix| {
                    pbr_map(&self.source, &path, suffix, count, animation.as_ref())
                        .map(|(side, frames)| (side, Arc::new(frames)))
                })
            }
            None if labpbr_maps() && !name.starts_with("fusion/") => ["_n", "_s"].map(|suffix| {
                pbr_map(&self.source, name, suffix, count, animation.as_ref())
                    .map(|(side, frames)| (side, Arc::new(frames)))
            }),
            _ => [None, None],
        };
        let settings = crate::mipmap::Settings {
            native_size: side,
            strategy,
            alpha_cutoff_bias,
            has_transparent,
            // Custom Item Textures' ids are all item textures.
            item: name.starts_with("horizon_cit:")
                || name
                    .split_once(':')
                    .is_some_and(|(_, p)| p.starts_with("item/")),
            dither_cutouts: crate::mipmap::dither_cutouts(),
        };
        let still =
            (animation.is_none() && frames.len() == 1 && !self.apart.contains(name)).then(|| {
                use std::hash::{Hash, Hasher};
                let mut frame = std::collections::hash_map::DefaultHasher::new();
                hash_square(&frames[0], frame_size, &mut frame);
                let mut maps = std::collections::hash_map::DefaultHasher::new();
                pbr.hash(&mut maps);
                StillKey {
                    frame: frame.finish(),
                    pbr: maps.finish(),
                    side,
                    settings: [
                        settings.strategy as u32,
                        settings.alpha_cutoff_bias.to_bits(),
                    ],
                    flags: [settings.has_transparent, settings.item, opaque, translucent],
                }
            });
        if let Some(&same) = still.as_ref().and_then(|k| self.still.get(k)) {
            self.layers.insert(name.to_owned(), same);
            return Ok(same);
        }
        // Past what the renderer draws from, a texture no plan foresaw shows missing too
        // (with room for the missing texture's own layer).
        if self.full() {
            let layer = self.overflow_layer();
            self.layers.insert(name.to_owned(), layer);
            return Ok(layer);
        }
        if let Some(key) = still {
            self.still.insert(key, layer);
        }
        let frames = Arc::new(frames);
        let mut initial = 0;
        if let Some(clip) = animation
            .as_ref()
            .map(|m| m.clip(count))
            .transpose()?
            .flatten()
        {
            initial = clip.sample(0).frame as usize;
            self.animations.push(SpriteAnimation {
                layer,
                frames: AnimationFrames::Raw(frames.clone()),
                clip: Arc::new(clip),
                materials: [None, None],
                shape: (frame_size != [side, side]).then_some(frame_size),
            });
        }
        self.mips.push(settings);
        self.opaque.push(opaque);
        self.translucent.push(translucent);
        self.pbr.push(pbr);
        self.layers.insert(name.to_owned(), layer);
        self.native.push((side, frames, initial));
        if frame_size != [side, side] {
            self.shapes.insert(layer, frame_size);
        }
        Ok(layer)
    }
}

/// Refuses `count` square frames of `side` before they are made: a frame of any shape becomes a
/// square layer, so a 32768x1 image of a few hundred bytes squared at its own side would ask for
/// 4 GiB. A side may be at most [`MAX_ATLAS_SIZE`](crate::hud::MAX_ATLAS_SIZE), and all `count`
/// squares together at most a full atlas, the ceiling [`crate::hud::read_png`] puts on the image.
pub(crate) fn check_frames(side: u32, count: u32) -> std::result::Result<(), String> {
    let atlas = u128::from(crate::hud::MAX_ATLAS_SIZE);
    let texels = u128::from(count) * u128::from(side) * u128::from(side);
    if u128::from(side) > atlas || texels > atlas * atlas {
        return Err(format!(
            "{count} frames of {side}x{side} are larger than any atlas"
        ));
    }
    Ok(())
}

/// A PNG's width, height and pixels as RGBA. From the prepared data file
/// when it holds the texture: no file opened per texture, nor for its absent
/// LabPBR maps.
fn read_rgba(source: &ResourceSource, path: &str) -> Result<(u32, u32, Vec<u8>)> {
    decode_rgba(&source.read(path)?)
}
fn read_png_dimensions(source: &ResourceSource, path: &str) -> Result<(u32, u32)> {
    let bytes = source.read(path)?;
    let mut decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
    decoder.ignore_checksums(true);
    let (width, height) = decoder.read_header_info()?.size();
    let atlas = u64::from(crate::hud::MAX_ATLAS_SIZE);
    if u64::from(width) * u64::from(height) > atlas * atlas {
        return Err(
            format!("a {width}x{height} PNG is larger than any texture it could fill").into(),
        );
    }
    Ok((width, height))
}
pub(crate) fn decode_rgba(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let (mut reader, mut bytes) = crate::hud::read_png(bytes)?;
    let info = reader.next_frame(&mut bytes)?;
    if info.color_type == png::ColorType::Rgba {
        bytes.truncate(info.buffer_size());
        return Ok((info.width, info.height, bytes));
    }
    let raw = &bytes[..info.buffer_size()];
    let mut pixels = Vec::with_capacity(info.width as usize * info.height as usize * 4);
    match info.color_type {
        png::ColorType::Rgb => {
            for pixel in raw.as_chunks::<3>().0 {
                pixels.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        png::ColorType::Grayscale => {
            for &g in raw {
                pixels.extend_from_slice(&[g, g, g, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for pixel in raw.as_chunks::<2>().0 {
                pixels.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        _ => return Err("unsupported expanded sprite format".into()),
    }
    Ok((info.width, info.height, pixels))
}

/// Companion frames use the colour image's grid; a single-frame map stays static.
fn pbr_map(
    source: &ResourceSource,
    path: &str,
    suffix: &str,
    count: u32,
    animation: Option<&AnimationMetadata>,
) -> Option<(u32, Vec<Vec<u8>>)> {
    let stem = path.strip_suffix(".png")?;
    let map = format!("{stem}{suffix}.png");
    let (map_width, map_height, pixels) = read_rgba(source, &map).ok()?;
    let (width, height) = read_png_dimensions(source, path).ok()?;
    let [frame_width, frame_height] = animation.map_or([width as i32, height as i32], |a| {
        a.frame_size(width as i32, height as i32)
    });
    let (frame_width, frame_height) = (
        u32::try_from(frame_width).ok()?,
        u32::try_from(frame_height).ok()?,
    );
    if frame_width == 0 || frame_height == 0 {
        return None;
    }
    let (columns, rows) = if count <= 1 || (map_width == frame_width && map_height == frame_height)
    {
        (1, 1)
    } else if u64::from(map_width) * u64::from(height) == u64::from(map_height) * u64::from(width) {
        (width / frame_width, height / frame_height)
    } else if u64::from(map_width) * u64::from(frame_height)
        == u64::from(map_height) * u64::from(frame_width)
    {
        (1, 1)
    } else {
        return None;
    };
    if columns == 0
        || rows == 0
        || !map_width.is_multiple_of(columns)
        || !map_height.is_multiple_of(rows)
    {
        return None;
    }
    let (w, h) = (map_width / columns, map_height / rows);
    if w == 0 || h == 0 {
        return None;
    }
    let side = w.max(h);
    let output_side = side_within(side, LAYERS_AT_LEAST, max_map_bytes()).min(side);
    let total = columns * rows;
    // Only the frames at `output_side` are made.
    check_frames(output_side, total).ok()?;
    let frames = if total == 1 && w == output_side && h == output_side {
        vec![pixels]
    } else {
        (0..total)
            .map(|index| {
                let (x, y) = (index % columns * w, index / columns * h);
                region_layer(&pixels, map_width, [x, y, w, h], output_side)
            })
            .collect()
    };
    Some((output_side, frames))
}
/// SpriteContents: the `.mcmeta` `texture` section (TextureMetadataSection's
/// mipmap strategy and alpha cutoff bias).
fn texture_section(
    metadata: Option<&BTreeMap<String, Box<serde_json::value::RawValue>>>,
) -> Option<serde_json::Value> {
    metadata
        .and_then(|v| v.get("texture"))
        .and_then(|v| serde_json::from_str(v.get()).ok())
}

#[cfg(test)]
pub(crate) fn test_animation(size: u32, layer: u32) -> SpriteAnimation {
    test_clip(size, layer, r#"{"frametime":1,"frames":[0,1]}"#)
}
#[cfg(test)]
pub(crate) fn test_clip(size: u32, layer: u32, metadata: &str) -> SpriteAnimation {
    let frame = |blue: u8| {
        (0..size * size)
            .flat_map(|i| [(i % size) as u8, (i / size) as u8, blue, 255])
            .collect()
    };
    test_frames(layer, vec![frame(0), frame(99)], metadata)
}
#[cfg(test)]
pub(crate) fn test_frames(layer: u32, frames: Vec<Vec<u8>>, metadata: &str) -> SpriteAnimation {
    let metadata = AnimationMetadata::from_json(metadata).unwrap();
    SpriteAnimation {
        layer,
        clip: Arc::new(metadata.clip(frames.len() as u32).unwrap().unwrap()),
        frames: AnimationFrames::Raw(Arc::new(frames)),
        materials: [None, None],
        shape: None,
    }
}

#[cfg(test)]
pub(crate) fn test_material_frames(
    frames: Vec<Vec<u8>>,
    metadata: &str,
    materials: [Option<Vec<Vec<u8>>>; 2],
) -> SpriteAnimation {
    let mut animation = test_frames(0, frames, metadata);
    animation.materials = materials.map(|m| m.map(Arc::new));
    animation
}

/// How many reduced copies a sprite atlas keeps: the Mipmap Levels option plus the
/// resampling-only levels an upscaled pack needs, within what the atlas holds.
pub(crate) fn sprite_mipmap_limit(requested: u32, size: u32, minimum: u32, available: u32) -> u32 {
    // Only the atlas native minimum determines how many resampling-only mips to skip.
    let upscale = (size / minimum.max(1)).max(1).ilog2();
    requested.saturating_add(upscale).min(available - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Past the arrays' layers, the namespaces with the most new textures give theirs up
    /// first and share the missing texture, under their own names; vanilla's are never
    /// given up.
    #[test]
    fn textures_past_the_layers_give_way_by_namespace() {
        let mut sprites = Sprites::default();
        let names = [
            "big:block/a",
            "big:block/b",
            "big:block/c",
            "small:block/x",
            "minecraft:block/stone",
        ];
        sprites.plan_within(names, 4);
        // Five new and four fit, one of them the missing texture's: two give way.
        let dropped: std::collections::BTreeSet<_> = sprites.dropped.iter().cloned().collect();
        assert_eq!(
            dropped,
            ["big:block/b".to_owned(), "big:block/c".to_owned()].into()
        );
        let missing = sprites.insert("big:block/c").unwrap();
        assert_eq!(sprites.insert("big:block/b").unwrap(), missing);
        assert_eq!(sprites.count(), 1);
        // Found by name too, as geometry that never inserts looks them up.
        assert_eq!(sprites.layers.get("big:block/b"), Some(&missing));
        assert_eq!(sprites.layers.get("big:block/a"), None);
        // A load that fits gives nothing up.
        let mut fits = Sprites::default();
        fits.plan_within(names, 5);
        assert!(fits.dropped.is_empty());
    }
    use std::fs::{self, File};
    #[test]
    fn archive_sprites_keep_deferred_reads_and_cache_on_their_reload_snapshot() -> Result<()> {
        use std::io::Write;
        let root =
            std::env::temp_dir().join(format!("horizon-archive-sprites-{}", std::process::id()));
        fs::create_dir_all(&root)?;
        let write = |path: &Path, red: u8| -> Result<()> {
            let mut bytes = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut bytes, 2, 4);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                let pixels = [[red, 0, 0, 255].repeat(4), [0, red, 0, 255].repeat(4)].concat();
                encoder.write_header()?.write_image_data(&pixels)?;
            }
            let mut writer = zip::ZipWriter::new(File::create(path)?);
            for name in [
                "assets/demo/textures/block/flow.png",
                "assets/demo/textures/block/flow_n.png",
                "assets/demo/optifine/cit/flow.png",
                "assets/demo/optifine/ctm/flow.png",
            ] {
                writer.start_file(name, zip::write::SimpleFileOptions::default())?;
                writer.write_all(&bytes)?;
                writer.start_file(
                    format!("{name}.mcmeta"),
                    zip::write::SimpleFileOptions::default(),
                )?;
                writer.write_all(br#"{"animation":{"frametime":3,"frames":[1,0],"interpolate":true},"texture":{"alpha_cutoff_bias":0.25}}"#)?;
            }
            writer.finish()?;
            Ok(())
        };
        let jar = root.join("textures.jar");
        write(&jar, 10)?;
        let before = ResourceRoot::new(
            root.join("prepared"),
            root.join("cache"),
            ResourceSource::archive(&jar)?,
        );
        let mut sprites = Sprites::with_disk_cache(&before);
        sprites.prefetch(["demo:block/flow"]);
        let replacement = root.join("replacement.jar");
        write(&replacement, 20)?;
        fs::rename(&jar, root.join("old.jar"))?;
        fs::rename(replacement, &jar)?;
        // Insertion after replacement uses the retained source, both prefetched
        // textures and deferred reads outside textures/ (which bypass the cache).
        let layer = sprites.try_insert("demo:block/flow")?;
        assert_eq!(sprites.native[layer as usize].1[0][..4], [10, 0, 0, 255]);
        assert_eq!(sprites.animations[0].frame(0), 1);
        assert_eq!(sprites.mips[layer as usize].alpha_cutoff_bias, 0.25);
        for name in [
            "horizon_cit:demo/optifine/cit/flow",
            "assets/demo/optifine/ctm/flow.png",
        ] {
            let layer = if name.contains(':') {
                sprites.try_insert(name)?
            } else {
                sprites.insert_file(name)?
            };
            assert_eq!(sprites.native[layer as usize].1[0][..4], [10, 0, 0, 255]);
        }
        let (_, frames) = pbr_map(
            before.source(),
            "assets/demo/textures/block/flow.png",
            "_n",
            2,
            None,
        )
        .unwrap();
        assert_eq!(frames[0][..4], [10, 0, 0, 255]);
        let mut sheet = Sprites::new(before.source().clone());
        let tiles = sheet.insert_tiles("demo:block/flow", (1, 1), false)?;
        assert_eq!(sheet.native[tiles[0].unwrap() as usize].1.len(), 2);
        sprites.save_disk_cache();
        let mut cached = Sprites::with_disk_cache(&before);
        cached.prefetch(["demo:block/flow"]);
        assert!(
            cached.ready.is_empty(),
            "the retained snapshot hits the disk cache"
        );
        let layer = cached.try_insert("demo:block/flow")?;
        assert_eq!(cached.native[layer as usize].1[0][0], 10);
        let after = ResourceRoot::new(
            root.join("prepared"),
            root.join("cache"),
            ResourceSource::archive(&jar)?,
        );
        let mut reloaded = Sprites::with_disk_cache(&after);
        let layer = reloaded.try_insert("demo:block/flow")?;
        assert_eq!(reloaded.native[layer as usize].1[0][0], 20);
        assert!(!root.join("assets").exists());
        assert!(!root.join("prepared").exists());
        drop((sprites, sheet, cached, reloaded, before, after));
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn native_frames_at_atlas_size_are_shared_without_resampling() {
        let pixels = vec![
            [10, 20, 30, 255].repeat((SPRITE_SIZE * SPRITE_SIZE) as usize),
            [40, 50, 60, 255].repeat((SPRITE_SIZE * SPRITE_SIZE) as usize),
        ];
        let native = Arc::new(pixels.clone());
        let smaller = Arc::new(vec![vec![12; 8 * 8 * 4], vec![34; 8 * 8 * 4]]);
        let metadata = r#"{"frametime":2}"#;
        let mut sprites = Sprites {
            native: vec![
                (SPRITE_SIZE, Arc::clone(&native), 1),
                (8, Arc::clone(&smaller), 0),
            ],
            pbr: vec![[None, None], [None, None]],
            animations: vec![
                test_frames(0, pixels.clone(), metadata),
                test_frames(1, smaller.as_ref().clone(), metadata),
            ],
            ..Default::default()
        };
        sprites.finish();
        // Both animations share their layers' decoded frames; a smaller one shows them
        // resampled to the layers' size.
        assert!(Arc::ptr_eq(
            sprites.animations[0].frames.raw_arc().unwrap(),
            &native
        ));
        assert!(Arc::ptr_eq(
            sprites.animations[1].frames.raw_arc().unwrap(),
            &smaller
        ));
        assert_eq!(sprites.animations[0].pixels(1), pixels[1].as_slice());
        let resized = resample(&smaller[0], 8, SPRITE_SIZE);
        assert_eq!(sprites.animations[1].frame_at(0, SPRITE_SIZE), resized);
        assert_eq!(&sprites.pixels[..pixels[1].len()], pixels[1].as_slice());
    }
    #[test]
    fn the_texture_budget_picks_the_largest_side_that_fits() {
        // 1300 layers: vanilla's count, about.
        assert_eq!(side_within(512, 1300, DEFAULT_TEXTURE_BUDGET), 128);
        assert_eq!(side_within(512, 1300, 1536 << 20), 512);
        assert_eq!(side_within(64, 1300, 1536 << 20), 64);
        // Never under vanilla's size, whatever the budget.
        assert_eq!(side_within(512, 1300, 1), SPRITE_SIZE);
    }
    #[test]
    fn labpbr_maps_follow_the_texture_budget() {
        // 1300 layers of 1024 px maps: 64 px at the default budget, 256 px at 1536 MiB.
        assert_eq!(
            side_within(1024, 1300, map_bytes(DEFAULT_TEXTURE_BUDGET)),
            64
        );
        assert_eq!(side_within(1024, 1300, map_bytes(1536 << 20)), 256);
    }
    #[test]
    fn a_labpbr_map_selects_whole_texels_and_missing_layers_get_defaults() {
        // Selecting a texel preserves encoded channels, including categorical values.
        let frame = [
            [200, 100, 0, 255],
            [0, 0, 0, 0],
            [200, 100, 0, 255],
            [0, 0, 0, 0],
        ]
        .concat();
        assert_eq!(shrink_map(frame, 2, 1), (1, vec![200, 100, 0, 255]));
        let mut sprites = Sprites {
            native: vec![
                (SPRITE_SIZE, Arc::new(vec![vec![0; 16 * 16 * 4]]), 0),
                (SPRITE_SIZE, Arc::new(vec![vec![0; 16 * 16 * 4]]), 0),
            ],
            pbr: vec![
                [
                    Some((16, Arc::new(vec![[9, 8, 7, 6].repeat(16 * 16)]))),
                    None,
                ],
                [None, None],
            ],
            ..Default::default()
        };
        sprites.finish();
        assert_eq!(sprites.pbr_size, 16);
        assert_eq!(sprites.normals[..4], [9, 8, 7, 6]);
        assert_eq!(sprites.normals[16 * 16 * 4..][..4], FLAT_NORMAL);
        assert_eq!(sprites.specular[..4], NO_SPECULAR);
        assert_eq!(sprites.specular.len(), 2 * 16 * 16 * 4);
    }
    /// Companion maps are kept like colour frames: a 16 px map's frames stay at 16 px beside a
    /// 128 px map, and only the initial frame is made at the shared size. A sheet of 16448
    /// such frames would be 16448 x 128^2 x 4 = 1,028 MiB per map resampled up front, against
    /// 16448 x 16^2 x 4 = 16 MiB as stored; the material animator resamples the one frame it
    /// shows (`gfx::material_animate`).
    #[test]
    fn companion_frames_keep_their_stored_size() {
        assert_eq!(16_448u64 * 128 * 128 * 4, 1_028 << 20);
        assert_eq!(16_448u64 * 16 * 16 * 4, 16_842_752);
        let maps: Vec<Vec<u8>> = (0..4u8).map(|i| [i, 1, 2, 255].repeat(16 * 16)).collect();
        let mut sprites = Sprites {
            native: vec![
                (16, Arc::new(vec![vec![0; 16 * 16 * 4]; 4]), 2),
                (128, Arc::new(vec![vec![0; 128 * 128 * 4]]), 0),
            ],
            pbr: vec![
                [None, Some((16, Arc::new(maps.clone())))],
                [
                    None,
                    Some((128, Arc::new(vec![[7, 7, 7, 255].repeat(128 * 128)]))),
                ],
            ],
            animations: vec![test_frames(
                0,
                vec![vec![0; 16 * 16 * 4]; 4],
                r#"{"frametime":1}"#,
            )],
            ..Default::default()
        };
        sprites.finish();
        assert_eq!(sprites.pbr_size, 128);
        assert_eq!(
            sprites.animations[0].material_frames(1),
            Some(maps.as_slice())
        );
        assert_eq!(sprites.animations[0].material_frames(0), None);
        assert_eq!(
            sprites.specular[..128 * 128 * 4],
            resample(&maps[2], 16, 128)
        );
    }
    #[test]
    fn companion_grid_and_static_map_follow_colour_frame_selection() {
        let root =
            std::env::temp_dir().join(format!("horizon-companion-grid-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let write = |name: &str, width, height, pixels: &[u8]| {
            let mut encoder =
                png::Encoder::new(File::create(root.join(name)).unwrap(), width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(pixels)
                .unwrap();
        };
        write("grid.png", 4, 4, &[200, 200, 200, 255].repeat(16));
        let maps: Vec<u8> = (0..16)
            .flat_map(|i| {
                let cell = (i / 8) * 2 + (i % 4) / 2;
                [10 + cell as u8, 230, 65, 255]
            })
            .collect();
        write("grid_s.png", 4, 4, &maps);
        write("grid_n.png", 2, 2, &[128, 128, 255, 255].repeat(4));
        let metadata =
            r#"{"width":2,"height":2,"frametime":2,"interpolate":true,"frames":[3,1,2,0]}"#;
        let animation = AnimationMetadata::from_json(metadata).unwrap();
        let (side, frames) = pbr_map(
            &ResourceSource::directory(root.clone()),
            "grid.png",
            "_s",
            4,
            Some(&animation),
        )
        .unwrap();
        assert_eq!(side, 2);
        assert_eq!(
            frames.iter().map(|f| f[0]).collect::<Vec<_>>(),
            [10, 11, 12, 13]
        );
        let (_, normals) = pbr_map(
            &ResourceSource::directory(root.clone()),
            "grid.png",
            "_n",
            4,
            Some(&animation),
        )
        .unwrap();
        assert_eq!(normals.len(), 1);
        let mut sprite = test_frames(0, vec![vec![0; 16]; 4], metadata);
        sprite.materials = [Some(Arc::new(normals)), Some(Arc::new(frames))];
        for (tick, expected) in [(0, 13), (1, 13), (2, 11), (4, 12), (6, 10)] {
            assert_eq!(sprite.material_pixels(1, tick).unwrap()[0], expected);
            assert_eq!(&sprite.material_pixels(0, tick).unwrap()[..4], &FLAT_NORMAL);
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn textures_without_labpbr_maps_keep_none() {
        let mut sprites = Sprites {
            native: vec![(SPRITE_SIZE, Arc::new(vec![vec![0; 16 * 16 * 4]]), 0)],
            pbr: vec![[None, None]],
            ..Default::default()
        };
        sprites.finish();
        assert!(sprites.normals.is_empty() && sprites.specular.is_empty());
    }
    #[test]
    fn mipmap_option_keeps_real_16px_reductions() {
        for levels in 0..=4 {
            assert_eq!(sprite_mipmap_limit(levels, 32, 16, 6), levels + 1);
            assert_eq!(sprite_mipmap_limit(levels, 16, 16, 5), levels);
        }
    }
    #[test]
    fn files_outside_textures_load_by_path() {
        let root = std::env::temp_dir().join(format!("horizon-sprite-file-{}", std::process::id()));
        let tile = "assets/minecraft/optifine/ctm/glass/0.png";
        fs::create_dir_all(root.join(tile).parent().unwrap()).unwrap();
        let mut encoder = png::Encoder::new(File::create(root.join(tile)).unwrap(), 16, 16);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels: Vec<u8> = (0..256).flat_map(|i| [i as u8, 0, 0, 128]).collect();
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        let layer = sprites.insert_file(tile).unwrap();
        assert_eq!(sprites.insert_file(tile).unwrap(), layer);
        assert!(sprites.translucent[layer as usize]);
        // A missing tile is an error, not the missing texture.
        assert!(
            sprites
                .insert_file("assets/minecraft/optifine/ctm/glass/1.png")
                .is_err()
        );
        assert_eq!(sprites.count(), 1);
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn tile_sheets_cut_into_a_layer_per_tile() {
        let root =
            std::env::temp_dir().join(format!("horizon-sprite-tiles-{}", std::process::id()));
        let file = root.join("assets/minecraft/textures/block/sheet.png");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        // 3 columns of 2 x 2 tiles over 4 rows of pixels: the pixel at (x, y) has red x, green
        // y; the middle column is transparent, and the top-right tile half see-through.
        let (w, h) = (6u32, 4u32);
        let pixels: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                let alpha = match x / 2 {
                    1 => 0,
                    2 if y < 2 => 128,
                    _ => 255,
                };
                [x as u8, y as u8, 0, alpha]
            })
            .collect();
        let mut encoder = png::Encoder::new(File::create(&file).unwrap(), w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        // Red and green of each pixel in a layer's frame.
        let texels = |sprites: &Sprites, layer: u32, frame: usize| -> Vec<(u8, u8)> {
            sprites.native[layer as usize].1[frame]
                .chunks(4)
                .map(|p| (p[0], p[1]))
                .collect()
        };
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        let name = "minecraft:block/sheet";
        let own = sprites.insert(name).unwrap();
        // Still, as a 3 x 2 grid: the empty middle column draws nothing.
        let tiles = sprites.insert_tiles(name, (3, 2), false).unwrap();
        // The sheet's own layer now shows the top-left tile, at a tile's size.
        assert_eq!(texels(&sprites, own, 0), [(0, 0), (1, 0), (0, 1), (1, 1)]);
        assert_eq!(
            tiles.iter().map(Option::is_some).collect::<Vec<_>>(),
            [true, false, true, true, false, true]
        );
        let bottom_left = tiles[3].unwrap();
        assert_eq!(
            texels(&sprites, bottom_left, 0),
            [(0, 2), (1, 2), (0, 3), (1, 3)]
        );
        assert!(sprites.translucent[tiles[2].unwrap() as usize]);
        assert!(sprites.opaque[bottom_left as usize]);
        // Animated, as one row of tiles: two frames, the grid's top and then its bottom half;
        // per tile, each cell is the column of a tile's frames, which here is the same.
        fs::write(file.with_extension("png.mcmeta"), r#"{"animation":{}}"#).unwrap();
        for per_tile in [false, true] {
            let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
            let tiles = sprites.insert_tiles(name, (3, 1), per_tile).unwrap();
            let first = tiles[0].unwrap();
            assert_eq!(texels(&sprites, first, 0), [(0, 0), (1, 0), (0, 1), (1, 1)]);
            assert_eq!(texels(&sprites, first, 1), [(0, 2), (1, 2), (0, 3), (1, 3)]);
            assert_eq!(sprites.animations.len(), 2, "two non-empty tiles");
        }
        // Not a grid of square tiles.
        assert!(sprites.insert_tiles(name, (4, 1), false).is_err());
        assert!(sprites.insert_tiles(name, (3, 3), false).is_err());
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn custom_item_texture_ids_load_with_their_animation() {
        let root = std::env::temp_dir().join(format!("horizon-sprite-cit-{}", std::process::id()));
        let file = root.join("assets/minecraft/optifine/cit/a.png");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let mut encoder = png::Encoder::new(File::create(&file).unwrap(), 16, 48);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels: Vec<u8> = (0..16 * 48).flat_map(|_| [200, 10, 10, 255]).collect();
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        fs::write(file.with_extension("png.mcmeta"), r#"{"animation":{}}"#).unwrap();
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        sprites
            .insert("horizon_cit:minecraft/optifine/cit/a")
            .unwrap();
        fs::remove_dir_all(&root).unwrap();
        // The missing texture would not animate: this one is the file, in three frames.
        assert_eq!(sprites.animations.len(), 1);
        assert_eq!(sprites.animations[0].frame_count(), 3);
    }
    #[test]
    fn custom_item_textures_skip_the_disk_cache() {
        let root =
            std::env::temp_dir().join(format!("horizon-sprite-cit-disk-{}", std::process::id()));
        // A realistic root: the cache's fingerprint walks the `textures` trees.
        let vanilla = root.join("assets/minecraft/textures/item/stick.png");
        let file = root.join("assets/minecraft/optifine/cit/a.png");
        let write = |path: &Path, red: u8| {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut encoder = png::Encoder::new(File::create(path).unwrap(), 16, 16);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let pixels: Vec<u8> = (0..256).flat_map(|_| [red, 0, 0, 255]).collect();
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        };
        write(&vanilla, 1);
        write(&file, 10);
        let id = "horizon_cit:minecraft/optifine/cit/a";
        let red = |sprites: &Sprites, layer: u32| sprites.native[layer as usize].1[0][0];
        let mut first = Sprites::with_disk_cache(&crate::resource_source::ResourceRoot::directory(
            root.clone(),
        ));
        let layer = first.insert(id).unwrap();
        assert_eq!(red(&first, layer), 10);
        first.save_disk_cache();
        // The pack's file changes; `textures/` does not, so a cached copy would still match.
        write(&file, 20);
        let mut second = Sprites::with_disk_cache(
            &crate::resource_source::ResourceRoot::directory(root.clone()),
        );
        let layer = second.insert(id).unwrap();
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(red(&second, layer), 20);
    }
    #[test]
    fn custom_item_textures_mip_as_items() {
        let root =
            std::env::temp_dir().join(format!("horizon-sprite-cit-mip-{}", std::process::id()));
        let write = |rel: &str| {
            let path = root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut encoder = png::Encoder::new(File::create(path).unwrap(), 16, 16);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            // Any pixels: whether a texture mips as an item is part of the shared-layer key.
            let pixels: Vec<u8> = (0..256)
                .flat_map(|i| [9, 9, 9, if i % 2 == 0 { 255 } else { 0 }])
                .collect();
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        };
        write("assets/a/textures/block/x.png");
        write("assets/a/textures/item/x.png");
        write("assets/minecraft/optifine/cit/x.png");
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        let block = sprites.insert("a:block/x").unwrap();
        let item = sprites.insert("a:item/x").unwrap();
        let cit = sprites
            .insert("horizon_cit:minecraft/optifine/cit/x")
            .unwrap();
        fs::remove_dir_all(&root).unwrap();
        // The same pixels share a layer only with the same mipmap settings: the CIT texture's
        // are the item's.
        assert_ne!(block, item);
        assert_eq!(cit, item);
    }
    #[test]
    fn prefetched_textures_take_the_layers_inserting_alone_gives() {
        let root =
            std::env::temp_dir().join(format!("horizon-sprite-prefetch-{}", std::process::id()));
        let png = |path: &str, side: u32, seed: u8| {
            let file = root.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            let mut encoder = png::Encoder::new(File::create(file).unwrap(), side, side);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let pixels: Vec<u8> = (0..side * side)
                .flat_map(|i| {
                    [
                        seed,
                        i as u8,
                        (i >> 8) as u8,
                        if i % 3 == 0 { 0 } else { 255 },
                    ]
                })
                .collect();
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        };
        for i in 0..40u8 {
            png(
                &format!("assets/minecraft/textures/block/b{i}.png"),
                16 << (i % 3),
                i,
            );
        }
        // Two textures alike share a layer; a tile outside `textures/`, a broken file and a
        // missing one take the paths they take without prefetching.
        png("assets/minecraft/textures/block/same.png", 16, 3);
        png("assets/minecraft/optifine/ctm/glass/0.png", 32, 77);
        fs::write(
            root.join("assets/minecraft/textures/block/broken.png"),
            b"no png",
        )
        .unwrap();
        let names: Vec<String> = (0..40)
            .map(|i| format!("minecraft:block/b{}", (i * 7) % 40))
            .chain(["minecraft:block/same", "minecraft:block/broken"].map(String::from))
            .chain(["minecraft:block/missing", "minecraft:block/b3"].map(String::from))
            .collect();
        let tile = "assets/minecraft/optifine/ctm/glass/0.png";
        let load = |prefetch: bool| {
            let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
            if prefetch {
                sprites.prefetch(names.iter().map(String::as_str).chain([tile]));
            }
            let mut layers: Vec<u32> = names
                .iter()
                .map(|name| sprites.insert(name).unwrap())
                .collect();
            layers.push(sprites.insert_file(tile).unwrap());
            sprites.finish();
            (
                layers,
                sprites.layers,
                sprites.pixels,
                sprites.translucent,
                sprites.size,
            )
        };
        let alone = load(false);
        assert_eq!(load(true), alone);
        // The textures alike and the broken one's missing texture: fewer layers than names.
        assert!(alone.1.values().max().unwrap() + 1 < names.len() as u32);
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn identical_still_textures_share_a_layer() {
        let root =
            std::env::temp_dir().join(format!("horizon-sprite-share-{}", std::process::id()));
        let directory = root.join("assets/a/textures/block");
        fs::create_dir_all(&directory).unwrap();
        let write = |name: &str, red: u8| {
            let mut encoder =
                png::Encoder::new(File::create(directory.join(name)).unwrap(), 16, 16);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let pixels: Vec<u8> = (0..256).flat_map(|i| [red, i as u8, 0, 255]).collect();
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        };
        write("one.png", 10);
        write("copy.png", 10);
        write("other.png", 20);
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        let one = sprites.insert("a:block/one").unwrap();
        let copy = sprites.insert("a:block/copy").unwrap();
        let other = sprites.insert("a:block/other").unwrap();
        // The same pixels as an item sprite mip differently: a layer of its own.
        fs::create_dir_all(root.join("assets/a/textures/item")).unwrap();
        fs::copy(
            directory.join("one.png"),
            root.join("assets/a/textures/item/one.png"),
        )
        .unwrap();
        let item = sprites.insert("a:item/one").unwrap();
        fs::remove_dir_all(&root).unwrap();
        assert_eq!((one, copy, other, item), (0, 0, 1, 2));
        assert_eq!((sprites.count(), sprites.layers.len()), (3, 4));
        assert_eq!((sprites.opaque.len(), sprites.mips.len()), (3, 3));
        sprites.finish();
        assert_eq!(sprites.pixels.len(), 3 * 16 * 16 * 4);
    }
    #[test]
    fn a_texture_a_connected_rule_matches_keeps_a_layer_of_its_own() {
        // A resource pack's transparent placeholders: a model's layer and a tile another
        // connected-texture rule chains onto (`matchTiles=./19.png`) have the same pixels. The
        // rule finds its faces by layer, so the one it names can't share the others'.
        let root =
            std::env::temp_dir().join(format!("horizon-sprite-apart-{}", std::process::id()));
        let directory = root.join("assets/a/textures/block");
        fs::create_dir_all(&directory).unwrap();
        let write = |name: &str| {
            let mut encoder =
                png::Encoder::new(File::create(directory.join(name)).unwrap(), 16, 16);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[0; 16 * 16 * 4])
                .unwrap();
        };
        for name in ["placeholder.png", "named.png", "again.png"] {
            write(name);
        }
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        sprites.keep_apart(["a:block/named".to_owned()]);
        let placeholder = sprites.try_insert("a:block/placeholder").unwrap();
        let named = sprites.try_insert("a:block/named").unwrap();
        let again = sprites.try_insert("a:block/again").unwrap();
        fs::remove_dir_all(&root).unwrap();
        // The named one alone; the others still share.
        assert_ne!(named, placeholder);
        assert_eq!(again, placeholder);
        assert_eq!(sprites.count(), 2);
    }
    #[test]
    fn a_32px_only_atlas_does_not_add_an_extra_mipmap() {
        let mut sprites = Sprites {
            native: vec![
                (32, Arc::new(vec![vec![255; 32 * 32 * 4]]), 0),
                (32, Arc::new(vec![vec![128; 32 * 32 * 4]]), 0),
            ],
            ..Default::default()
        };
        sprites.finish();
        assert_eq!((sprites.size, sprites.minimum_size), (32, 32));
        for level in 0..=4 {
            assert_eq!(
                sprite_mipmap_limit(level, sprites.size, sprites.minimum_size, 6),
                level
            );
        }
        sprites
            .native
            .push((16, Arc::new(vec![vec![255; 16 * 16 * 4]]), 0));
        sprites.finish();
        assert_eq!((sprites.size, sprites.minimum_size), (32, 16));
        assert_eq!(
            sprite_mipmap_limit(0, sprites.size, sprites.minimum_size, 6),
            1
        );
    }

    #[test]
    fn hud_animation_resampling_uses_the_actual_frame_size() {
        for size in [16, 32, 64] {
            let animation = test_animation(size, 0);
            let resized = animation.resized_pixels(1, 16, 16);
            let last = (15 * size / 16) as u8;
            assert_eq!(&resized[15 * 4..16 * 4], &[last, 0, 99, 255]);
            assert_eq!(&resized[255 * 4..256 * 4], &[last, last, 99, 255]);
        }
    }

    #[test]
    fn native_fire_sprites_register_full_animation_sequences() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root
            .join("assets/minecraft/textures/block/fire_0.png")
            .exists()
        {
            return;
        }
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        for name in ["fire_0", "fire_1", "soul_fire_0", "soul_fire_1"] {
            let layer = sprites.insert(&format!("minecraft:block/{name}")).unwrap();
            let animation = sprites
                .animations
                .iter()
                .find(|a| a.layer == layer)
                .expect("fire must be animated");
            assert_ne!(
                animation.pixels(animation.frame(0)),
                animation.pixels(animation.frame(1))
            );
            assert_ne!(
                animation.resized_pixels(animation.frame(0), 16, 16),
                animation.resized_pixels(animation.frame(1), 16, 16)
            );
            assert_eq!(
                animation.pixels(animation.frame(0)).len(),
                (SPRITE_SIZE * SPRITE_SIZE * 4) as usize
            );
        }
    }

    #[test]
    fn native_32_pixel_animation_frames_keep_every_texel() {
        let root = std::env::temp_dir().join(format!(
            "horizon-sprite-detail-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let directory = root.join("assets/test/textures");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("flow.png");
        let mut pixels = Vec::new();
        for y in 0..64u8 {
            for x in 0..32u8 {
                pixels.extend([x, y, 0, 255]);
            }
        }
        let mut encoder = png::Encoder::new(File::create(&path).unwrap(), 32, 64);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        fs::write(path.with_extension("png.mcmeta"), r#"{"animation":{}}"#).unwrap();
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        sprites.insert("test:flow").unwrap();
        sprites.finish();
        let size = sprites.size;
        let initial = sprites.pixels.clone();
        let animation = sprites.animations[0].clone();
        fs::remove_dir_all(root).unwrap();
        assert_eq!(
            size, 32,
            "half a flowing-water frame must retain 16 texels, not 8"
        );
        assert_eq!(initial, pixels[..32 * 32 * 4]);
        assert_eq!(animation.pixels(animation.frame(1)), &pixels[32 * 32 * 4..]);
    }

    #[test]
    fn animation_boundaries_and_repeated_frames_do_not_depend_on_render_rate() {
        let animation = SpriteAnimation {
            layer: 0,
            materials: [None, None],
            shape: None,
            frames: AnimationFrames::Raw(Arc::new(vec![vec![], vec![], vec![]])),
            clip: Arc::new(AnimationMetadata::from_json(r#"{"frames":[{"index":2,"time":8},{"index":1,"time":3},{"index":1,"time":2},0]}"#).unwrap().clip(3).unwrap().unwrap()),
        };
        for (tick, frame) in [
            (0, 2),
            (7, 2),
            (8, 1),
            (10, 1),
            (11, 1),
            (12, 1),
            (13, 0),
            (14, 2),
            (29, 2),
            (u64::MAX, 2),
        ] {
            assert_eq!(animation.frame(tick), frame);
        }
    }

    #[test]
    fn interpolated_animations_blend_towards_the_next_frame() {
        let animation = test_clip(16, 0, r#"{"frametime":4,"interpolate":true}"#);
        let blend = |frame, next, millis| FrameBlend {
            frame,
            next,
            millis,
        };
        for (tick, expected) in [
            (0, blend(0, 0, 0)),
            (1, blend(0, 1, 250)),
            (3, blend(0, 1, 750)),
            (4, blend(1, 1, 0)),
            (5, blend(1, 0, 250)),
            (8, blend(0, 0, 0)),
        ] {
            assert_eq!(animation.blend(tick), expected, "tick {tick}");
        }
        let stepped = test_clip(16, 0, r#"{"frametime":4}"#);
        assert_eq!(stepped.blend(1), blend(0, 0, 0));
        assert_eq!(stepped.blend(5), blend(1, 1, 0));
    }

    #[test]
    fn animation_pack_compresses_frames_and_round_trips_pixels() {
        let frames: Vec<Vec<u8>> = (0..16u8)
            .map(|f| {
                let mut buf = Vec::with_capacity(64 * 64 * 4);
                for i in 0..64 * 64 {
                    buf.extend_from_slice(&[(i & 3) as u8, ((i >> 6) & 3) as u8, f, 255]);
                }
                buf
            })
            .collect();
        let mut animation = test_frames(3, frames.clone(), r#"{"frametime":1}"#);
        let raw_bytes = animation.frame_ram_bytes();
        assert!(!animation.is_packed());
        assert_eq!(raw_bytes, 16 * 64 * 64 * 4);

        animation.pack();
        assert!(animation.is_packed());
        let packed_bytes = animation.frame_ram_bytes();
        assert!(
            packed_bytes * 4 < raw_bytes,
            "expected LZ4 pack to shrink repeated animation frames: packed={packed_bytes} raw={raw_bytes}"
        );
        // Packing again is idempotent.
        animation.pack();
        assert_eq!(animation.frame_count(), frames.len());
        for (i, expected) in frames.iter().enumerate() {
            assert_eq!(animation.pixels(i), expected.as_slice());
        }
        assert_eq!(animation.resized_pixels(2, 64, 64), frames[2]);
        assert_eq!(
            animation.resized_pixels(2, 32, 32),
            resample(&frames[2], 64, 32)
        );
        assert_eq!(animation.frame_at(2, 32), resample(&frames[2], 64, 32));
    }

    #[test]
    fn fast_resample_paths_match_nearest_formula() {
        let reference = |frame: &[u8], from: u32, to: u32| -> Vec<u8> {
            let mut out = Vec::with_capacity(to as usize * to as usize * 4);
            for row in 0..to {
                let source_row = row * from / to;
                for column in 0..to {
                    let offset = ((source_row * from + column * from / to) * 4) as usize;
                    out.extend_from_slice(&frame[offset..offset + 4]);
                }
            }
            out
        };
        for (from, to) in [
            (8u32, 32u32),
            (16, 64),
            (32, 16),
            (64, 16),
            (24, 32),
            (16, 16),
        ] {
            let src: Vec<u8> = (0..from * from)
                .flat_map(|i| [i as u8, (i >> 2) as u8, (i >> 4) as u8, 255])
                .collect();
            assert_eq!(
                resample(&src, from, to),
                reference(&src, from, to),
                "{from}->{to}"
            );
        }
    }

    #[test]
    fn repeated_finish_keeps_labpbr_maps_indexed_by_layer() {
        let normal_0 = vec![[10, 20, 30, 255].repeat(16 * 16)];
        let spec_0 = vec![
            [1, 2, 3, 255].repeat(16 * 16),
            [4, 5, 6, 255].repeat(16 * 16),
        ];
        let normal_1 = vec![[70, 80, 90, 255].repeat(16 * 16)];
        let mut sprites = Sprites {
            native: vec![(16, Arc::new(vec![vec![0; 16 * 16 * 4]; 2]), 1)],
            pbr: vec![[
                Some((16, Arc::new(normal_0.clone()))),
                Some((16, Arc::new(spec_0.clone()))),
            ]],
            animations: vec![test_frames(
                0,
                vec![vec![0; 16 * 16 * 4]; 2],
                r#"{"frametime":1}"#,
            )],
            ..Default::default()
        };
        sprites.finish();
        assert_eq!(sprites.normals[..4], [10, 20, 30, 255]);
        assert_eq!(sprites.specular[..4], [4, 5, 6, 255]);
        assert_eq!(
            sprites.animations[0].material_frames(1),
            Some(spec_0.as_slice())
        );
        // Adding a second layer after the first layout keeps layer 0's maps and aligns layer 1's.
        sprites
            .native
            .push((16, Arc::new(vec![vec![0; 16 * 16 * 4]]), 0));
        sprites
            .pbr
            .push([Some((16, Arc::new(normal_1.clone()))), None]);
        sprites.finish();
        let layer_bytes = 16 * 16 * 4;
        assert_eq!(sprites.normals.len(), 2 * layer_bytes);
        assert_eq!(sprites.specular.len(), 2 * layer_bytes);
        assert_eq!(sprites.normals[..4], [10, 20, 30, 255]);
        assert_eq!(sprites.normals[layer_bytes..][..4], [70, 80, 90, 255]);
        assert_eq!(sprites.specular[..4], [4, 5, 6, 255]);
        assert_eq!(sprites.specular[layer_bytes..][..4], NO_SPECULAR);
        assert_eq!(
            sprites.animations[0].material_frames(0),
            Some(normal_0.as_slice())
        );
        assert_eq!(
            sprites.animations[0].material_frames(1),
            Some(spec_0.as_slice())
        );
    }
}

#[cfg(test)]
mod png_size_tests {
    use crate::hud::{claimed_size_png, grey_png};

    /// A block texture whose header claims more pixels than a full atlas is refused from its
    /// header, as an undecodable one is, before memory for its pixels is reserved.
    #[test]
    fn a_sprite_claiming_a_huge_size_is_refused() {
        let error = super::decode_rgba(&claimed_size_png(16_385, 16_384)).unwrap_err();
        assert!(
            error.to_string().contains("16385x16384 PNG is larger"),
            "{error}"
        );
    }

    #[test]
    fn a_sprite_at_the_side_limit_still_decodes() {
        let side = crate::atlas_sources::MAX_SPRITE_SIDE as u32;
        let (width, height, rgba) = super::decode_rgba(&grey_png(side, 1)).unwrap();
        assert_eq!((width, height, rgba.len()), (side, 1, side as usize * 4));
    }

    fn decode_sprite(
        png: &[u8],
        mcmeta: Option<&str>,
    ) -> crate::Result<crate::sprite_cache::Decoded> {
        let path = "assets/minecraft/textures/block/test.png";
        let files = mcmeta.map(|text| {
            (
                format!("{path}.mcmeta"),
                horizon_assets::resource_source::Resource::bytes(text.as_bytes().to_vec()),
            )
        });
        let source = super::ResourceSource::indexed(files.into_iter().collect());
        super::Sprites::decode_file_data(&source, path, "minecraft:block/test", Some(png))
    }

    /// A real 32768x1 (or 1x32768) PNG is a few hundred bytes and decodes to 128 KiB; its
    /// square of 32768 (4 GiB) is never made: the frame keeps its shape until the layers' size
    /// is known.
    #[test]
    fn a_sprite_far_from_square_keeps_its_shape() {
        let side = crate::atlas_sources::MAX_SPRITE_SIDE as u32;
        for (width, height) in [(side, 1), (1, side)] {
            let decoded = decode_sprite(&grey_png(width, height), None).unwrap();
            assert_eq!((decoded.side, decoded.frame_size), (side, [width, height]));
            assert_eq!(decoded.frames.len(), 1);
            assert_eq!(decoded.frames[0].len(), side as usize * 4);
        }
    }

    /// An RGBA image whose texel at (x, y) is unlike its neighbours'.
    fn pattern(width: u32, height: u32) -> Vec<u8> {
        (0..width * height)
            .flat_map(|i| {
                let (x, y) = (i % width, i / width);
                [x as u8, y as u8, (x * 7 + y * 13) as u8, 255]
            })
            .collect()
    }
    fn png_of(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
        let mut encoded = Vec::new();
        let mut encoder = png::Encoder::new(&mut encoded, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(pixels).unwrap();
        writer.finish().unwrap();
        encoded
    }
    /// What Horizon did before frames kept their shape: the region squared at its longer side,
    /// then that square resampled to `size`.
    fn squared_then_resampled(
        pixels: &[u8],
        stride: u32,
        [x, y, w, h]: [u32; 4],
        size: u32,
    ) -> Vec<u8> {
        let side = w.max(h);
        let mut square = Vec::new();
        for row in 0..side {
            let source_row = y + row * h / side;
            for column in 0..side {
                let offset = ((source_row * stride + x + column * w / side) * 4) as usize;
                square.extend_from_slice(&pixels[offset..offset + 4]);
            }
        }
        super::resample(&square, side, size)
    }

    /// The one-pass layer picks the same texels as squaring then resampling, for square and
    /// non-square regions, scaled up, down and kept.
    #[test]
    fn a_region_layer_matches_squaring_then_resampling() {
        for (w, h, size) in [
            (16, 16, 16),
            (16, 16, 32),
            (16, 16, 8),
            (24, 10, 32),
            (24, 10, 24),
            (24, 10, 16),
            (10, 24, 32),
            (7, 3, 16),
            (1, 32, 16),
            (32, 1, 64),
            (64, 2, 1024),
        ] {
            let stride = w + 5;
            let pixels = pattern(stride, h + 4);
            assert_eq!(
                super::region_layer(&pixels, stride, [3, 2, w, h], size),
                squared_then_resampled(&pixels, stride, [3, 2, w, h], size),
                "{w}x{h} to {size}"
            );
        }
    }

    /// Layers and animation frames after `finish` are byte for byte what they were when frames
    /// were squared at decoding: a square-frame strip, an interpolated sheet of 24x10 frames,
    /// and a still 24x10 texture.
    #[test]
    fn finished_layers_are_unchanged() {
        for (width, height, frame, mcmeta) in [
            (16, 48, [16, 16], Some(r#"{"animation":{}}"#)),
            (
                24,
                40,
                [24, 10],
                Some(r#"{"animation":{"width":24,"height":10,"interpolate":true}}"#),
            ),
            (24, 10, [24, 10], None),
        ] {
            let pixels = pattern(width, height);
            let path = "assets/minecraft/textures/block/test.png";
            let mut files = vec![(
                path.to_owned(),
                horizon_assets::resource_source::Resource::bytes(png_of(width, height, &pixels)),
            )];
            if let Some(text) = mcmeta {
                files.push((
                    format!("{path}.mcmeta"),
                    horizon_assets::resource_source::Resource::bytes(text.as_bytes().to_vec()),
                ));
            }
            let source = super::ResourceSource::indexed(files.into_iter().collect());
            let mut sprites = super::Sprites::new(source);
            sprites.insert("minecraft:block/test").unwrap();
            sprites.finish();
            let [fw, fh] = frame;
            let expected: Vec<Vec<u8>> = (0..height / fh)
                .map(|i| squared_then_resampled(&pixels, width, [0, i * fh, fw, fh], sprites.size))
                .collect();
            assert_eq!(sprites.pixels, expected[0], "{width}x{height}");
            match mcmeta {
                Some(_) => {
                    let animation = &sprites.animations[0];
                    let frames: Vec<Vec<u8>> = (0..animation.frame_count())
                        .map(|f| animation.frame_at(f, sprites.size))
                        .collect();
                    assert_eq!(frames, expected);
                }
                None => assert!(sprites.animations.is_empty()),
            }
        }
    }

    /// 1024x8224 with 1024x32 frames (257 of them) decodes to 32 MiB and loads; charged as 257
    /// squares of 1024 it passed a full atlas and was refused. A small sheet of the same shape
    /// loads with its frames as cut, and at the real size only the layers' size decides: its
    /// frames animate at layers of 512 and below and stay still at 1024 (257 x 1024^2 texels
    /// is 0.4% over a full atlas).
    #[test]
    fn a_sheet_of_wide_frames_loads() {
        let mcmeta = r#"{"animation":{"width":64,"height":2}}"#;
        let decoded = decode_sprite(&grey_png(64, 2 * 257), Some(mcmeta)).unwrap();
        assert_eq!((decoded.side, decoded.frame_size), (64, [64, 2]));
        assert_eq!(decoded.frames.len(), 257);
        assert!(decoded.frames.iter().all(|f| f.len() == 64 * 2 * 4));
        assert!(super::check_frames(512, 257).is_ok());
        assert!(super::check_frames(1024, 257).is_err());
    }

    /// An animated HUD sprite is read before `finish` squares its frames: it reads each
    /// frame as its square would, and as it does once squared.
    #[test]
    fn a_frame_not_yet_square_resizes_as_its_square() {
        let frame = pattern(4, 2);
        let square = super::region_layer(&frame, 4, [0, 0, 4, 2], 4);
        let mut wide = super::test_frames(0, vec![frame.clone(), frame], r#"{"frametime":1}"#);
        let squared = super::test_frames(0, vec![square.clone(), square], r#"{"frametime":1}"#);
        wide.shape = Some([4, 2]);
        for (width, height) in [(4, 4), (8, 8), (3, 5), (16, 2)] {
            assert_eq!(
                wide.resized_pixels(1, width, height),
                squared.resized_pixels(1, width, height),
                "{width}x{height}"
            );
        }
    }

    /// Opacity under a quad reads the frame's own texels: a 4x1 frame whose last texel is
    /// see-through is opaque under its left three quarters only.
    #[test]
    fn opacity_of_a_frame_not_yet_square() {
        let mut frame = [9, 9, 9, 255].repeat(4);
        frame[15] = 0;
        let sprites = super::Sprites {
            native: vec![(4, std::sync::Arc::new(vec![frame]), 0)],
            shapes: std::collections::HashMap::from([(0, [4, 1])]),
            opaque: vec![false],
            ..Default::default()
        };
        let quad = |u: f32| [[0.0, 0.0], [u, 0.0], [u, 1.0], [0.0, 1.0]];
        assert!(sprites.region_opaque(0, &quad(0.75)));
        assert!(!sprites.region_opaque(0, &quad(1.0)));
    }

    /// `finish` makes only the frame a layer shows first at the layers' size: 65 frames of
    /// 2048x1 (520 KiB as decoded) would be 65 x 2048^2 texels (1.06 GiB) squared. The animation
    /// keeps its decoded frames; the animator charges what it would build (`gfx::animate`).
    #[test]
    fn finish_makes_only_the_first_frame_at_the_layers_size() {
        let frames: Vec<Vec<u8>> = (0..65u8).map(|i| [i, 0, 0, 255].repeat(2048)).collect();
        let native = std::sync::Arc::new(frames.clone());
        let mut sprites = super::Sprites {
            native: vec![(2048, native.clone(), 0)],
            shapes: std::collections::HashMap::from([(0, [2048, 1])]),
            pbr: vec![[None, None]],
            animations: vec![super::test_frames(0, frames, r#"{"frametime":1}"#)],
            ..Default::default()
        };
        sprites.finish();
        assert_eq!(sprites.size, 2048);
        assert!(std::sync::Arc::ptr_eq(
            sprites.animations[0].frames.raw_arc().unwrap(),
            &native
        ));
        assert_eq!(sprites.pixels.len(), 2048 * 2048 * 4);
        assert_eq!(sprites.pixels[..4], [0, 0, 0, 255]);
    }

    /// A 1024x4112 sheet of 16x16 frames (16448 of them) beside 128 px textures: squared at the
    /// layers' size its frames would be 1,028 MiB, while the animator builds them at their own
    /// 16 px edge (about 21.4 MiB of chains, `gfx::animate`) and plays them. A small sheet of
    /// 16x16 frames beside a 128 px texture keeps its frames as decoded, and shows each as the
    /// frames squared at 128 did.
    #[test]
    fn small_frames_beside_large_textures_keep_their_size() {
        assert!(super::check_frames(16, 16_448).is_ok());
        let sheet = pattern(32, 32);
        let path = "assets/minecraft/textures/block/test.png";
        let large = "assets/minecraft/textures/block/large.png";
        let bytes = |b: Vec<u8>| horizon_assets::resource_source::Resource::bytes(b);
        let mcmeta = r#"{"animation":{"width":16,"height":16}}"#;
        let source = super::ResourceSource::indexed(
            [
                (path.to_owned(), bytes(png_of(32, 32, &sheet))),
                (format!("{path}.mcmeta"), bytes(mcmeta.as_bytes().to_vec())),
                (large.to_owned(), bytes(grey_png(128, 128))),
            ]
            .into_iter()
            .collect(),
        );
        let mut sprites = super::Sprites::new(source);
        sprites.insert("minecraft:block/test").unwrap();
        sprites.insert("minecraft:block/large").unwrap();
        sprites.finish();
        assert_eq!(sprites.size, 128);
        let animation = &sprites.animations[0];
        assert_eq!(animation.frame_count(), 4);
        for f in 0..4u32 {
            assert_eq!(animation.pixels(f as usize).len(), 16 * 16 * 4);
            let region = [f % 2 * 16, f / 2 * 16, 16, 16];
            assert_eq!(
                animation.frame_at(f as usize, 128),
                squared_then_resampled(&sheet, 32, region, 128)
            );
        }
    }

    /// The LabPBR sizing of a tall, narrow companion map: its side is clamped before the
    /// budget is charged, which overflowed `u64` for 1024 layers of a 2^28 side.
    #[test]
    fn the_layer_size_of_a_huge_side_stays_in_range() {
        let maps = super::map_bytes(super::DEFAULT_TEXTURE_BUDGET);
        assert_eq!(
            super::side_within(1 << 28, super::LAYERS_AT_LEAST, maps),
            128
        );
        assert_eq!(super::side_within(u32::MAX, 1, u64::MAX), 16_384);
        // Every input at its maximum: no overflow, the smallest side.
        assert_eq!(
            super::side_within(u32::MAX, u64::MAX, u64::MAX),
            super::SPRITE_SIZE
        );
        assert_eq!(super::side_within(16_385, 1, u64::MAX), 16_384);
    }

    /// Still textures share a layer when their squares are equal: a patterned 2x4 texture and
    /// a 4x4 copy with each column doubled, as before frames kept their shape.
    #[test]
    fn stills_with_equal_squares_hash_alike() {
        use std::hash::Hasher;
        let hash = |frame: &[u8], shape: [u32; 2]| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            super::hash_square(frame, shape, &mut hasher);
            hasher.finish()
        };
        let narrow = pattern(2, 4);
        let doubled: Vec<u8> = narrow.chunks(4).flat_map(|p| [p, p].concat()).collect();
        assert_eq!(hash(&narrow, [2, 4]), hash(&doubled, [4, 4]));
        assert_ne!(hash(&narrow, [2, 4]), hash(&pattern(4, 4), [4, 4]));
    }

    #[test]
    fn a_tall_animation_strip_still_loads() {
        let decoded = decode_sprite(&grey_png(16, 64), Some(r#"{"animation":{}}"#)).unwrap();
        assert_eq!((decoded.side, decoded.frames.len()), (16, 4));
        assert_eq!(decoded.frames[3], [128, 128, 128, 255].repeat(256));
    }

    /// A strip of 16x16 frames a little over 32768 pixels tall (about 2 MiB) loads in the Java
    /// game: its sheet is longer than any texture side, its frames are small.
    #[test]
    fn a_strip_longer_than_any_texture_side_loads() {
        let height = crate::atlas_sources::MAX_SPRITE_SIDE as u32 + 16;
        let decoded = decode_sprite(&grey_png(16, height), Some(r#"{"animation":{}}"#)).unwrap();
        assert_eq!(
            (decoded.side, decoded.frames.len()),
            (16, height as usize / 16)
        );
    }

    /// The largest frame the check lets through is 1 GiB, too heavy to decode in CI: its edges
    /// are tested on the check alone.
    #[test]
    fn frames_are_bounded_by_the_atlas() {
        let atlas = crate::hud::MAX_ATLAS_SIZE;
        assert_eq!(super::check_frames(atlas, 1), Ok(()));
        assert!(super::check_frames(atlas + 1, 1).is_err());
        assert!(super::check_frames(atlas, 2).is_err());
        assert_eq!(super::check_frames(16, atlas * atlas / 256), Ok(()));
        assert!(super::check_frames(16, atlas * atlas / 256 + 1).is_err());
        assert!(super::check_frames(u32::MAX, u32::MAX).is_err());
    }

    #[test]
    fn read_png_dimensions_refuses_huge_claimed_size() {
        let path = "assets/minecraft/textures/block/huge.png";
        let source = super::ResourceSource::indexed(
            [(
                path.to_owned(),
                horizon_assets::resource_source::Resource::bytes(claimed_size_png(16_385, 16_384)),
            )]
            .into_iter()
            .collect(),
        );
        let error = super::read_png_dimensions(&source, path).unwrap_err();
        assert!(
            error.to_string().contains("16385x16384 PNG is larger"),
            "{error}"
        );
    }
}
