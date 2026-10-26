//! Developers' aid: `HORIZON_SHADER_PACK_TIMING=1` times every pass of a shader pack on the
//! GPU, one frame in `EVERY`, and logs each pass's median every `REPORT` timed frames. The
//! frame after a timed one waits for the GPU before reading the timestamps (Metal resolves
//! them without waiting for the passes that write them), so timing slows the game.

const EVERY: u32 = 30;
const REPORT: usize = 30;
/// Passes timed per frame at most (two timestamps each).
const CAPACITY: u32 = 64;

pub(super) struct PassTiming {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
    period_ns: f32,
    frame: u32,
    /// The passes timed in the frame being recorded or waiting to be read, in order.
    names: Vec<String>,
    timed: bool,
    samples: Vec<(String, Vec<f32>)>,
    reports: usize,
}

impl PassTiming {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if std::env::var_os("HORIZON_SHADER_PACK_TIMING").is_none()
            || !device.features().contains(wgpu::Features::TIMESTAMP_QUERY)
        {
            return None;
        }
        let size = u64::from(CAPACITY) * 2 * 8;
        let buffer = |label, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        Some(Self {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("Shader pack pass timing"),
                ty: wgpu::QueryType::Timestamp,
                count: CAPACITY * 2,
            }),
            resolve: buffer(
                "Shader pack timing resolve",
                wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            ),
            read: buffer(
                "Shader pack timing read",
                wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            ),
            period_ns: queue.get_timestamp_period(),
            frame: 0,
            names: Vec::new(),
            timed: false,
            samples: Vec::new(),
            reports: 0,
        })
    }

    /// At the start of a frame: reads the last timed frame's passes, and decides whether this
    /// one is timed.
    pub fn begin(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.timed && !self.names.is_empty() {
            self.collect(device, queue);
        }
        self.names.clear();
        self.frame += 1;
        self.timed = self.frame.is_multiple_of(EVERY);
    }

    /// Timestamp writes for the pass `name`, in a timed frame.
    pub fn writes(&mut self, name: &str) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        let i = self.names.len() as u32;
        if !self.timed || i >= CAPACITY {
            return None;
        }
        self.names.push(name.to_owned());
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(2 * i),
            end_of_pass_write_index: Some(2 * i + 1),
        })
    }

    fn collect(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        // The timed frame first, then the resolve, so every timestamp is written.
        _ = device.poll(wgpu::PollType::wait_indefinitely());
        let count = self.names.len() as u32 * 2;
        let bytes = u64::from(count) * 8;
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.resolve_query_set(&self.queries, 0..count, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.read, 0, bytes);
        queue.submit([encoder.finish()]);
        let slice = self.read.slice(..bytes);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        _ = device.poll(wgpu::PollType::wait_indefinitely());
        let Ok(data) = slice.get_mapped_range() else {
            return;
        };
        let stamps: Vec<u64> = data
            .as_chunks::<8>()
            .0
            .iter()
            .map(|b| u64::from_le_bytes(*b))
            .collect();
        drop(data);
        self.read.unmap();
        // Passes overlap on tiled GPUs (a pass begins before the one before it ends): a pass
        // costs the time from the later of its beginning and the previous pass's end.
        let mut previous_end = 0;
        for (name, pair) in self.names.iter().zip(stamps.as_chunks::<2>().0) {
            if pair[0] == 0 || pair[1] < pair[0] {
                continue;
            }
            let from = pair[0].max(previous_end).min(pair[1]);
            previous_end = pair[1];
            let ms = (pair[1] - from) as f32 * self.period_ns / 1e6;
            match self.samples.iter_mut().find(|s| s.0 == *name) {
                Some(s) => s.1.push(ms),
                None => self.samples.push((name.clone(), vec![ms])),
            }
        }
        self.reports += 1;
        if self.reports.is_multiple_of(REPORT) {
            self.report();
        }
    }

    fn report(&mut self) {
        let mut total = 0.0;
        let mut parts = Vec::new();
        for (name, v) in &mut self.samples {
            v.sort_by(f32::total_cmp);
            let median = v[v.len() / 2];
            total += median;
            parts.push(format!("{name} {median:.3}"));
            v.clear();
        }
        log::info!(
            "shader pack GPU time per pass (median ms of {REPORT} frames): {} = {total:.3}",
            parts.join(", ")
        );
    }
}
