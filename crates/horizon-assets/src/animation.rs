//! Java Edition 26.3 animation metadata and sprite tick selection.
use crate::json::*;
use serde_json::value::RawValue;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimationFrame {
    pub index: i32,
    pub time: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimationMetadata {
    pub frames: Option<Vec<AnimationFrame>>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub frame_time: i32,
    pub interpolate: bool,
}

fn optional<'a>(object: &Object<'a>, name: &str) -> Option<&'a RawValue> {
    object.get(name).copied().filter(|v| v.get() != "null")
}
fn bounded_integer(value: &RawValue, minimum: i32) -> Result<i32> {
    // A lenient JsonParser exposes scanner failures as strings, which the
    // uncompressed numeric codec rejects. Model streams instead reject them
    // globally, even inside ignored fields.
    if !native_number_token(value.get()) {
        return Err(error(
            "native reader classifies animation number as a string",
        ));
    }
    let value = integer(value)?;
    if value < minimum {
        return Err(error("animation integer outside codec range"));
    }
    Ok(value)
}
impl AnimationMetadata {
    /// Decode the animation section itself, using the uncompressed JsonOps codec
    /// rules. Numeric primitives truncate and wrap as Java Number.intValue does;
    /// strings and booleans are not numbers. Null optional fields are absent.
    pub fn from_json(text: &str) -> Result<Self> {
        let raw: &RawValue = serde_json::from_str(text)?;
        let fields = object(raw)?;
        let positive = |name| {
            optional(&fields, name)
                .map(|v| bounded_integer(v, 1))
                .transpose()
        };
        let frames = optional(&fields, "frames")
            .map(|v| {
                array(v)?
                    .into_iter()
                    .map(|v| {
                        if v.get().starts_with('{') {
                            let fields = object(v)?;
                            Ok(AnimationFrame {
                                index: bounded_integer(required(&fields, "index")?, 0)?,
                                time: optional(&fields, "time")
                                    .map(|v| bounded_integer(v, 1))
                                    .transpose()?,
                            })
                        } else {
                            Ok(AnimationFrame {
                                index: bounded_integer(v, 0)?,
                                time: None,
                            })
                        }
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?;
        Ok(Self {
            frames,
            width: positive("width")?,
            height: positive("height")?,
            frame_time: positive("frametime")?.unwrap_or(1),
            interpolate: optional(&fields, "interpolate")
                .map(|v| serde_json::from_str(v.get()))
                .transpose()?
                .unwrap_or(false),
        })
    }

    pub fn frame_size(&self, image_width: i32, image_height: i32) -> [i32; 2] {
        match (self.width, self.height) {
            (Some(w), Some(h)) => [w, h],
            (Some(w), None) => [w, image_height],
            (None, Some(h)) => [image_width, h],
            (None, None) => [image_width.min(image_height); 2],
        }
    }

    /// Build the animation after image/frame dimensions have been validated by
    /// the resource loader. Out-of-image indices are removed, preserving order
    /// and repeats. Zero or one remaining entries mean static physical frame 0.
    pub fn clip(&self, physical_frames: u32) -> Result<Option<AnimationClip>> {
        if self.frame_time <= 0 {
            return Err(error("nonpositive default animation duration"));
        }
        let sequence: Vec<_> = match &self.frames {
            Some(frames) => frames
                .iter()
                .filter_map(|v| {
                    let time = v.time.unwrap_or(self.frame_time);
                    (v.index >= 0 && (v.index as u32) < physical_frames && time > 0)
                        .then_some((v.index as u32, time as u32))
                })
                .collect(),
            None => (0..physical_frames)
                .map(|i| (i, self.frame_time as u32))
                .collect(),
        };
        if sequence.len() <= 1 {
            return Ok(None);
        }
        let mut duration = 0u64;
        let ends = sequence
            .iter()
            .map(|&(_, time)| {
                duration = duration
                    .checked_add(u64::from(time))
                    .ok_or_else(|| error("animation duration overflow"))?;
                Ok(duration)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(AnimationClip {
            sequence,
            ends,
            duration,
            interpolate: self.interpolate,
        }))
    }
}

#[derive(Clone, Debug)]
pub struct AnimationClip {
    sequence: Vec<(u32, u32)>,
    ends: Vec<u64>,
    duration: u64,
    interpolate: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimationSample {
    pub sequence_index: usize,
    pub sub_frame: u32,
    pub frame: u32,
    pub next_frame: u32,
    pub dirty: bool,
    pub needs_draw: bool,
    /// Native drawToAtlas encodes floor((subFrame / time) * 1000) in its
    /// first-vertex argument. This is not a claim about GPU interpolation pixels.
    pub interpolation_millis: u32,
}
impl AnimationClip {
    pub fn sequence(&self) -> &[(u32, u32)] {
        &self.sequence
    }
    pub fn duration(&self) -> u64 {
        self.duration
    }
    pub fn interpolates(&self) -> bool {
        self.interpolate
    }

    /// Random access to the state after `tick` native ticks. Binary search avoids
    /// advancing every missed tick when a render frame takes longer than 50 ms.
    pub fn sample(&self, tick: u64) -> AnimationSample {
        let time = tick % self.duration;
        let index = self.ends.partition_point(|end| *end <= time);
        let start = if index == 0 { 0 } else { self.ends[index - 1] };
        let sub_frame = (time - start) as u32;
        let (frame, duration) = self.sequence[index];
        let previous = (index + self.sequence.len() - 1) % self.sequence.len();
        let dirty = tick == 0 || (sub_frame == 0 && frame != self.sequence[previous].0);
        AnimationSample {
            sequence_index: index,
            sub_frame,
            frame,
            next_frame: self.sequence[(index + 1) % self.sequence.len()].0,
            dirty,
            needs_draw: self.interpolate || dirty,
            interpolation_millis: ((sub_frame as f32 / duration as f32) * 1000.0) as u32,
        }
    }
}
