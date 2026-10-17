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
    pages: Vec<Option<Page>>,
    stride: u64,
    page_limit: u64,
    initial_capacity: u64,
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
            pages: Vec::new(),
            stride,
            page_limit,
            initial_capacity: capacity,
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
        self.pages.iter().flatten().map(|p| p.capacity).sum::<u64>() * self.stride
    }
    pub fn buffer(&self, page: u32) -> &wgpu::Buffer {
        &self.pages[page as usize]
            .as_ref()
            .expect("live arena page")
            .buffer
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
            if let Some(page) = page
                && let Some(start) = page.free.take(len)
            {
                return (i as u32, start);
            }
        }
        let last = self.pages.len().saturating_sub(1);
        if let Some(page) = self.pages.last().and_then(Option::as_ref)
            && page.capacity < self.page_limit
        {
            let current = page.capacity;
            let grown = (current * 2).max(current + len).min(self.page_limit);
            if grown - current >= len || grown == self.page_limit {
                self.grow(device, queue, last, grown);
                if let Some(start) = self.pages[last].as_mut().unwrap().free.take(len) {
                    return (last as u32, start);
                }
            }
        }
        let capacity = if self.pages.is_empty() {
            self.initial_capacity
        } else {
            1 << 20
        }
        .max(len)
        .min(self.page_limit);
        let mut free = FreeList::new(capacity);
        let start = free.take(len).expect("a new page fits one section");
        let page = Page {
            buffer: Self::create(device, self.stride, capacity, self.usage),
            capacity,
            free,
        };
        let index = self
            .pages
            .iter()
            .position(Option::is_none)
            .unwrap_or_else(|| {
                self.pages.push(None);
                self.pages.len() - 1
            });
        self.pages[index] = Some(page);
        (index as u32, start)
    }
    pub fn release(&mut self, page: u32, start: u64, vertices: u64) {
        let len = Self::span(vertices).min(self.page_limit);
        self.pages[page as usize]
            .as_mut()
            .expect("live arena page")
            .free
            .give(start, len);
    }
    fn grow(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, page: usize, capacity: u64) {
        let capacity = capacity.next_multiple_of(GRAIN).min(self.page_limit);
        let buffer = Self::create(device, self.stride, capacity, self.usage);
        let old = self.pages[page].as_mut().expect("live arena page");
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
    /// Reclaim after an upload batch, so replacing a section can reuse its range.
    pub fn trim(&mut self) {
        for page in &mut self.pages {
            if let Some(p) = page
                && p.free.0.len() == 1
                && p.free.0.get(&0) == Some(&p.capacity)
            {
                *page = None;
            }
        }
        // Keep live page indices stable; later allocations reuse interior holes.
        while self.pages.last().is_some_and(Option::is_none) {
            self.pages.pop();
        }
    }
    pub fn clear(&mut self) {
        self.pages.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        eprintln!("arena GPU regression: {}", adapter.get_info().name);
        pollster::block_on(adapter.request_device(&Default::default())).ok()
    }

    #[test]
    fn empty_pages_are_reclaimed_without_moving_live_ranges() {
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let mut arena = Arena::new(&device, 4, GRAIN);
        arena.page_limit = 2 * GRAIN;
        assert_eq!(arena.bytes(), 0);
        let (first, a) = arena.alloc(&device, &queue, GRAIN);
        let (_, b) = arena.alloc(&device, &queue, GRAIN);
        let (live, c) = arena.alloc(&device, &queue, 2 * GRAIN);
        let buffer = arena.buffer(live).clone();
        let data = [1u8, 2, 3, 4];
        queue.write_buffer(&buffer, c * 4, &data);
        assert_eq!(arena.bytes(), 4 * GRAIN * 4);
        arena.release(first, a, GRAIN);
        arena.trim();
        assert_eq!(arena.bytes(), 4 * GRAIN * 4);
        arena.release(first, b, GRAIN);
        arena.trim();
        assert_eq!(arena.bytes(), 2 * GRAIN * 4);
        assert_eq!(arena.buffer(live), &buffer);
        let (reused, d) = arena.alloc(&device, &queue, GRAIN);
        assert_eq!(reused, first);

        let (tx, rx) = std::sync::mpsc::channel();
        wgpu::util::DownloadBuffer::read_buffer(
            &device,
            &queue,
            &arena.buffer(live).slice(c * 4..c * 4 + 4),
            move |result| tx.send(result.unwrap().to_vec()).unwrap(),
        );
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert_eq!(rx.recv().unwrap(), data);
        arena.release(reused, d, GRAIN);
        arena.release(live, c, 2 * GRAIN);
        arena.trim();
        assert_eq!(arena.bytes(), 0);
        assert!(arena.pages.is_empty());
    }

    #[test]
    fn clearing_an_arena_releases_buffers_and_can_allocate_again() {
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        for mut arena in [
            Arena::new(&device, 28, GRAIN),
            Arena::indices(&device, GRAIN),
        ] {
            arena.alloc(&device, &queue, GRAIN);
            assert_eq!(arena.bytes(), arena.stride * GRAIN);
            arena.clear();
            assert_eq!(arena.bytes(), 0);
            assert_eq!(arena.alloc(&device, &queue, GRAIN), (0, 0));
            assert_eq!(arena.bytes(), arena.stride * GRAIN);
        }
    }

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
