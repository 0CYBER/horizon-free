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

/// One vertex buffer of the arena; only the newest page grows.
struct Page {
    buffer: wgpu::Buffer,
    /// Capacity in vertices.
    capacity: u64,
    free: FreeList,
}

/// Terrain vertices in pages of at most `page_limit` vertices (devices cap
/// buffer sizes): a section's vertices sit in one page.
pub struct Arena {
    pages: Vec<Page>,
    stride: u64,
    page_limit: u64,
    usage: wgpu::BufferUsages,
}

impl Arena {
    pub fn new(device: &wgpu::Device, stride: u64, capacity: u64) -> Self {
        Self::with_usage(device, stride, capacity, wgpu::BufferUsages::VERTEX)
    }
    pub fn indices(device: &wgpu::Device, capacity: u64) -> Self {
        Self::with_usage(device, 4, capacity, wgpu::BufferUsages::INDEX)
    }
    fn with_usage(
        device: &wgpu::Device,
        stride: u64,
        capacity: u64,
        usage: wgpu::BufferUsages,
    ) -> Self {
        // 256 MiB pages, or less where the device's buffers are smaller.
        let bytes = device.limits().max_buffer_size.min(256 << 20);
        let page_limit = (bytes / stride) / GRAIN * GRAIN;
        let capacity = capacity.next_multiple_of(GRAIN).min(page_limit);
        Self {
            pages: vec![Page {
                buffer: Self::create(device, stride, capacity, usage),
                capacity,
                free: FreeList::new(capacity),
            }],
            stride,
            page_limit,
            usage,
        }
    }
    fn create(
        device: &wgpu::Device,
        stride: u64,
        capacity: u64,
        usage: wgpu::BufferUsages,
    ) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Section buffer arena"),
            size: capacity * stride,
            usage: usage | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }
    /// The rounded length a request of `vertices` occupies.
    fn span(vertices: u64) -> u64 {
        vertices.max(1).next_multiple_of(GRAIN)
    }
    /// Bytes of every page's buffer.
    pub fn bytes(&self) -> u64 {
        self.pages.iter().map(|p| p.capacity).sum::<u64>() * self.stride
    }
    pub fn buffer(&self, page: u32) -> &wgpu::Buffer {
        &self.pages[page as usize].buffer
    }
    /// A range of `vertices`: (page, first vertex). Grows the newest page
    /// (copying its contents on the GPU) or starts a new one when nothing
    /// free is large enough.
    pub fn alloc(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vertices: u64,
    ) -> (u32, u64) {
        let len = Self::span(vertices).min(self.page_limit);
        for (i, page) in self.pages.iter_mut().enumerate() {
            if let Some(start) = page.free.take(len) {
                return (i as u32, start);
            }
        }
        let last = self.pages.len() - 1;
        let current = self.pages[last].capacity;
        if current < self.page_limit {
            let grown = (current * 2).max(current + len).min(self.page_limit);
            if grown - current >= len || grown == self.page_limit {
                self.grow(device, queue, last, grown);
                if let Some(start) = self.pages[last].free.take(len) {
                    return (last as u32, start);
                }
            }
        }
        let capacity = (1 << 20u64).max(len).min(self.page_limit);
        let mut free = FreeList::new(capacity);
        let start = free.take(len).expect("a new page fits one section");
        self.pages.push(Page {
            buffer: Self::create(device, self.stride, capacity, self.usage),
            capacity,
            free,
        });
        ((self.pages.len() - 1) as u32, start)
    }
    pub fn release(&mut self, page: u32, start: u64, vertices: u64) {
        let len = Self::span(vertices).min(self.page_limit);
        self.pages[page as usize].free.give(start, len);
    }
    fn grow(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, page: usize, capacity: u64) {
        let capacity = capacity.next_multiple_of(GRAIN).min(self.page_limit);
        let buffer = Self::create(device, self.stride, capacity, self.usage);
        let old = &mut self.pages[page];
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Grow section arena"),
        });
        encoder.copy_buffer_to_buffer(&old.buffer, 0, &buffer, 0, old.capacity * self.stride);
        // Writes queued for the old buffer run before this copy.
        queue.submit([encoder.finish()]);
        let previous = old.capacity;
        old.buffer = buffer;
        old.capacity = capacity;
        old.free.give(previous, capacity - previous);
    }
    pub fn clear(&mut self) {
        for page in &mut self.pages {
            page.free = FreeList::new(page.capacity);
        }
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
