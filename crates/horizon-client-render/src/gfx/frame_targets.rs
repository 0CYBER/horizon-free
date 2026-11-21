//! Targets and buffers a frame makes on demand, made so that running out of video memory is not
//! fatal (#13624). Once terrain has filled video memory, a texture made outside an error scope
//! that finds no room reaches wgpu's default error handler, which panics. These are made under
//! out-of-memory and validation scopes instead (a texture that failed is invalid, and so are its
//! views and bind groups), and what failed is dropped, never kept: an effect the frame can do
//! without is left out, and a frame without what it needs is given up and tried again.
//!
//! A validation error with no out-of-memory error in its frame is a bug, and panics as wgpu's
//! handler would.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// What had no room, for its warning: each is logged once.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Kind {
    MenuBlur,
    HdrFrame,
    Depth,
    Readback,
    Mirror,
    Outline,
    Transparency,
    Visibility,
    FrameGeneration,
    Buffers,
}

impl Kind {
    fn consequence(self) -> &'static str {
        match self {
            Kind::MenuBlur => "menus draw without the background blur",
            Kind::HdrFrame => "no HDR frame; frames are skipped until there is room for it",
            Kind::Depth => "no window depth; frames are skipped until there is room for it",
            Kind::Readback => "no frame target; frames are skipped until there is room for it",
            Kind::Mirror => "water draws without its reflection",
            Kind::Outline => "glowing entities draw without their outline",
            Kind::Transparency => "Improved Transparency is off until the window's size changes",
            Kind::Visibility => "solid terrain draws without the visibility buffer",
            Kind::FrameGeneration => "frame generation is off until the window's size changes",
            Kind::Buffers => "a frame that needs a larger buffer is skipped",
        }
    }
}

/// The kinds warned of, one bit each.
static WARNED: AtomicU32 = AtomicU32::new(0);
/// A window frame ran out of memory where no fallback of its own caught it (warned of once).
static FRAME_WARNED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// A scaled screenshot's frame is being drawn (`in_screenshot`).
    static SCREENSHOT: Cell<bool> = const { Cell::new(false) };
    /// A `made` found no room since the window frame began (`begin_frame`).
    static OUT_OF_MEMORY: Cell<bool> = const { Cell::new(false) };
}

#[cfg(test)]
thread_local! {
    /// Tests: the next this many `made` report no room, as a device out of video memory would.
    pub(crate) static NO_ROOM: Cell<u32> = const { Cell::new(0) };
}

/// Draws a scaled screenshot's frame (`draw`), in which `made` makes what it is given without
/// scopes of its own: the frame's own scopes see a failure and take the screenshot again at
/// half the size, which a target left out would not.
pub(crate) fn in_screenshot<T>(draw: impl FnOnce() -> T) -> T {
    let was = SCREENSHOT.replace(true);
    let drawn = draw();
    SCREENSHOT.set(was);
    drawn
}

/// A window frame begins: no `made` in it has found no room yet.
pub(crate) fn begin_frame() {
    OUT_OF_MEMORY.set(false);
}

/// What `make` makes, or `None` where it ran out of memory (warned of once per `kind`); what it
/// made is dropped then. A validation error alone, after no out-of-memory error this frame,
/// panics as wgpu's handler would. Native error scopes resolve at once, so the wait for them
/// costs nothing.
pub(crate) fn made<T>(device: &wgpu::Device, kind: Kind, make: impl FnOnce() -> T) -> Option<T> {
    if SCREENSHOT.get() {
        return Some(make());
    }
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let value = make();
    let memory = pollster::block_on(memory.pop()).map(|e| e.to_string());
    let validation = pollster::block_on(validation.pop()).map(|e| e.to_string());
    #[cfg(test)]
    let memory = memory.or_else(|| {
        let left = NO_ROOM.get().checked_sub(1)?;
        NO_ROOM.set(left);
        Some("no room (test)".to_owned())
    });
    let error = match (memory, validation) {
        (None, None) => return Some(value),
        (Some(error), _) => {
            OUT_OF_MEMORY.set(true);
            error
        }
        // What an earlier failure this frame left invalid.
        (None, Some(error)) if OUT_OF_MEMORY.get() => error,
        (None, Some(error)) => panic!("wgpu error: {error}"),
    };
    let bit = 1 << kind as u32;
    if WARNED.fetch_or(bit, Ordering::Relaxed) & bit == 0 {
        log::warn!(
            "Horizon renderer: video memory is full; {} ({error})",
            kind.consequence()
        );
    }
    None
}

