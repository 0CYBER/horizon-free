//! An offscreen capture queues copies in submission order. The GPU and writer
//! can each hold a few frames while the render thread prepares the next one.

use super::{GpuChecks, Renderer, Result, copy_frame_into, drawable_size, padded_row};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
        mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use wgpu::TextureFormat;

const SLOTS: usize = 3;
const CAPTURING: u8 = 0;
const DRAINING: u8 = 1;
const ABORTED: u8 = 2;
pub const FRAME_EXPORT_STALL_TIMEOUT: Duration = Duration::from_secs(60);

pub struct ExportFrame {
    pub index: u32,
    pub size: [u32; 2],
    pub rgb: Vec<u8>,
}

struct Copy {
    index: u32,
    buffer: wgpu::Buffer,
    submission: wgpu::SubmissionIndex,
    bgra: bool,
}

pub(super) struct Readbacks {
    original_size: [u32; 2],
    size: [u32; 2],
    available: Receiver<wgpu::Buffer>,
    armed: Option<(u32, wgpu::Buffer)>,
    reserved: Option<wgpu::Buffer>,
    bgra: bool,
    copies: Option<SyncSender<Copy>>,
    recycle: SyncSender<Vec<u8>>,
    state: Arc<AtomicU8>,
    worker: Option<JoinHandle<io::Result<()>>>,
}

impl Readbacks {
    fn new(
        device: &wgpu::Device,
        original_size: [u32; 2],
        size: [u32; 2],
        out: SyncSender<ExportFrame>,
        stall_timeout: Duration,
    ) -> Result<Self> {
        let bytes = u64::from(padded_row(size[0])) * u64::from(size[1]);
        if bytes > device.limits().max_buffer_size {
            return Err("export readback exceeds the device's buffer limit".into());
        }
        let (return_buffer, available) = sync_channel(SLOTS);
        let checks = GpuChecks::push(device);
        for _ in 0..SLOTS {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Horizon export readback"),
                size: bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            return_buffer.try_send(buffer).expect("new export pool");
        }
        if let Some(error) = checks.pop() {
            return Err(format!("could not allocate export readbacks: {error}").into());
        }

        let (copies, ready) = sync_channel::<Copy>(SLOTS);
        let (recycle, recycled) = sync_channel::<Vec<u8>>(SLOTS);
        let state = Arc::new(AtomicU8::new(CAPTURING));
        let control = Arc::clone(&state);
        let device = device.clone();
        let worker = thread::Builder::new()
            .name("frame-readback".into())
            .spawn(move || {
                for copy in ready {
                    if control.load(Ordering::Acquire) == ABORTED {
                        break;
                    }
                    let slice = copy.buffer.slice(..);
                    let (mapped, result) = sync_channel(1);
                    slice.map_async(wgpu::MapMode::Read, move |status| {
                        let _ = mapped.try_send(status);
                    });
                    device
                        .poll(wgpu::PollType::Wait {
                            submission_index: Some(copy.submission),
                            timeout: Some(Duration::from_secs(10)),
                        })
                        .map_err(|e| io::Error::other(e.to_string()))?;
                    result
                        .recv_timeout(Duration::from_secs(10))
                        .map_err(|e| io::Error::other(e.to_string()))?
                        .map_err(|e| io::Error::other(e.to_string()))?;
                    let mut rgb = recycled.try_recv().unwrap_or_default();
                    {
                        let mapped = slice
                            .get_mapped_range()
                            .map_err(|e| io::Error::other(e.to_string()))?;
                        unpack_rgb(&mapped, size, copy.bgra, &mut rgb);
                    }
                    copy.buffer.unmap();
                    send_frame(
                        &out,
                        &control,
                        stall_timeout,
                        ExportFrame {
                            index: copy.index,
                            size,
                            rgb,
                        },
                    )?;
                    let _ = return_buffer.try_send(copy.buffer);
                }
                Ok(())
            })?;
        Ok(Self {
            original_size,
            size,
            available,
            armed: None,
            reserved: None,
            bgra: false,
            copies: Some(copies),
            recycle,
            state,
            worker: Some(worker),
        })
    }

    #[cfg(test)]
    fn arm(&mut self, index: u32) -> Result<()> {
        if !self.try_arm(index)? {
            return Err(std::io::Error::from(std::io::ErrorKind::WouldBlock).into());
        }
        Ok(())
    }

