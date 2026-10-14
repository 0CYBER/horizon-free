//! Shared nearest-sampled sprite array for block and item textures.
use crate::Result;
use horizon_assets::animation::{AnimationClip, AnimationMetadata};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::BufReader,
    path::Path,
    sync::Arc,
};

#[derive(Clone)]
pub struct SpriteAnimation {
    pub layer: u32,
    frames: Arc<Vec<Vec<u8>>>,
    clip: Arc<AnimationClip>,
}
impl SpriteAnimation {
    pub fn frame(&self, tick: u64) -> usize {
        self.clip.sample(tick).frame as usize
    }
    pub fn pixels(&self, frame: usize) -> &[u8] {
        &self.frames[frame]
    }
}

#[derive(Default)]
pub struct Sprites {
    /// Edge length of every array layer and the layers' current frames, set
    /// by [`Sprites::finish`].
    pub size: u32,
    pub pixels: Vec<u8>,
    pub layers: BTreeMap<String, u32>,
    pub animations: Vec<SpriteAnimation>,
    pub opaque: Vec<bool>,
    pub translucent: Vec<bool>,
    /// Each layer's frames at the texture's own resolution (square), and the
    /// frame shown first.
    native: Vec<(u32, Arc<Vec<Vec<u8>>>, usize)>,
}
/// Edge length of vanilla's block textures: the smallest array layer.
pub const SPRITE_SIZE: u32 = 16;
/// Largest sprite array (all layers of the top mip level): resource packs
/// with bigger textures are sampled down to fit.
const MAX_ARRAY_BYTES: u64 = 256 << 20;
/// Nearest-resample a square RGBA frame to another square size.
fn resample(frame: &[u8], from: u32, to: u32) -> Vec<u8> {
    if from == to {
        return frame.to_vec();
    }
    let mut out = Vec::with_capacity((to * to * 4) as usize);
    for row in 0..to {
        let source_row = row * from / to;
        for column in 0..to {
            let offset = ((source_row * from + column * from / to) * 4) as usize;
            out.extend_from_slice(&frame[offset..offset + 4]);
        }
    }
    out
}
impl Sprites {
    /// Lay every layer out at one size: the largest texture's (resource packs
    /// may ship 32x, 64x... textures, like vanilla's stitched atlas), rounded
    /// up to a power of two and limited to [`MAX_ARRAY_BYTES`]. Smaller
    /// textures are scaled up by whole pixels, so UVs stay per layer.
    pub fn finish(&mut self) {
        let layers = self.native.len().max(1) as u64;
        let largest = self.native.iter().map(|n| n.0).max().unwrap_or(SPRITE_SIZE);
        let mut size = largest.max(SPRITE_SIZE).next_power_of_two();
        while size > SPRITE_SIZE && layers * u64::from(size * size) * 4 > MAX_ARRAY_BYTES {
            size /= 2;
        }
        let mut resized = Vec::with_capacity(self.native.len());
        self.pixels.clear();
        for (from, frames, initial) in &self.native {
            let frames: Arc<Vec<Vec<u8>>> =
                Arc::new(frames.iter().map(|f| resample(f, *from, size)).collect());
            self.pixels.extend_from_slice(&frames[*initial]);
            resized.push(frames);
        }
        for animation in &mut self.animations {
            animation.frames = resized[animation.layer as usize].clone();
        }
        self.size = size;
    }
    /// The layer of a texture, added on first use. A texture that is missing
    /// or does not load (a resource pack's filter or a broken file) becomes
    /// vanilla's missing texture, as in a stitched atlas.
    pub fn insert(&mut self, root: &Path, name: &str) -> Result<u32> {
        if let Some(layer) = self.layers.get(name) {
            return Ok(*layer);
        }
        match self.load(root, name) {
            Ok(layer) => Ok(layer),
            Err(e) => {
                log::warn!("Horizon: using the missing texture for {name}: {e}");
                // MissingTextureAtlasSprite.generateMissingImage: 16x16,
                // magenta and black quarters.
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
                let layer = u32::try_from(self.layers.len())?;
                self.opaque.push(true);
                self.translucent.push(false);
                self.layers.insert(name.to_owned(), layer);
                self.native.push((SPRITE_SIZE, Arc::new(vec![frame]), 0));
                Ok(layer)
            }
        }
    }
    fn load(&mut self, root: &Path, name: &str) -> Result<u32> {
        let (namespace, path) = name.split_once(':').ok_or("invalid sprite identifier")?;
        let path = root.join(format!("assets/{namespace}/textures/{path}.png"));
        let metadata = match fs::read_to_string(path.with_extension("png.mcmeta")) {
            Ok(text) => Some(serde_json::from_str::<
                BTreeMap<String, Box<serde_json::value::RawValue>>,
            >(&text)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let animation = metadata
            .as_ref()
            .and_then(|v| v.get("animation"))
            .map(|v| AnimationMetadata::from_json(v.get()))
            .transpose()?;
        let mut decoder = png::Decoder::new(BufReader::new(File::open(path)?));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info()?;
        let mut bytes = vec![
            0;
            reader
                .output_buffer_size()
                .ok_or("sprite exceeds decoder limits")?
        ];
        let info = reader.next_frame(&mut bytes)?;
        let (width, height) = if let Some(animation) = &animation {
            let [width, height] =
                animation.frame_size(i32::try_from(info.width)?, i32::try_from(info.height)?);
            (u32::try_from(width)?, u32::try_from(height)?)
        } else {
            (info.width, info.height)
        };
        if width == 0 || height == 0 || info.width % width != 0 || info.height % height != 0 {
            return Err(format!("sprite {name} has invalid frame dimensions").into());
        }
        let channels = info.color_type.samples();
        let mut pixels = Vec::with_capacity(info.width as usize * info.height as usize * 4);
        for pixel in bytes[..info.buffer_size()].chunks_exact(channels) {
            let rgba = match info.color_type {
                png::ColorType::Rgba => [pixel[0], pixel[1], pixel[2], pixel[3]],
                png::ColorType::Rgb => [pixel[0], pixel[1], pixel[2], 255],
                png::ColorType::Grayscale => [pixel[0], pixel[0], pixel[0], 255],
                png::ColorType::GrayscaleAlpha => [pixel[0], pixel[0], pixel[0], pixel[1]],
                _ => return Err("unsupported expanded sprite format".into()),
            };
            pixels.extend(rgba);
        }
        let layer = u32::try_from(self.layers.len())?;
        let columns = info.width / width;
        let count = columns * (info.height / height);
        // Layers are square; a frame of another shape is nearest-resampled to
        // its longer side so UVs (normalized per layer) are unchanged.
        let side = width.max(height);
        let mut frames = Vec::new();
        for index in 0..count {
            let x = index % columns * width;
            let y = index / columns * height;
            let mut frame = Vec::with_capacity((side * side * 4) as usize);
            for row in 0..side {
                let source_row = y + row * height / side;
                for column in 0..side {
                    let offset =
                        ((source_row * info.width + x + column * width / side) * 4) as usize;
                    frame.extend_from_slice(&pixels[offset..offset + 4]);
                }
            }
            frames.push(frame);
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
                frames: frames.clone(),
                clip: Arc::new(clip),
            });
        }
        self.opaque
            .push(pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        self.translucent.push(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[3] != 0 && p[3] != 255),
        );
        self.layers.insert(name.to_owned(), layer);
        self.native.push((side, frames, initial));
        Ok(layer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn animation_boundaries_and_repeated_frames_do_not_depend_on_render_rate() {
        let animation = SpriteAnimation {
            layer: 0,
            frames: Arc::new(vec![vec![], vec![], vec![]]),
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
}
