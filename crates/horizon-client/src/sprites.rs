//! Shared nearest-sampled sprite array for the controlled scene.
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
    pub size: u32,
    pub pixels: Vec<u8>,
    pub layers: BTreeMap<String, u32>,
    pub animations: Vec<SpriteAnimation>,
}
impl Sprites {
    pub fn insert(&mut self, root: &Path, name: &str) -> Result<u32> {
        if let Some(layer) = self.layers.get(name) {
            return Ok(*layer);
        }
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
            if animation.interpolate {
                return Err(
                    "interpolated animation is not yet supported by the inspection renderer".into(),
                );
            }
            let [width, height] =
                animation.frame_size(i32::try_from(info.width)?, i32::try_from(info.height)?);
            (u32::try_from(width)?, u32::try_from(height)?)
        } else {
            (info.width, info.height)
        };
        if width != height
            || width == 0
            || info.width % width != 0
            || info.height % height != 0
            || (self.size != 0 && self.size != width)
        {
            return Err(format!(
                "inspection sprite {name} must be square and match the sprite array dimensions"
            )
            .into());
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
            if rgba[3] != 0 && rgba[3] != 255 {
                return Err(format!(
                    "inspection sprite {name} requires fractional-alpha rendering"
                )
                .into());
            }
            pixels.extend(rgba);
        }
        let layer = u32::try_from(self.layers.len())?;
        let columns = info.width / width;
        let count = columns * (info.height / height);
        let mut frames = Vec::new();
        for index in 0..count {
            let x = index % columns * width;
            let y = index / columns * height;
            let mut frame = Vec::with_capacity((width * height * 4) as usize);
            for row in y..y + height {
                let offset = ((row * info.width + x) * 4) as usize;
                frame.extend_from_slice(&pixels[offset..offset + (width * 4) as usize]);
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
        self.layers.insert(name.to_owned(), layer);
        self.size = width;
        self.pixels.extend_from_slice(&frames[initial]);
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