    fn try_arm(&mut self, index: u32) -> Result<bool> {
        if self.copies.is_none() {
            return Err("frame export is draining".into());
        }
        if let Some((armed, _)) = &self.armed {
            return if *armed == index {
                Ok(true)
            } else {
                Err("another export frame is already armed".into())
            };
        }
        let buffer = match self.reserved.take() {
            Some(buffer) => buffer,
            None => match self.available.try_recv() {
                Ok(buffer) => buffer,
                Err(TryRecvError::Empty) => return Ok(false),
                Err(error) => return Err(error.into()),
            },
        };
        self.armed = Some((index, buffer));
        Ok(true)
    }

    pub(super) fn armed(&self) -> bool {
        self.armed.is_some()
    }

    pub(super) fn draw_allowed(export: Option<&Self>) -> bool {
        export.is_none_or(Self::armed)
    }

    pub(super) fn copy(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) -> Result<()> {
        self.bgra = rgb_order(texture.format()).ok_or("unsupported SDR capture format")?;
        let size = texture.size();
        if [size.width, size.height] != self.size {
            return Err("export capture source changed size".into());
        }
        if !texture.usage().contains(wgpu::TextureUsages::COPY_SRC) {
            return Err("export has no readable SDR capture source".into());
        }
        let (_, buffer) = self.armed.as_ref().expect("armed export copy");
        copy_frame_into(encoder, texture, buffer, self.size);
        Ok(())
    }

    pub(super) fn submitted(&mut self, submission: wgpu::SubmissionIndex) -> Result<()> {
        let (index, buffer) = self.armed.take().expect("submitted export copy");
        self.copies.as_ref().unwrap().send(Copy {
            index,
            buffer,
            submission,
            bgra: self.bgra,
        })?;
        Ok(())
    }

    fn begin_finish(&mut self) {
        let _ =
            self.state
                .compare_exchange(CAPTURING, DRAINING, Ordering::AcqRel, Ordering::Acquire);
        self.armed.take();
        self.reserved.take();
        self.copies.take();
        if let Some(worker) = &self.worker {
            worker.thread().unpark();
        }
    }

    fn is_finished(&self) -> bool {
        self.copies.is_none() && self.worker.as_ref().is_none_or(JoinHandle::is_finished)
    }

    fn finish(&mut self) -> Result<()> {
        self.begin_finish();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| "export readback worker panicked")??;
        }
        Ok(())
    }

    fn abort(&mut self) {
        self.state.store(ABORTED, Ordering::Release);
        self.begin_finish();
        // A mapped copy may still be waiting on the device. Its worker owns
        // everything it needs until the wait returns.
        self.worker.take();
    }
}

impl Drop for Readbacks {
    fn drop(&mut self) {
        self.abort();
    }
}

fn send_frame(
    out: &SyncSender<ExportFrame>,
    state: &AtomicU8,
    stall_timeout: Duration,
    mut frame: ExportFrame,
) -> io::Result<()> {
    let mut stalled_since = None;
    loop {
        let state = state.load(Ordering::Acquire);
        if state == ABORTED {
            return Ok(());
        }
        match out.try_send(frame) {
            Ok(()) => return Ok(()),
            Err(TrySendError::Disconnected(_)) => {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "export receiver disconnected",
                ));
            }
            Err(TrySendError::Full(pending)) => frame = pending,
        }
        // Give each frame a fresh handoff deadline once draining starts.
        if state == DRAINING
            && stalled_since.get_or_insert_with(Instant::now).elapsed() >= stall_timeout
        {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "export output stalled while draining a frame",
            ));
        }
        thread::park_timeout(Duration::from_millis(2));
    }
}

fn rgb_order(format: TextureFormat) -> Option<bool> {
    match format.remove_srgb_suffix() {
        TextureFormat::Bgra8Unorm => Some(true),
        TextureFormat::Rgba8Unorm => Some(false),
        _ => None,
    }
}

fn unpack_rgb(mapped: &[u8], [width, height]: [u32; 2], bgra: bool, rgb: &mut Vec<u8>) {
    rgb.clear();
    rgb.reserve(width as usize * height as usize * 3);
    let (red, blue) = if bgra { (2, 0) } else { (0, 2) };
    for row in mapped.chunks_exact(padded_row(width) as usize) {
        for p in row[..width as usize * 4].as_chunks::<4>().0 {
            rgb.extend_from_slice(&[p[red], p[1], p[blue]]);
        }
    }
}

impl Renderer {
    pub fn export_adapter_name(&self) -> String {
        self.adapter.get_info().name
    }

