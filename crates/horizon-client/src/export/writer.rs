use super::{EncoderPipe, ExportFolder, FrameSink, PngSequence};
use std::io;
use std::sync::atomic::{AtomicU32, Ordering};
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
}

impl Progress {
    pub fn frames(&self) -> u32 {
        self.frames.load(Ordering::Acquire)
    }

    pub fn error(&self) -> Option<io::Error> {
        self.failure
            .lock()
            .unwrap()
            .as_ref()
            .map(|err| io::Error::new(err.kind(), err.to_string()))
    }

    /// Stop starting queued frames on an aborted export.
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
        match sink.write(frame.index, frame.size, &frame.rgb) {
            Ok(written) => {
                bytes += written;
                progress.frames.fetch_add(1, Ordering::Release);
            }
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
