//! Deterministic frame timing and output for the offscreen exporter.

mod clock;
mod config;
mod dir;
mod process;
mod sink;
mod writer;

pub use clock::{ClockError, ExportClock, FrameStep};
pub use config::{ConfigError, EncoderConfig};
pub use dir::{DirError, ExportFolder, ExportManifest};
pub use sink::{EncoderPipe, FrameSink, PngSequence};
pub use writer::Writer;

use crate::file_id::valid_export_name;
use serde::Serialize;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExportSpec {
    pub name: String,
    pub size: [u32; 2],
    /// Frames per real second as a numerator and denominator.
    pub fps: (u32, u32),
    /// Zero runs until stopped or the replay ends.
    pub frames: u32,
    pub format: Format,
    pub hud: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    PngSequence,
    Encoder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecError {
    InvalidName,
    InvalidSize,
    InvalidFps,
    TooManyFrames,
}

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidName => "invalid export name",
            Self::InvalidSize => {
                "export dimensions must be even and between 16 and the texture limit (at most 8192)"
            }
            Self::InvalidFps => "export frame rate must be between 1 and 240 frames per second",
            Self::TooManyFrames => "export cannot request more than 216000 frames",
        })
    }
}

impl std::error::Error for SpecError {}

impl ExportSpec {
    pub fn validate(&self, max_texture: u32) -> Result<(), SpecError> {
        if !valid_export_name(&self.name) {
            return Err(SpecError::InvalidName);
        }
        if self
            .size
            .iter()
            .any(|side| !(16..=8192.min(max_texture)).contains(side) || side % 2 != 0)
        {
            return Err(SpecError::InvalidSize);
        }
        validate_fps(self.fps)?;
        if self.frames > 216_000 {
            return Err(SpecError::TooManyFrames);
        }
        Ok(())
    }
}

fn validate_fps((num, den): (u32, u32)) -> Result<(), SpecError> {
    if den == 0 || num < den || u64::from(num) > u64::from(den) * 240 {
        return Err(SpecError::InvalidFps);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_render_and_recording_limits() {
        let spec = ExportSpec {
            name: "portrait".into(),
            size: [1080, 1920],
            fps: (30_000, 1001),
            frames: 216_000,
            format: Format::PngSequence,
            hud: false,
        };
        assert_eq!(spec.validate(1920), Ok(()));
        assert_eq!(spec.validate(1919), Err(SpecError::InvalidSize));
        for size in [[14, 16], [16, 17], [8194, 16]] {
            let spec = ExportSpec {
                size,
                ..spec.clone()
            };
            assert_eq!(spec.validate(u32::MAX), Err(SpecError::InvalidSize));
        }
        for fps in [(0, 1), (1, 0), (999, 1000), (241_000, 1000)] {
            let spec = ExportSpec {
                fps,
                ..spec.clone()
            };
            assert_eq!(spec.validate(8192), Err(SpecError::InvalidFps));
        }
        let too_long = ExportSpec {
            frames: 216_001,
            ..spec.clone()
        };
        assert_eq!(too_long.validate(8192), Err(SpecError::TooManyFrames));
        for name in ["../portrait", "portrait?"] {
            let unsafe_name = ExportSpec {
                name: name.into(),
                ..spec.clone()
            };
            assert_eq!(unsafe_name.validate(8192), Err(SpecError::InvalidName));
        }
    }
}
