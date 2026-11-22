use super::{ByteBudget, EncoderPipe, ExportFolder, FrameSink, PngSequence};
use std::io;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

struct Frame {
    index: u32,
    size: [u32; 2],
    rgb: Vec<u8>,
    recycle: Option<SyncSender<Vec<u8>>>,
}

#[derive(Default)]
pub struct Progress {
    failure: Mutex<Option<io::Error>>,
    frames: AtomicU32,
    bytes: AtomicU64,
    publication: Mutex<Publication>,
}

#[derive(Default)]
struct Publication {
    fenced: bool,
    in_flight_publishes: usize,
}

impl Progress {
    pub fn frames(&self) -> u32 {
        self.frames.load(Ordering::Acquire)
    }

    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Acquire)
    }

    /// Read the counters together.
    pub fn snapshot(&self) -> (u32, u64) {
        let _publication = self.publication.lock().unwrap();
        (self.frames(), self.bytes())
    }

    /// Freeze the confirmed counters and report whether a rename was still in flight.
    /// An admitted rename can still complete after this returns.
    pub fn freeze_and_snapshot(&self) -> (u32, u64, bool) {
        let mut publication = self.publication.lock().unwrap();
        publication.fenced = true;
        (
            self.frames(),
            self.bytes(),
            publication.in_flight_publishes != 0,
        )
    }

    /// The gate protects admission and accounting. A worker must release it before
    /// touching the filesystem so teardown can still take a terminal snapshot.
    pub(super) fn publish_frame(
        &self,
        bytes: u64,
        publish: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<u64> {
        {
            let mut publication = self.publication.lock().unwrap();
            if let Some(error) = self.error() {
                return Err(error);
            }
            if publication.fenced {
                return Err(io::ErrorKind::Interrupted.into());
            }
            publication.in_flight_publishes += 1;
        }

        let result = publish();
        let mut publication = self.publication.lock().unwrap();
        publication.in_flight_publishes -= 1;
        result?;
        if !publication.fenced {
            self.add_frame(bytes);
        }
        Ok(bytes)
    }

    pub(super) fn record_frame(&self, bytes: u64) {
        let publication = self.publication.lock().unwrap();
        if !publication.fenced {
            self.add_frame(bytes);
        }
    }

    fn add_frame(&self, bytes: u64) {
        self.bytes.fetch_add(bytes, Ordering::Release);
        self.frames.fetch_add(1, Ordering::Release);
    }

    pub fn error(&self) -> Option<io::Error> {
        self.failure
            .lock()
            .unwrap()
            .as_ref()
            .map(|err| io::Error::new(err.kind(), err.to_string()))
    }

    /// Refuse new frames and staged publications on an aborted export.
    pub fn cancel(&self) {
        self.fail(io::ErrorKind::Interrupted.into());
    }

    fn fail(&self, error: io::Error) {
        self.failure.lock().unwrap().get_or_insert(error);
    }
}

/// A bounded queue of owned RGB frames. The encoder uses one worker to preserve submission
/// order. PNG file names carry their index, so different workers can encode them in parallel.
pub struct Writer {
    sender: Option<SyncSender<Frame>>,
    workers: Vec<JoinHandle<u64>>,
    progress: Arc<Progress>,
}

impl Writer {
    pub fn png(folder: Arc<ExportFolder>, workers: usize) -> io::Result<Self> {
        Self::start(
            (0..workers.clamp(1, 4))
                .map(|_| Box::new(PngSequence::new(Arc::clone(&folder))) as Box<dyn FrameSink>),
        )
    }

    pub fn png_with_budget(
        folder: Arc<ExportFolder>,
        workers: usize,
        budget: &Arc<ByteBudget>,
    ) -> io::Result<Self> {
        Self::start((0..workers.clamp(1, 4)).map(|_| {
            Box::new(PngSequence::with_budget(
                Arc::clone(&folder),
                Arc::clone(budget),
            )) as Box<dyn FrameSink>
        }))
    }