/// A buffer of `size` bytes, for one a frame grows: `None` where there was no room for it.
pub(crate) fn buffer(
    device: &wgpu::Device,
    label: &'static str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> Option<wgpu::Buffer> {
    made(device, Kind::Buffers, || {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage,
            mapped_at_creation: false,
        })
    })
}

/// The verdict on a window frame's own scopes (`Renderer::draw_frame`): `true` where it ran out
/// of memory, so its commands may hold something invalid and the frame is given up. A
/// validation error in a frame that ran out of memory nowhere panics, as wgpu's handler would.
pub(super) fn frame_lost(memory: Option<wgpu::Error>, validation: Option<wgpu::Error>) -> bool {
    let error = match (memory, validation) {
        (None, None) => return false,
        (Some(error), _) => error,
        (None, Some(error)) if OUT_OF_MEMORY.get() => error,
        (None, Some(error)) => panic!("wgpu error: {error}"),
    };
    OUT_OF_MEMORY.set(true);
    warn_frame_lost(&error);
    true
}

/// A frame given up where it ran out of video memory (here, or on the submit thread as its
/// commands were finished), warned of once.
pub(super) fn warn_frame_lost(error: &wgpu::Error) {
    if !FRAME_WARNED.swap(true, Ordering::Relaxed) {
        log::warn!("Horizon renderer: video memory is full; a frame is skipped ({error})");
    }
}

/// What a window frame given up after it took its swapchain image still holds.
pub(super) struct Abandoned {
    /// The frame's commands, where they hold nothing that failed (a buffer that could not
    /// grow kept the old one): submitted, so the uploads recorded before the failure land.
    /// `None` drops them.
    pub encoder: Option<wgpu::CommandEncoder>,
    /// The section copies staged before the frame.
    pub copies: Option<wgpu::CommandBuffer>,
    pub surface: Option<wgpu::SurfaceTexture>,
    /// Frame generation's pair, taken from its present thread.
    pub generated: Option<(wgpu::Texture, wgpu::Texture)>,
    /// A screenshot the frame took, asked for again.
    pub screenshot: Option<super::ScreenshotRequest>,
    /// The frame's own scopes, still open: read first, and where they caught running out of
    /// memory, `encoder` is dropped.
    pub backstop: Option<super::GpuChecks>,
}

impl Abandoned {
    /// A frame given up where a buffer it grows had no room for its new size: the old one was
    /// kept, so its commands so far hold nothing that failed, unless its own scopes (`backstop`)
    /// say otherwise.
    pub fn grown(
        encoder: wgpu::CommandEncoder,
        copies: Option<wgpu::CommandBuffer>,
        surface: Option<wgpu::SurfaceTexture>,
        generated: Option<(wgpu::Texture, wgpu::Texture)>,
        backstop: Option<super::GpuChecks>,
    ) -> Self {
        Self {
            encoder: Some(encoder),
            copies,
            surface,
            generated,
            screenshot: None,
            backstop,
        }
    }
}

impl super::Renderer {
    /// Gives up a window frame after it took its swapchain image (#13624). The image is
    /// cleared to black and presented, as a Vulkan swapchain image that is never presented is
    /// not given back; the section copies staged before it are submitted, and a screenshot it
    /// took is asked for again. `render` then uploads the HUD's patches again, and where the
    /// frame's commands are dropped, the next frame what they uploaded (`upload_again`).
    pub(super) fn abandon_frame(&mut self, frame: Abandoned) -> crate::Result<bool> {
        let lost = frame.backstop.is_some_and(|backstop| {
            let (memory, validation) = backstop.errors();
            frame_lost(memory, validation)
        });
        let encoder = frame.encoder.filter(|_| !lost);
        self.frame_lost |= encoder.is_none();
        let mut encoder = encoder.unwrap_or_else(|| {
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Horizon frame given up"),
                })
        });
        if let Some(surface) = &frame.surface {
            let view = surface.texture.create_view(&wgpu::TextureViewDescriptor {
                format: Some(surface.texture.format().remove_srgb_suffix()),
                ..Default::default()
            });
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Frame given up"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        // The belt's chunks a dropped encoder wrote to come back once this one is submitted.
        self.uploads.finish(&encoder);
        self.queue
            .submit(frame.copies.into_iter().chain([encoder.finish()]));
        if let Some(surface) = frame.surface {
            // As the frame's own present would (Wayland's frame callbacks).
            if self.config.present_mode == wgpu::PresentMode::Fifo {
                self.window.pre_present_notify();
            }
            self.queue.present(surface);
        }
        if frame.generated.is_some() {
            self.frame_gen.stop();
        }
        if let Some(request) = frame.screenshot {
            self.screenshot = Some(request);
        }
        self.frame_given_up();
        Ok(false)
    }
}
