//! Pack sky submission and visibility state.
use super::*;

impl PackPost {
    /// The sky's parts in vanilla's order, each with the program that draws it; one render pass
    /// while consecutive programs write the same buffers.
    pub(super) fn draw_sky(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        frame: &super::super::Frame,
    ) {
        let Some(mut sky) = self.sky.take() else {
            return;
        };
        let parts = sky.geometry.prepare(
            queue,
            frame,
            self.uniforms.shadow.sun_path_rotation,
            self.sky_visibility,
        );
        let mut groups = [None, None];
        for (i, pass) in [&mut sky.basic, &mut sky.textured].into_iter().enumerate() {
            if let Some(pass) = pass {
                groups[i] = Some(self.bind(device, queue, encoder, pass, size));
                self.mips_valid &= !pass.written;
            }
        }
        let passes = [&sky.basic, &sky.textured];
        let mut open: Option<(wgpu::RenderPass<'_>, usize)> = None;
        for part in parts {
            let i = usize::from(part.textured);
            let (Some(pass), Some(group)) = (passes[i], &groups[i]) else {
                continue;
            };
            let current = open.as_ref().map(|o| o.1);
            if current != Some(i) {
                let same = current
                    .and_then(|c| passes[c].as_ref())
                    .is_some_and(|p| p.writes == pass.writes);
                if !same {
                    drop(open.take());
                    open = Some((self.begin(encoder, pass).forget_lifetime(), i));
                }
                let rp = &mut open.as_mut().expect("open").0;
                rp.set_pipeline(pass.render());
                rp.set_bind_group(0, group, &[]);
                // A sky program that reads the pack's SSBOs (Noble's `gbuffers_textured`, the
                // sun's fallback) binds them, as the other passes do.
                if pass.buffers
                    && let Some(g) = &self.buffers.group
                {
                    rp.set_bind_group(4, g, &[]);
                }
                sky.geometry.bind(rp, part.textured);
                open.as_mut().expect("open").1 = i;
            }
            let rp = &mut open.as_mut().expect("open").0;
            // Each part with vanilla's blend for it: one program, three pipelines.
            let part_pipeline =
                pass.sky_parts
                    .as_ref()
                    .and_then(|[plain, stars]| match part.stage {
                        "SKY" | "VOID" => plain.get(),
                        "STARS" => stars.get(),
                        _ => None,
                    });
            rp.set_pipeline(part_pipeline.unwrap_or(pass.render()));
            if pass.draw {
                self.draws.bind(rp, part.stage);
            }
            rp.draw(part.range, 0..1);
        }
        drop(open);
        self.sky = Some(sky);
    }

    /// Visibility requests remain active when a pack falls back to native sky geometry.
    pub fn sky_visibility(&self) -> horizon_shaders::SkyVisibility {
        self.sky_visibility
    }
}