    pub fn encoder(pipe: EncoderPipe) -> io::Result<Self> {
        Self::start([Box::new(pipe) as Box<dyn FrameSink>])
    }

    fn start(sinks: impl IntoIterator<Item = Box<dyn FrameSink>>) -> io::Result<Self> {
        let (sender, receiver) = sync_channel(2);
        let receiver = Arc::new(Mutex::new(receiver));
        let progress = Arc::new(Progress::default());
        let mut writer = Self {
            sender: Some(sender),
            workers: Vec::new(),
            progress,
        };
        for sink in sinks {
            let receiver = Arc::clone(&receiver);
            let progress = writer.progress();
            writer.workers.push(
                thread::Builder::new()
                    .name("export-writer".into())
                    .spawn(move || write_frames(sink, &receiver, &progress))?,
            );
        }
        Ok(writer)
    }

    /// Queue a frame, waiting for capacity if both queued slots are occupied.
    pub fn write(&self, index: u32, size: [u32; 2], rgb: Vec<u8>) -> io::Result<()> {
        self.queue(Frame {
            index,
            size,
            rgb,
            recycle: None,
        })
    }

    pub fn write_recycled(
        &self,
        index: u32,
        size: [u32; 2],
        rgb: Vec<u8>,
        recycle: &SyncSender<Vec<u8>>,
    ) -> io::Result<()> {
        self.queue(Frame {
            index,
            size,
            rgb,
            recycle: Some(recycle.clone()),
        })
    }

    pub fn progress(&self) -> Arc<Progress> {
        Arc::clone(&self.progress)
    }

    fn queue(&self, frame: Frame) -> io::Result<()> {
        if let Some(err) = self.progress.error() {
            return Err(err);
        }
        let disconnected = self
            .sender
            .as_ref()
            .expect("active export writer")
            .send(frame)
            .is_err();
        match self.progress.error() {
            Some(err) => Err(err),
            None if disconnected => Err(io::Error::other("export writer disconnected")),
            None => Ok(()),
        }
    }

    /// Drain submitted frames, finalize all sinks and return their total bytes written.
    pub fn finish(mut self) -> io::Result<u64> {
        self.drain()
    }

