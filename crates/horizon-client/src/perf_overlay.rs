//! The performance overlay (Horizon's Overlay setting): frames per second,
//! frame, CPU, GPU and wait times, and a graph of recent frame times.
use crate::{
    graphics::Overlay,
    hud::{HudAtlas, HudBatch, rgba},
    text::Text,
};
use std::collections::VecDeque;

const HISTORY: usize = 160;

#[derive(Default)]
pub struct FrameGraph {
    /// Frame, CPU, GPU and wait milliseconds of recent frames, oldest first.
    frames: VecDeque<(f32, f32, f32, f32)>,
    /// The resolution scale Dynamic Resolution has the world at, in percent, while it is on:
    /// the numbers above were measured at a resolution that moves.
    pub dynamic_scale: Option<u32>,
}
impl FrameGraph {
    /// One frame's times. `wait_ms`: the render thread blocked on the GPU or the display
    /// (acquiring, presenting, the submit thread), in the frame but in neither the CPU's time
    /// nor the GPU's.
    pub fn record(&mut self, frame_ms: f32, cpu_ms: f32, gpu_ms: Option<f32>, wait_ms: f32) {
        let gpu = gpu_ms.unwrap_or_else(|| self.frames.back().map_or(0.0, |f| f.2));
        if self.frames.len() == HISTORY {
            self.frames.pop_front();
        }
        self.frames.push_back((frame_ms, cpu_ms, gpu, wait_ms));
    }
    pub fn draw(&self, mode: Overlay, batch: &mut HudBatch, atlas: &HudAtlas) {
        if mode == Overlay::Off || self.frames.is_empty() {
            return;
        }
        let n = self.frames.len() as f32;
        let avg = |f: fn(&(f32, f32, f32, f32)) -> f32| self.frames.iter().map(f).sum::<f32>() / n;
        let (frame, cpu, gpu, wait) = (avg(|f| f.0), avg(|f| f.1), avg(|f| f.2), avg(|f| f.3));
        let worst = self.frames.iter().map(|f| f.0).fold(0.0f32, f32::max);
        let lines = [
            match self.dynamic_scale {
                Some(scale) => format!("{:.0} fps  res {scale}%", 1000.0 / frame.max(0.001)),
                None => format!("{:.0} fps", 1000.0 / frame.max(0.001)),
            },
            format!("frame {frame:.1} ms  max {worst:.1}"),
        ];
        // The times on a line of their own, each right-aligned in a slot as wide as the widest
        // two-digit time, so neither they nor the box move as they change.
        let times = [("cpu ", cpu), ("  gpu ", gpu), ("  wait ", wait)]
            .map(|(label, ms)| (label, format!("{ms:.1}")));
        let digit = (0..10)
            .map(|d| atlas.font.width(&d.to_string()))
            .fold(0.0, f32::max);
        let slot = |value: &str| (3.0 * digit + atlas.font.width(".")).max(atlas.font.width(value));
        let times_width = times
            .iter()
            .map(|(label, value)| atlas.font.width(label) + slot(value.as_str()))
            .sum::<f32>()
            + atlas.font.width(" ms");
        let rows = lines.len() + 1;
        let width = 120.0f32.max(
            lines
                .iter()
                .map(|l| atlas.font.width(l))
                .fold(times_width, f32::max)
                + 6.0,
        );
        let x = batch.width - width - 2.0;
        let graph = if mode == Overlay::Graph { 42.0 } else { 0.0 };
        batch.fill(
            atlas,
            x,
            2.0,
            width,
            4.0 + rows as f32 * 10.0 + graph,
            rgba(0, 0, 0, 144),
        );
        for (i, line) in lines.iter().enumerate() {
            let color = if i == 0 {
                rgba(255, 255, 255, 255)
            } else {
                rgba(200, 200, 200, 255)
            };
            batch.text_styled(
                atlas,
                &Text::literal(line.clone()),
                x + 3.0,
                4.0 + i as f32 * 10.0,
                color,
                true,
            );
        }
        let grey = rgba(200, 200, 200, 255);
        let y = 4.0 + lines.len() as f32 * 10.0;
        let mut tx = x + 3.0;
        for (label, value) in &times {
            batch.text_styled(atlas, &Text::literal(*label), tx, y, grey, true);
            tx += atlas.font.width(label) + slot(value.as_str());
            let vx = tx - atlas.font.width(value);
            batch.text_styled(atlas, &Text::literal(value.clone()), vx, y, grey, true);
        }
        batch.text_styled(atlas, &Text::literal(" ms"), tx, y, grey, true);
        if mode != Overlay::Graph {
            return;
        }
        // Bars of frame time on a 0..33 ms scale; lines at 60 and 30 fps.
        let (gy, gh) = (4.0 + rows as f32 * 10.0 + 2.0, 36.0);
        let scale = gh / 33.3;
        let bar = (width - 6.0) / HISTORY as f32;
        for (i, &(ms, cpu, _, _)) in self.frames.iter().enumerate() {
            let height = (ms * scale).min(gh);
            let bx = x + 3.0 + i as f32 * bar;
            let color = if ms > 33.3 {
                rgba(240, 90, 80, 255)
            } else if ms > 16.7 {
                rgba(240, 200, 90, 255)
            } else {
                rgba(110, 210, 120, 255)
            };
            batch.fill(atlas, bx, gy + gh - height, bar.max(1.0), height, color);
            let cpu_height = (cpu * scale).min(height);
            batch.fill(
                atlas,
                bx,
                gy + gh - cpu_height,
                bar.max(1.0),
                cpu_height,
                rgba(80, 140, 230, 200),
            );
        }
        for target in [16.7, 33.3] {
            let ly = gy + gh - target * scale;
            batch.fill(
                atlas,
                x + 3.0,
                ly,
                width - 6.0,
                1.0,
                rgba(255, 255, 255, 90),
            );
        }
    }
}
