//! Fixed-clock video export as a client mod requests and reads it.

/// A fixed-clock offscreen capture. Wire fields are fixed once published.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ExportRequest {
    /// The new directory's name inside the game directory's `exports`.
    pub name: String,
    /// Output width in pixels, even and at least 16.
    pub width: u32,
    /// Output height in pixels, even and at least 16.
    pub height: u32,
    /// Frame rate numerator.
    pub fps_num: u32,
    /// Frame rate denominator. The rate must be in 1..=240 FPS.
    pub fps_den: u32,
    /// Frame limit (up to 216,000), or zero to run until stopped or the replay ends.
    pub frames: u32,
    /// Where the engine sends the rendered frames.
    pub format: ExportFormat,
    /// Whether to leave the player's HUD visibility in effect.
    pub hud: bool,
}

impl ExportRequest {
    /// Capture up to `frames` at the given dimensions and rational frame rate.
    /// Defaults to PNG output with the HUD hidden.
    pub fn new(name: impl Into<String>, size: [u32; 2], fps: (u32, u32), frames: u32) -> Self {
        Self {
            name: name.into(),
            width: size[0],
            height: size[1],
            fps_num: fps.0,
            fps_den: fps.1,
            frames,
            format: ExportFormat::PngSequence,
            hud: false,
        }
    }
}

/// Engine-owned output. Wire variants are fixed once published.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ExportFormat {
    /// Numbered sRGB PNGs.
    PngSequence,
    /// The player's configured external encoder.
    Encoder,
}

/// The client's active or most recent mod-started export. Wire fields are fixed once published.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ExportStatus {
    /// The id returned by `start_export`.
    pub request: u32,
    /// The directory name passed to `start_export`.
    pub name: String,
    /// Whether the export is still running (including setup and draining) or has ended.
    pub state: ExportState,
    /// Zero-based index during a frame's hooks (including `camera_pose`). Between frames this
    /// is the number submitted; after completion it is the number written.
    pub frame: u32,
    /// Requested frame limit, or zero for no limit.
    pub frames: u32,
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Frame rate numerator.
    pub fps_num: u32,
    /// Frame rate denominator.
    pub fps_den: u32,
    /// Bytes written as complete PNG frames or sent to the encoder as complete RGB frames.
    pub bytes: u64,
}

/// The export's state. Wire variants are fixed once published.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ExportState {
    /// Accepted, preparing or drawing frames, or draining output.
    Running,
    /// All accepted frames and the manifest were finalized, including after a graceful stop.
    Finished,
    /// The export failed to start, write, or finalize.
    Failed,
}

/// Why a call could not start or stop an export. Wire variants are fixed once published.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ExportError {
    /// The name is not a safe export directory name.
    InvalidName,
    /// The dimensions are odd or outside 16..=8192 or the renderer's texture limit.
    InvalidSize,
    /// The frame rate is invalid or the frame limit exceeds 216,000.
    InvalidRate,
    /// The named export directory already exists.
    Exists,
    /// Another capture or script is active, or the export completion hook is running.
    Busy,
    /// The request would exceed the session's shared 64 GiB mod export allowance.
    BudgetExhausted,
    /// The player's encoder could not be configured or started.
    EncoderUnavailable,
    /// This client cannot export, or has used every request id.
    Unavailable,
    /// The world is not in play or is changing.
    WorldBusy,
    /// There is no mod-started export to stop.
    NotRunning,
    /// The output folder could not be opened or created.
    Folder,
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidName => "invalid export name",
            Self::InvalidSize => "invalid export size",
            Self::InvalidRate => "invalid export frame rate or limit",
            Self::Exists => "the export directory already exists",
            Self::Busy => "another capture or script is active",
            Self::BudgetExhausted => "mod exports have used this session's budget",
            Self::EncoderUnavailable => "the export encoder is unavailable",
            Self::Unavailable => "video export is unavailable",
            Self::WorldBusy => "video export needs a world in play",
            Self::NotRunning => "there is no mod export to stop",
            Self::Folder => "the export folder could not be created",
        })
    }
}

impl std::error::Error for ExportError {}

impl crate::NoBackend for ExportError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