    fn drain(&mut self) -> io::Result<u64> {
        self.sender.take();
        let mut bytes = 0;
        for worker in self.workers.drain(..) {
            match worker.join() {
                Ok(written) => bytes += written,
                Err(_) => self
                    .progress
                    .fail(io::Error::other("export writer panicked")),
            }
        }
        match self.progress.error() {
            Some(err) => Err(err),
            None => Ok(bytes),
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        let _ = self.drain();
    }
}

fn write_frames(
    mut sink: Box<dyn FrameSink>,
    receiver: &Mutex<Receiver<Frame>>,
    progress: &Progress,
) -> u64 {
    let mut bytes = 0;
    loop {
        let frame = receiver.lock().unwrap().recv();
        if progress.error().is_some() {
            break;
        }
        let Ok(frame) = frame else {
            if let Err(err) = sink.finish() {
                progress.fail(err);
            }
            break;
        };
        match sink.write_tracked(frame.index, frame.size, &frame.rgb, progress) {
            Ok(written) => bytes += written,
            Err(err) => {
                progress.fail(err);
                break;
            }
        }
        if let Some(recycle) = frame.recycle {
            let _ = recycle.try_send(frame.rgb);
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::BufReader;

    #[test]
    fn an_oversized_png_is_not_published_and_control_output_ignores_the_mod_budget() {
        let game = tempfile::tempdir().unwrap();
        let mod_folder = Arc::new(ExportFolder::create(game.path(), "limited").unwrap());
        let ctl_folder = Arc::new(ExportFolder::create(game.path(), "control").unwrap());
        // A 2x2 RGB frame fits by its raw size but the PNG header alone exceeds it.
        let budget = Arc::new(ByteBudget::new(12));
        let mod_writer = Writer::png_with_budget(Arc::clone(&mod_folder), 2, &budget).unwrap();
        let progress = mod_writer.progress();
        let _ = mod_writer.write(0, [2, 2], vec![0; 12]);
        assert!(
            mod_writer
                .finish()
                .unwrap_err()
                .to_string()
                .contains("budget")
        );
        assert_eq!((progress.frames(), progress.bytes()), (0, 0));
        assert_eq!(std::fs::read_dir(mod_folder.path()).unwrap().count(), 0);

        let ctl_writer = Writer::png(Arc::clone(&ctl_folder), 1).unwrap();
        ctl_writer.write(0, [2, 2], vec![0; 12]).unwrap();
        let bytes = ctl_writer.finish().unwrap();
        assert_eq!(
            std::fs::metadata(ctl_folder.path().join("frame_000000.png"))
                .unwrap()
                .len(),
            bytes
        );
        assert!(bytes > 12);
    }

    #[test]
    fn a_staged_png_cannot_publish_after_the_terminal_snapshot() {
        use std::io::Write as _;

        let game = tempfile::tempdir().unwrap();
        let folder = Arc::new(ExportFolder::create(game.path(), "take").unwrap());
        let budget = Arc::new(ByteBudget::new(1024));
        let progress = Arc::new(Progress::default());
        let mut sink = PngSequence::with_budget(Arc::clone(&folder), Arc::clone(&budget));
        let rgb = [0; 12];
        let first = sink.write_tracked(0, [2, 2], &rgb, &progress).unwrap();
        let (ready, staged) = sync_channel(0);
        let (release, hold) = sync_channel(0);
        let worker = thread::scope(|scope| {
            let (folder, progress) = (&folder, &progress);
            let worker = scope.spawn(move || {
                let staging = folder.stage().unwrap();
                let mut pending = staging.create_new("frame_000001.png").unwrap();
                let file = std::fs::read(folder.path().join("frame_000000.png")).unwrap();
                pending.file.write_all(&file).unwrap();
                ready.send(()).unwrap();
                hold.recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                progress.publish_frame(file.len() as u64, || pending.publish())
            });
            staged
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();

            // Another worker reports a budget error while frame one is staged.
            progress.fail(io::Error::other("budget exhausted"));
            progress.cancel();
            let snapshot = progress.snapshot();
            release.send(()).unwrap();
            (snapshot, worker.join().unwrap())
        });
        assert_eq!(worker.0, (1, first));
        assert_eq!(worker.1.unwrap_err().to_string(), "budget exhausted");
        assert_eq!(progress.snapshot(), worker.0);
        assert!(!folder.path().join("frame_000001.png").exists());
    }

    #[test]
    fn freezing_progress_fences_new_publications_without_cancellation() {
        let progress = Progress::default();
        assert_eq!(progress.freeze_and_snapshot(), (0, 0, false));
        let error = progress
            .publish_frame(1, || panic!("a rename started after the snapshot"))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn a_stalled_png_rename_does_not_block_the_terminal_snapshot() {
        use std::io::Write as _;
        use std::time::Duration;

        let game = tempfile::tempdir().unwrap();
        let folder = Arc::new(ExportFolder::create(game.path(), "take").unwrap());
        let budget = Arc::new(ByteBudget::new(1024));
        let progress = Progress::default();
        let mut sink = PngSequence::with_budget(Arc::clone(&folder), budget);
        let first = sink.write_tracked(0, [2, 2], &[0; 12], &progress).unwrap();
        let (renaming, entered) = sync_channel(0);
        let (release, resume) = sync_channel(1);
        let (report, snapshot) = sync_channel(1);

        let (frozen, rejected) = thread::scope(|scope| {
            let (progress, folder) = (&progress, &folder);
            let worker = scope.spawn(move || {
                let staging = folder.stage().unwrap();
                let mut pending = staging.create_new("frame_000001.png").unwrap();
                let image = std::fs::read(folder.path().join("frame_000000.png")).unwrap();
                pending.file.write_all(&image).unwrap();
                progress.publish_frame(image.len() as u64, || {
                    renaming.send(()).unwrap();
                    resume.recv_timeout(Duration::from_secs(5)).unwrap();
                    pending.publish()
                })
            });
            entered.recv_timeout(Duration::from_secs(2)).unwrap();
            scope.spawn(move || {
                progress.cancel();
                let frozen = progress.freeze_and_snapshot();
                let rejected = progress
                    .publish_frame(1, || panic!("a later rename entered the fenced gate"))
                    .unwrap_err();
                report.send((frozen, rejected.kind())).unwrap();
            });

            let result = snapshot.recv_timeout(Duration::from_secs(2));
            // Unblock the worker even when the regression is present.
            release.send(()).unwrap();
            assert_eq!(worker.join().unwrap().unwrap(), first);
            result.expect("abort and snapshot should finish while the rename remains stalled")
        });

        assert_eq!(frozen, (1, first, true));
        assert_eq!(rejected, io::ErrorKind::Interrupted);
        assert!(folder.path().join("frame_000001.png").is_file());
        assert_eq!(progress.snapshot(), (1, first));
    }

    #[test]
    fn a_png_that_cannot_publish_refunds_its_reservation() {
        let game = tempfile::tempdir().unwrap();
        let folder = Arc::new(ExportFolder::create(game.path(), "take").unwrap());
        let original = folder.create_new("frame_000000.png").unwrap();
        let budget = Arc::new(ByteBudget::new(1024));
        let mut sink = PngSequence::with_budget(Arc::clone(&folder), Arc::clone(&budget));
        let progress = Progress::default();

        let error = sink
            .write_tracked(0, [2, 2], &[0; 12], &progress)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(original.metadata().unwrap().len(), 0);
        assert_eq!(progress.snapshot(), (0, 0));
        assert!(budget.fits(1024));

        let bytes = sink.write_tracked(1, [2, 2], &[0; 12], &progress).unwrap();
        assert!(bytes > 12);
        assert!(budget.fits(1024 - bytes));
        assert!(!budget.fits(1025 - bytes));
    }

    #[test]
    fn parallel_pngs_round_trip_and_report_file_bytes() {
        let game = tempfile::tempdir().unwrap();
        let folder = Arc::new(ExportFolder::create(game.path(), "take").unwrap());
        let writer = Writer::png(Arc::clone(&folder), 4).unwrap();
        let pixels = [1_u8, 3, 5, 100, 20, 7, 255, 29, 0, 0, 2, 200];
        for index in 0..8 {
            writer
                .write(
                    index,
                    [2, 2],
                    pixels.map(|p| p.wrapping_add(index as u8)).to_vec(),
                )
                .unwrap();
        }
        let total = writer.finish().unwrap();
        let mut bytes = 0;
        for index in 0..8 {
            let file = File::open(folder.path().join(format!("frame_{index:06}.png"))).unwrap();
            bytes += file.metadata().unwrap().len();
            let mut decoder = png::Decoder::new(BufReader::new(file)).read_info().unwrap();
            let mut decoded = vec![0; decoder.output_buffer_size().unwrap()];
            let info = decoder.next_frame(&mut decoded).unwrap();
            assert_eq!((info.width, info.height), (2, 2));
            assert_eq!(
                (info.color_type, info.bit_depth),
                (png::ColorType::Rgb, png::BitDepth::Eight)
            );
            assert_eq!(decoded, pixels.map(|p| p.wrapping_add(index as u8)));
        }
        assert_eq!(total, bytes);
    }

    #[test]
    fn writer_reports_sink_write_failure_when_joined() {
        let game = tempfile::tempdir().unwrap();
        let folder = Arc::new(ExportFolder::create(game.path(), "take").unwrap());
        folder.create_new("frame_000000.png").unwrap();
        let writer = Writer::png(folder, 4).unwrap();
        let _ = writer.write(0, [1, 1], vec![1, 2, 3]);
        assert_eq!(
            writer.finish().unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
    }
}
