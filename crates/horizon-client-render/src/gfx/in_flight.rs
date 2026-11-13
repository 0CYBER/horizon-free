//! Frames in flight: the readbacks a submission starts, landing frames the submit thread
//! submitted, keeping offscreen frames at most two ahead of the GPU, and a screenshot's copy.

use super::{Renderer, ScreenshotRequest, copy_frame, screenshot_save, submit};

impl Renderer {
    /// The frame's readbacks to start right after its submission (on the submit thread when
    /// the frame goes there): occlusion culling's results (`cull`) and GPU culling's
    /// statistics. GPU frames leave occlusion culling off, so a frame has one at most.
    pub(super) fn after_submit(&mut self) -> Option<submit::AfterSubmit> {
        let cull = self.cull.as_mut().and_then(|c| c.submitted());
        let gpu = self.gpu_culling.as_mut().and_then(|g| g.submitted());
        match (cull, gpu) {
            (Some(c), Some(g)) => Some(Box::new(move || {
                c();
                g();
            })),
            (Some(c), None) => Some(Box::new(c)),
            (None, Some(g)) => Some(Box::new(g)),
            (None, None) => None,
        }
    }
    /// Frame `frame`'s statistics shown, unless a newer frame's are: a CPU frame's as it is
    /// drawn, a GPU frame's when its counts come back (`GpuCulling::collect`).
    pub(super) fn show_stats(&mut self, frame: u64, [sections, draws, calls]: [usize; 3]) {
        if frame > self.stats_frame {
            self.stats_frame = frame;
            (
                self.drawn_sections,
                self.section_draws,
                self.section_draw_calls,
            ) = (sections, draws, calls);
        }
    }
    /// Frames between the last one drawn and the one whose statistics are shown: 0 after a
    /// CPU frame, one to three in GPU frames, while their counts are on their way back.
    pub fn stats_lag(&self) -> u64 {
        self.frame_serial - self.stats_frame
    }
    /// Take in the frame the submit thread submitted: start its readbacks and, offscreen,
    /// keep at most two frames ahead of the GPU, as a frame submitted in place does.
    pub(super) fn land_frame(&mut self) {
        let Some(offscreen) = self.deferred.take() else {
            return;
        };
        let submission = self.queue.landed();
        if let Some(timer) = &mut self.gpu_timer {
            timer.submitted(&self.queue);
            timer.collect(&self.device, &mut self.gpu_times);
        }
        if let Some(Some(timer)) = Self::pass_timer(&mut self.pack_post, &mut self.native) {
            timer.submitted(&self.queue);
        }
        if let Some(timer) = &mut self.own_timing {
            timer.submitted(&self.queue);
        }
        if offscreen && let Some(submission) = submission {
            self.in_flight.push_back(submission);
            if !self.paced_elsewhere {
                self.wait_frames();
            }
        }
    }
    /// No swapchain paces offscreen frames: keep two on the GPU at most.
    pub fn wait_in_flight(&mut self) {
        self.land_frame();
        self.wait_frames();
    }
    /// The frame on the submit thread submitted: a surface reconfigure waits for the
    /// device and breaks if a frame is submitted while it waits, and the compositor
    /// copies views after them. (The end of a frame already lands the frames of the
    /// turn before; this is the turn's own.)
    pub fn wait_submissions(&mut self) {
        self.queue.wait();
    }
    pub(super) fn wait_frames(&mut self) {
        while self.in_flight.len() > 2 {
            let index = self.in_flight.pop_front();
            let _ = self.device.poll(wgpu::PollType::Wait {
                submission_index: index,
                timeout: Some(std::time::Duration::from_secs(5)),
            });
        }
    }
    /// Take a screenshot request: queue the frame's copy, or report why the
    /// frame cannot be read back.
    pub(super) fn take_readback(
        &mut self,
        request: ScreenshotRequest,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) -> Option<(ScreenshotRequest, wgpu::Buffer)> {
        // The copy and PNG writer both assume 8-bit pixels; a float surface would make
        // the row stride too small for the copy and abort in validation.
        if self.config.format.block_copy_size(None) == Some(4) {
            let size = [self.config.width, self.config.height];
            let format = self.config.format.remove_srgb_suffix();
            // The layered float frame maps the world to SDR, then adds the HUD as drawn.
            // Other 16-bit frames keep their clipped 8-bit copy.
            let copy = self
                .hdr
                .as_mut()
                .and_then(|hdr| {
                    hdr.sdr_screenshot(&self.device, encoder, format, request.size.is_some())
                })
                .or_else(|| {
                    (texture.format() != format && texture.format() == self.output.frame.format)
                        .then(|| {
                            self.readback
                                .unorm_copy(&self.device, encoder, texture, format)
                        })
                });
            let texture = copy.as_ref().unwrap_or(texture);
            Some((request, copy_frame(&self.device, encoder, texture, size)))
        } else {
            let error = format!(
                "screenshots need a 32-bit surface format, not {:?}",
                self.config.format
            );
            if request.background {
                self.saved_screenshots.push(screenshot_save::Saved {
                    path: request.path,
                    result: Err(error),
                    hdr: None,
                });
            } else {
                self.screenshot_result = Some(Err(error));
            }
            None
        }
    }
}