    pub fn begin_frame_export(
        &mut self,
        size: [u32; 2],
        out: SyncSender<ExportFrame>,
    ) -> Result<()> {
        if self.frame_export.is_some() || self.surface.is_some() || self.offscreen.is_none() {
            return Err("frame export needs an idle offscreen renderer".into());
        }
        if rgb_order(self.config.format).is_none() {
            return Err("frame export needs an 8-bit RGBA or BGRA target".into());
        }
        if !drawable_size(
            size[0],
            size[1],
            self.device.limits().max_texture_dimension_2d,
        ) {
            return Err("invalid export target size".into());
        }
        let original = [self.config.width, self.config.height];
        let readbacks = Readbacks::new(
            &self.device,
            original,
            size,
            out,
            FRAME_EXPORT_STALL_TIMEOUT,
        )?;
        self.land_frame();
        self.frame_gen.stop();
        let checks = GpuChecks::push(&self.device);
        self.resize(size[0], size[1]);
        let error = checks.pop();
        if error.is_some() || self.depth.is_none() {
            self.resize(original[0], original[1]);
            return Err("could not allocate the export color and depth targets".into());
        }
        self.world_post.reset_temporal_sequence();
        self.reset_shader_pack_history();
        self.frame_export = Some(readbacks);
        Ok(())
    }

    /// Arms the copy or returns WouldBlock if all staging buffers are in flight.
    pub fn arm_export_frame(&mut self, index: u32) -> Result<()> {
        if !self.try_arm_export_frame(index)? {
            return Err(io::Error::from(io::ErrorKind::WouldBlock).into());
        }
        Ok(())
    }

    /// Try to reserve one of the three slots without blocking the control loop.
    pub fn try_arm_export_frame(&mut self, index: u32) -> Result<bool> {
        if let Some((_, Some(post))) = &mut self.pack_post
            && !post.poll_export_center_depth(&self.device)?
        {
            return Ok(false);
        }
        self.frame_export
            .as_mut()
            .ok_or("no frame export")?
            .try_arm(index)
    }

    /// Submit the terrain copies staged while the export waits on scene readiness.
    pub fn flush_frame_export_uploads(&mut self) {
        if !self.device_lost()
            && let Some(copies) = self.take_section_copies()
        {
            self.queue.submit([copies]);
        }
    }

    /// Return a slot if the attempted draw left its armed copy unsubmitted.
    pub fn disarm_export_frame(&mut self) -> bool {
        if let Some(export) = &mut self.frame_export
            && let Some((_, buffer)) = export.armed.take()
        {
            export.reserved = Some(buffer);
            true
        } else {
            false
        }
    }

    pub fn frame_export_failed(&self) -> bool {
        self.frame_export
            .as_ref()
            .and_then(|r| r.worker.as_ref())
            .is_some_and(|w| w.is_finished())
    }

    pub fn frame_export_recycler(&self) -> Option<SyncSender<Vec<u8>>> {
        self.frame_export
            .as_ref()
            .map(|export| export.recycle.clone())
    }

    /// Close capture input and let the worker deliver frames already submitted.
    pub fn begin_finish_frame_export(&mut self) {
        if let Some(readbacks) = &mut self.frame_export {
            readbacks.begin_finish();
        }
    }

    /// Restore the original frame size once the worker has finished draining.
    pub fn poll_finish_frame_export(&mut self) -> Result<bool> {
        if self.frame_export.as_ref().is_some_and(|r| !r.is_finished()) {
            return Ok(false);
        }
        self.end_frame_export()?;
        Ok(true)
    }

    /// Drain accepted frames, bounding each stalled send by the export deadline.
    pub fn end_frame_export(&mut self) -> Result<()> {
        self.close_frame_export(Readbacks::finish)
    }

    /// Cancel pending readbacks even if the receiver is still connected.
    pub fn abort_frame_export(&mut self) -> Result<()> {
        self.close_frame_export(|readbacks| {
            readbacks.abort();
            Ok(())
        })
    }

