//! Shared nearest-sampled sprite array for the controlled scene.
use crate::Result;
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
    sequence: Arc<Vec<(usize, u32)>>,
    duration: u64,
}
impl SpriteAnimation {
    pub fn frame(&self, tick: u64) -> usize {
        let mut time = tick % self.duration;
        for &(index, duration) in self.sequence.iter() {
            if time < u64::from(duration) {
                return index;
            }
            time -= u64::from(duration);
        }
        unreachable!("validated animation duration")
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
            Ok(text) => Some(serde_json::from_str::<serde_json::Value>(&text)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let animation = metadata.as_ref().and_then(|v| v.get("animation"));
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
        let positive = |value: &serde_json::Value| -> Result<u32> {
            let value = value
                .as_u64()
                .filter(|v| *v > 0 && *v <= i32::MAX as u64)
                .ok_or("invalid positive animation integer")?;
            Ok(value as u32)
        };
        let (width, height) = if let Some(animation) = animation {
            if !animation.is_object() {
                return Err("invalid animation metadata".into());
            }
            if animation
                .get("interpolate")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                return Err(
                    "interpolated animation is not yet supported by the inspection renderer".into(),
                );
            }
            match (
                animation.get("width").map(positive).transpose()?,
                animation.get("height").map(positive).transpose()?,
            ) {
                (Some(w), Some(h)) => (w, h),
                (Some(w), None) => (w, info.height),
                (None, Some(h)) => (info.width, h),
                (None, None) => {
                    let size = info.width.min(info.height);
                    (size, size)
                }
            }
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
        if let Some(metadata) = animation {
            let default_time = metadata
                .get("frametime")
                .map(positive)
                .transpose()?
                .unwrap_or(1);
            let sequence = if let Some(values) = metadata.get("frames") {
                let mut sequence = Vec::new();
                for value in values.as_array().ok_or("invalid animation frame list")? {
                    let (index, time) = if value.is_object() {
                        (
                            value.get("index").and_then(serde_json::Value::as_u64),
                            value
                                .get("time")
                                .map(positive)
                                .transpose()?
                                .unwrap_or(default_time),
                        )
                    } else {
                        (value.as_u64(), default_time)
                    };
                    if let Some(index) = index.filter(|i| *i < u64::from(count)) {
                        sequence.push((index as usize, time));
                    }
                }
                sequence
            } else {
                (0..count).map(|i| (i as usize, default_time)).collect()
            };
            if sequence.len() > 1 {
                initial = sequence[0].0;
                let duration = sequence.iter().map(|(_, time)| u64::from(*time)).sum();
                self.animations.push(SpriteAnimation {
                    layer,
                    frames: frames.clone(),
                    sequence: Arc::new(sequence),
                    duration,
                });
            }
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
            sequence: Arc::new(vec![(2, 8), (1, 3), (1, 2), (0, 1)]),
            duration: 14,
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
