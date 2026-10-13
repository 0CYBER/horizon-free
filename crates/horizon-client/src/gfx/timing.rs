//! GPU frame time from timestamp queries, read back a few frames late so the
//! CPU never waits for it. Frames whose readback slot is still busy go untimed.
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

const SLOTS: usize = 4;
const FREE: u8 = 0;
const PENDING: u8 = 1;
const MAPPED: u8 = 2;

struct Slot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
}

pub struct GpuTimer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    slots: Vec<Slot>,
    /// The slot this frame writes into, when one was free at `begin`.
    current: Option<usize>,
    next: usize,
    period_ns: f32,
}

impl GpuTimer {
    /// The features timing needs; `None` when the adapter lacks them.
    pub fn features(adapter: &wgpu::Adapter) -> wgpu::Features {
        let wanted =
            wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
        if adapter.features().contains(wanted) {
            wanted
        } else {
            wgpu::Features::empty()
        }
    }
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
        {
            return None;
        }
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("Frame timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: 2,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Frame timestamp resolve"),
            size: 16,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let slots = (0..SLOTS)
            .map(|_| Slot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Frame timestamp readback"),
                    size: 16,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(FREE)),
            })
            .collect();
        Some(Self {
            queries,
            resolve,
            slots,
            current: None,
            next: 0,
            period_ns: queue.get_timestamp_period(),
        })
    }
    pub fn begin(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let slot = self.next;
        self.current = (self.slots[slot].state.load(Ordering::Acquire) == FREE).then_some(slot);
        if self.current.is_some() {
            encoder.write_timestamp(&self.queries, 0);
        }
    }
    pub fn end(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let Some(slot) = self.current else { return };
        encoder.write_timestamp(&self.queries, 1);
        encoder.resolve_query_set(&self.queries, 0..2, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.slots[slot].buffer, 0, 16);
    }
    /// After the frame's submission: start reading its timestamps back.
    pub fn submitted(&mut self) {
        let Some(slot) = self.current.take() else {
            return;
        };
        let state = self.slots[slot].state.clone();
        state.store(PENDING, Ordering::Release);
        self.slots[slot]
            .buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                state.store(
                    if result.is_ok() { MAPPED } else { FREE },
                    Ordering::Release,
                );
            });
        self.next = (slot + 1) % SLOTS;
    }
    /// GPU milliseconds of frames whose timestamps have arrived, oldest first.
    pub fn collect(&mut self, device: &wgpu::Device, out: &mut Vec<f32>) {
        let _ = device.poll(wgpu::PollType::Poll);
        for i in 0..SLOTS {
            let slot = &self.slots[(self.next + i) % SLOTS];
            if slot.state.load(Ordering::Acquire) != MAPPED {
                continue;
            }
            if let Ok(data) = slot.buffer.slice(..).get_mapped_range() {
                let stamps: &[u64] = bytemuck::cast_slice(&data);
                let ticks = stamps[1].saturating_sub(stamps[0]);
                out.push((ticks as f64 * f64::from(self.period_ns) / 1e6) as f32);
            }
            slot.buffer.unmap();
            slot.state.store(FREE, Ordering::Release);
        }
    }
}