    fn close_frame_export(
        &mut self,
        close: impl FnOnce(&mut Readbacks) -> Result<()>,
    ) -> Result<()> {
        let Some(mut readbacks) = self.frame_export.take() else {
            return Ok(());
        };
        let outcome = close(&mut readbacks);
        let [width, height] = readbacks.original_size;
        self.land_frame();
        if self.device_lost() {
            self.config.width = width;
            self.config.height = height;
            return outcome;
        }
        self.resize(width, height);
        self.world_post.reset_temporal_sequence();
        self.reset_shader_pack_history();
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abort_detaches_a_worker_that_cannot_check_for_cancellation() {
        let (release, blocked) = sync_channel(0);
        let (done, aborted) = sync_channel(1);
        let state = Arc::new(AtomicU8::new(CAPTURING));
        let mut readbacks = Readbacks {
            original_size: [1, 1],
            size: [1, 1],
            available: sync_channel(0).1,
            armed: None,
            reserved: None,
            bgra: false,
            copies: Some(sync_channel(0).0),
            recycle: sync_channel(0).0,
            state: Arc::clone(&state),
            worker: Some(thread::spawn(move || {
                blocked.recv().unwrap();
                Ok(())
            })),
        };
        let game_thread = thread::spawn(move || {
            readbacks.abort();
            done.send(readbacks.copies.is_none()).unwrap();
        });
        let outcome = aborted.recv_timeout(Duration::from_secs(2));
        release.send(()).unwrap();
        game_thread.join().unwrap();
        assert!(outcome.unwrap());
        assert_eq!(state.load(Ordering::Acquire), ABORTED);
    }

    #[test]
    #[ignore = "daily"]
    fn a_connected_non_draining_receiver_never_blocks_arming_or_teardown() {
        let _gpu = super::super::test_gpu::lock();
        let Some(adapter) = super::super::test_gpu::adapter() else {
            return;
        };
        let (device, queue) = super::super::test_gpu::device(&adapter);
        let (sender, receiver) = sync_channel(0);
        let mut readbacks = Readbacks::new(
            &device,
            [16, 16],
            [16, 16],
            sender,
            FRAME_EXPORT_STALL_TIMEOUT,
        )
        .unwrap();
        for index in 0..SLOTS as u32 {
            assert!(readbacks.try_arm(index).unwrap());
            readbacks.submitted(queue.submit([])).unwrap();
        }

        let now = Instant::now();
        assert!(!readbacks.try_arm(SLOTS as u32).unwrap());
        assert!(now.elapsed() < Duration::from_secs(1));
        readbacks.begin_finish();
        assert!(!readbacks.is_finished());
        readbacks.abort();
        assert!(now.elapsed() < Duration::from_secs(5));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match receiver.try_recv() {
                Err(TryRecvError::Disconnected) => break,
                Err(TryRecvError::Empty) if Instant::now() < deadline => thread::yield_now(),
                frame => panic!(
                    "expected the aborted worker to disconnect, got {}",
                    frame.is_ok()
                ),
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn graceful_drain_times_out_on_a_connected_non_draining_receiver() {
        let _gpu = super::super::test_gpu::lock();
        let Some(adapter) = super::super::test_gpu::adapter() else {
            return;
        };
        let (device, queue) = super::super::test_gpu::device(&adapter);
        let (out, receiver) = sync_channel(0);
        let mut readbacks =
            Readbacks::new(&device, [16, 16], [16, 16], out, Duration::from_millis(50)).unwrap();
        readbacks.arm(0).unwrap();
        readbacks.submitted(queue.submit([])).unwrap();

        let (done, finished) = sync_channel(1);
        let drain = thread::spawn(move || {
            let error = readbacks.finish().unwrap_err();
            done.send(error.downcast::<io::Error>().unwrap().kind())
                .unwrap();
        });
        let outcome = finished.recv_timeout(Duration::from_secs(5));
        drop(receiver);
        drain.join().unwrap();
        assert_eq!(outcome.unwrap(), io::ErrorKind::TimedOut);
    }

    #[test]
    #[ignore = "daily"]
    fn graceful_drain_keeps_every_frame_for_a_slow_receiver() {
        let _gpu = super::super::test_gpu::lock();
        let Some(adapter) = super::super::test_gpu::adapter() else {
            return;
        };
        let (device, queue) = super::super::test_gpu::device(&adapter);
        let size = [16, 16];
        let (out, frames) = sync_channel(1);
        let mut readbacks =
            Readbacks::new(&device, size, size, out, FRAME_EXPORT_STALL_TIMEOUT).unwrap();
        for index in 0..SLOTS as u32 {
            assert!(readbacks.try_arm(index).unwrap());
            let (_, buffer) = readbacks.armed.as_ref().unwrap();
            let bytes = vec![index as u8 + 1; (padded_row(size[0]) * size[1]) as usize];
            queue.write_buffer(buffer, 0, &bytes);
            readbacks.submitted(queue.submit([])).unwrap();
        }

        let (start, started) = sync_channel(0);
        let (done, finished) = sync_channel(1);
        let drain = thread::spawn(move || {
            start.send(()).unwrap();
            done.send(readbacks.finish().map_err(|e| e.to_string()))
                .unwrap();
        });
        started.recv().unwrap();
        // Hold the consumer beyond the old one-second shutdown cutoff.
        let still_draining = finished.recv_timeout(Duration::from_millis(1500));
        let received: Vec<_> = frames.into_iter().collect();
        drain.join().unwrap();

        assert!(matches!(
            still_draining,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        finished.try_recv().unwrap().unwrap();
        assert_eq!(received.len(), SLOTS);
        for (index, frame) in received.into_iter().enumerate() {
            assert_eq!(frame.index, index as u32);
            assert_eq!(frame.size, size);
            assert_eq!(
                frame.rgb,
                vec![index as u8 + 1; (size[0] * size[1] * 3) as usize]
            );
        }
    }

    #[test]
    #[ignore = "daily"]
    fn settling_iterations_preserve_the_taa_sequence() {
        use glam::{DVec3, Mat4};
        let _gpu = super::super::test_gpu::lock();
        let Some(adapter) = super::super::test_gpu::adapter() else {
            return;
        };
        let (device, _) = super::super::test_gpu::device(&adapter);
        let (out, _frames) = sync_channel(1);
        let mut readbacks =
            Readbacks::new(&device, [16, 16], [16, 16], out, FRAME_EXPORT_STALL_TIMEOUT).unwrap();
        let mut post = super::super::world_post::Post::new(&device, TextureFormat::Rgba8Unorm);
        post.taa = true;
        let window = [16, 16];
        let mut jitter = |post: &mut super::super::world_post::Post, settling_frames| {
            post.reset_temporal_sequence();
            let mut matrices = Vec::new();
            for index in 0..3 {
                for _ in 0..settling_frames {
                    if Readbacks::draw_allowed(Some(&readbacks)) {
                        post.jitter(window, true, Mat4::IDENTITY, DVec3::ZERO);
                    }
                }
                assert!(readbacks.try_arm(index).unwrap());
                assert!(Readbacks::draw_allowed(Some(&readbacks)));
                matrices.push(post.jitter(window, true, Mat4::IDENTITY, DVec3::ZERO));
                readbacks.reserved = readbacks.armed.take().map(|(_, buffer)| buffer);
            }
            matrices
        };
        let clean = jitter(&mut post, 0);
        assert_ne!(clean[0], Mat4::IDENTITY);
        assert_ne!(clean[0], clean[1]);
        assert_eq!(jitter(&mut post, 11), clean);
    }

    #[test]
    #[ignore = "daily"]
    fn portrait_readbacks_keep_frame_order_rows_and_rgb_channels() {
        let _gpu = super::super::test_gpu::lock();
        let Some(adapter) = super::super::test_gpu::adapter() else {
            return;
        };
        let (device, queue) = super::super::test_gpu::device(&adapter);
        let [width, height] = [1080, 1920];
        assert_eq!(padded_row(width), 4352);
        let colors: [[u8; 3]; 7] = [
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [0, 255, 255],
            [255, 255, 0],
            [255, 0, 255],
            [255, 255, 255],
        ];
        for format in [TextureFormat::Bgra8Unorm, TextureFormat::Rgba8Unorm] {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let (out, frames) = sync_channel(1);
            let mut readbacks = Readbacks::new(
                &device,
                [640, 480],
                [width, height],
                out,
                FRAME_EXPORT_STALL_TIMEOUT,
            )
            .unwrap();
            let recycle = readbacks.recycle.clone();
            let consume = thread::spawn(move || {
                let mut order = Vec::new();
                for frame in frames {
                    assert_eq!(frame.size, [width, height]);
                    assert_eq!(frame.rgb.len(), (width * height * 3) as usize);
                    let expected = colors[frame.index as usize];
                    assert!(frame.rgb.as_chunks::<3>().0.iter().all(|p| *p == expected));
                    order.push(frame.index);
                    let _ = recycle.try_send(frame.rgb);
                }
                order
            });
            for (index, rgb) in colors.into_iter().enumerate() {
                let [r, g, b] = rgb.map(|c| f64::from(c) / 255.0);
                let deadline = Instant::now() + Duration::from_secs(10);
                while !readbacks.try_arm(index as u32).unwrap() {
                    assert!(
                        Instant::now() < deadline,
                        "readback ring did not release a slot"
                    );
                    thread::yield_now();
                }
                let view = texture.create_view(&Default::default());
                let mut encoder = device.create_command_encoder(&Default::default());
                drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a: 1.0 }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                }));
                readbacks.copy(&mut encoder, &texture).unwrap();
                readbacks
                    .submitted(queue.submit([encoder.finish()]))
                    .unwrap();
            }
            readbacks.finish().unwrap();
            assert_eq!(consume.join().unwrap(), [0, 1, 2, 3, 4, 5, 6]);
        }
    }
}
