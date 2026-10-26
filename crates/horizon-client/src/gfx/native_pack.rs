//! One of Horizon's own shader packs while it runs (docs/shaders.md): the frame its stages
//! read, its post stage, and the timer of its passes.

use horizon_shader_api::{Report, StageKind};

use super::native_post::{NativeFrame, NativePost};
use super::pack_timing::PassTiming;

pub struct NativePack {
    pub frame: NativeFrame,
    pub post: Option<NativePost>,
    /// GPU time of each pass, while the renderer asks for it (`pack_timing.rs`).
    pub timing: Option<PassTiming>,
}

impl NativePack {
    /// The pack of the checked stages `report` with the option values `constants`, drawing
    /// onto targets of `format`. Also returns its stages' labels, in the order they draw.
    /// Errors name the stage wgpu refused.
    pub fn new(
        device: &wgpu::Device,
        report: Report,
        constants: &[(String, f64)],
        format: wgpu::TextureFormat,
    ) -> Result<(Self, Vec<String>), String> {
        let frame = NativeFrame::new(device);
        let post: Vec<_> = report
            .stages
            .into_iter()
            .filter(|s| s.kind == StageKind::Post)
            .collect();
        let post = (!post.is_empty())
            .then(|| NativePost::new(device, post, constants, format, &frame))
            .transpose()?;
        let labels = post
            .iter()
            .flat_map(|p| p.labels())
            .map(str::to_owned)
            .collect();
        let pack = Self {
            frame,
            post,
            timing: None,
        };
        Ok((pack, labels))
    }

    /// Whether the pack draws anything the game runs today.
    pub fn draws(&self) -> bool {
        self.post.is_some()
    }
}
