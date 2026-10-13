//! One vertex buffer for every terrain section: sections own ranges of it, so
//! a frame binds it once and draws every section with indirect multi-draw.
use std::collections::BTreeMap;

/// Ranges are rounded up to this many vertices to limit fragmentation.
const GRAIN: u64 = 256;

/// Free vertex ranges: start -> length, never adjacent.
#[derive(Debug, PartialEq)]
struct FreeList(BTreeMap<u64, u64>);

impl FreeList {
    fn new(capacity: u64) -> Self {
        Self(BTreeMap::from([(0, capacity)]))
    }
    /// First fit.
    fn take(&mut self, len: u64) -> Option<u64> {
        let (&start, &free) = self.0.iter().find(|(_, l)| **l >= len)?;
        self.0.remove(&start);
        if free > len {
            self.0.insert(start + len, free - len);
        }
        Some(start)
    }
    fn give(&mut self, start: u64, len: u64) {
        let (mut start, mut len) = (start, len);
        if let Some((&prev, &prev_len)) = self.0.range(..start).next_back()
            && prev + prev_len == start
        {
            self.0.remove(&prev);
            start = prev;
            len += prev_len;
        }
        if let Some(next_len) = self.0.remove(&(start + len)) {
            len += next_len;
        }
        self.0.insert(start, len);
    }
}

pub struct Arena {
    pub buffer: wgpu::Buffer,
    /// Capacity in vertices.
    capacity: u64,
    stride: u64,
    free: FreeList,
}

impl Arena {
    pub fn new(device: &wgpu::Device, stride: u64, capacity: u64) -> Self {
        let capacity = capacity.next_multiple_of(GRAIN);
        Self {
            buffer: Self::create(device, stride, capacity),
            capacity,
            stride,
            free: FreeList::new(capacity),
        }
    }
    fn create(device: &wgpu::Device, stride: u64, capacity: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Section vertex arena"),
            size: capacity * stride,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }
    /// The rounded length a request of `vertices` occupies.
    fn span(vertices: u64) -> u64 {
        vertices.max(1).next_multiple_of(GRAIN)
    }
    /// A range of `vertices`, growing the buffer (copying its contents on the
    /// GPU) when no free range is large enough. Returns the first vertex.
    pub fn alloc(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, vertices: u64) -> u64 {
        let len = Self::span(vertices);
        if let Some(start) = self.free.take(len) {
            return start;
        }
        self.grow(device, queue, (self.capacity * 2).max(self.capacity + len));
        self.free.take(len).expect("grown arena has room")
    }
    pub fn release(&mut self, start: u64, vertices: u64) {
        self.free.give(start, Self::span(vertices));
    }
    fn grow(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, capacity: u64) {
        let capacity = capacity.next_multiple_of(GRAIN);
        let buffer = Self::create(device, self.stride, capacity);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Grow section arena"),
        });
        encoder.copy_buffer_to_buffer(&self.buffer, 0, &buffer, 0, self.capacity * self.stride);
        // Writes queued for the old buffer run before this copy.
        queue.submit([encoder.finish()]);
        let old = self.capacity;
        self.buffer = buffer;
        self.capacity = capacity;
        self.free.give(old, capacity - old);
    }
    pub fn clear(&mut self) {
        self.free = FreeList::new(self.capacity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_ranges_coalesce() {
        let mut free = FreeList::new(4 * GRAIN);
        let a = free.take(GRAIN).unwrap();
        let b = free.take(2 * GRAIN).unwrap();
        let c = free.take(GRAIN).unwrap();
        assert_eq!((a, b, c), (0, GRAIN, 3 * GRAIN));
        assert_eq!(free.take(1), None);
        free.give(a, GRAIN);
        free.give(c, GRAIN);
        // Two holes of one grain: a two-grain request does not fit.
        assert_eq!(free.take(2 * GRAIN), None);
        free.give(b, 2 * GRAIN);
        assert_eq!(free, FreeList::new(4 * GRAIN));
    }

    #[test]
    fn first_fit_reuses_the_lowest_hole() {
        let mut free = FreeList::new(8 * GRAIN);
        let a = free.take(GRAIN).unwrap();
        let _b = free.take(GRAIN).unwrap();
        free.give(a, GRAIN);
        assert_eq!(free.take(GRAIN), Some(a));
    }
}
