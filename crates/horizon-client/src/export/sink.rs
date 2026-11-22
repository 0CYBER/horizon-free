use super::budget::{BudgetedWrite, Reservation};
use super::dir::{ExportDestination, Staging};
use super::process::EncoderChild;
use super::{ByteBudget, DirError, EncoderConfig, ExportFolder, ExportSpec, Progress};
use crate::gfx::FRAME_EXPORT_STALL_TIMEOUT;
use std::borrow::Borrow;
use std::io::{self, BufWriter, Write as _};
use std::path::Path;
use std::process::{ChildStdin, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::time::Duration;

const ENCODER_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

pub trait FrameSink: Send {
    /// Write one packed RGB8 frame. Returns bytes written to the file or pipe.
    fn write(&mut self, index: u32, size: [u32; 2], rgb: &[u8]) -> io::Result<u64>;

    /// Write a frame and record its completion for the export session.
    #[doc(hidden)]
    fn write_tracked(
        &mut self,
        index: u32,
        size: [u32; 2],
        rgb: &[u8],
        progress: &Progress,
    ) -> io::Result<u64> {
        let bytes = self.write(index, size, rgb)?;
        progress.record_frame(bytes);
        Ok(bytes)
    }

    fn finish(self: Box<Self>) -> io::Result<()>;
}

pub struct PngSequence {
    folder: Arc<ExportFolder>,
    budget: Option<Arc<ByteBudget>>,
}

impl PngSequence {
    pub fn new(folder: Arc<ExportFolder>) -> Self {
        Self {
            folder,
            budget: None,
        }
    }

    pub(super) fn with_budget(folder: Arc<ExportFolder>, budget: Arc<ByteBudget>) -> Self {
        Self {
            folder,
            budget: Some(budget),
        }
    }

    fn write_png(
        &self,
        index: u32,
        size: [u32; 2],
        rgb: &[u8],
        progress: Option<&Progress>,
    ) -> io::Result<u64> {
        check_frame(size, rgb)?;
        let name = format!("frame_{index:06}.png");
        if let Some(budget) = &self.budget {
            let raw = rgb.len() as u64;
            let mut reservation = budget.reserve(raw)?;
            let staging = self.folder.stage()?;
            let mut pending = staging.create_new(&name)?;
            encode_png(
                BudgetedWrite {
                    file: &mut pending.file,
                    prepaid: raw,
                    budget: &mut reservation,
                },
                size,
                rgb,
            )?;
            let len = pending.file.metadata()?.len();
            if let Some(progress) = progress {
                progress.publish_frame(len, || pending.publish())?;
            } else {
                pending.publish()?;
            }
            reservation.commit();
            Ok(len)
        } else {
            let mut file = self.folder.create_new(&name)?;
            encode_png(&mut file, size, rgb)?;
            let bytes = file.metadata()?.len();
            if let Some(progress) = progress {
                progress.record_frame(bytes);
            }
            Ok(bytes)
        }
    }
}

impl FrameSink for PngSequence {
    fn write(&mut self, index: u32, size: [u32; 2], rgb: &[u8]) -> io::Result<u64> {
        self.write_png(index, size, rgb, None)
    }

    fn write_tracked(
        &mut self,
        index: u32,
        size: [u32; 2],
        rgb: &[u8],
        progress: &Progress,
    ) -> io::Result<u64> {
        self.write_png(index, size, rgb, Some(progress))
    }

    fn finish(self: Box<Self>) -> io::Result<()> {
        Ok(())
    }
}

fn encode_png(file: impl io::Write, size: [u32; 2], rgb: &[u8]) -> io::Result<()> {
    let mut buffered = BufWriter::new(file);
    let mut encoder = png::Encoder::new(&mut buffered, size[0], size[1]);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgb)?;
    writer.finish()?;
    buffered.flush()
}

pub struct EncoderPipe {
    stdin: Option<ChildStdin>,
    child: Option<EncoderChild>,
    size: [u32; 2],
    budget: Option<Arc<ByteBudget>>,
}

#[derive(Debug)]
pub enum EncoderStartError {
    Destination(DirError),
    Folder(io::Error),
    Encoder(io::Error),
}

impl EncoderPipe {
    pub fn cancel_handle(&self) -> Option<Box<dyn FnOnce() + Send>> {
        self.child.as_ref().map(EncoderChild::cancel_handle)
    }

    /// The encoder must support FFmpeg's seekable `fd:` protocol on stdout.
    pub fn spawn(
        folder: &ExportFolder,
        config: &EncoderConfig,
        size: [u32; 2],
        fps: (u32, u32),
    ) -> io::Result<Self> {
        Self::spawn_with_timeout(folder, config, size, fps, FRAME_EXPORT_STALL_TIMEOUT)
    }

    fn spawn_with_timeout(
        folder: &ExportFolder,
        config: &EncoderConfig,
        size: [u32; 2],
        fps: (u32, u32),
        timeout: Duration,
    ) -> io::Result<Self> {
        Self::check_config(config)?;
        Self::spawn_verified(folder, config, size, fps, timeout)
    }

    /// Check encoder capabilities before reserving a take directory or budget.
    pub fn check_config(config: &EncoderConfig) -> io::Result<()> {
        config.validate_output_args()?;
        config.verify_fd_output(ENCODER_PROBE_TIMEOUT)
    }

    /// Start an encoder after `check_config` accepted its configuration.
    pub fn spawn_for_mod(
        game: &Path,
        spec: &ExportSpec,
        config: &EncoderConfig,
        budget: Arc<ByteBudget>,
        charge: Reservation<'_>,
    ) -> Result<(ExportFolder, Self), EncoderStartError> {
        let destination_error = EncoderStartError::Destination;
        let destination = ExportDestination::open(game, &spec.name).map_err(destination_error)?;
        destination.check_absent().map_err(destination_error)?;
        let staging = destination.stage().map_err(EncoderStartError::Folder)?;
        let (folder, mut pipe) = Self::spawn_staged(
            &staging,
            config,
            spec.size,
            spec.fps,
            FRAME_EXPORT_STALL_TIMEOUT,
            || destination.create(Some(charge)).map_err(destination_error),
        )?;
        pipe.budget = Some(budget);
        Ok((folder, pipe))
    }

    fn spawn_verified(
        folder: &ExportFolder,
        config: &EncoderConfig,
        size: [u32; 2],
        fps: (u32, u32),
        timeout: Duration,
    ) -> io::Result<Self> {
        Self::spawn_configured(folder, config, size, fps, timeout).map_err(|error| match error {
            EncoderStartError::Destination(error) => io::Error::other(error),
            EncoderStartError::Folder(error) | EncoderStartError::Encoder(error) => error,
        })
    }

    fn spawn_configured(
        folder: &ExportFolder,
        config: &EncoderConfig,
        size: [u32; 2],
        fps: (u32, u32),
        timeout: Duration,
    ) -> Result<Self, EncoderStartError> {
        let folder_error = EncoderStartError::Folder;
        let video_name = format!("video.{}", config.container);
        folder.check_absent("encoder.log").map_err(folder_error)?;
        folder.check_absent(&video_name).map_err(folder_error)?;
        let staging = folder.stage().map_err(folder_error)?;
        Self::spawn_staged(&staging, config, size, fps, timeout, || Ok(folder))
            .map(|(_, pipe)| pipe)
    }

    fn spawn_staged<T: Borrow<ExportFolder>>(
        staging: &Staging<'_>,
        config: &EncoderConfig,
        size: [u32; 2],
        fps: (u32, u32),
        timeout: Duration,
        destination: impl FnOnce() -> Result<T, EncoderStartError>,
    ) -> Result<(T, Self), EncoderStartError> {
        let folder_error = EncoderStartError::Folder;
        let video_name = format!("video.{}", config.container);
        let mut log = staging.create_new("encoder.log").map_err(folder_error)?;
        let mut output = staging.create_new(&video_name).map_err(folder_error)?;
        let muxer = match config.container {
            "mkv" => "matroska",
            other => other,
        };
        // The fd protocol uses stdout by default for output. It detects the
        // inherited regular file and allows the muxer to seek on Unix and Windows.
        let mut command = Command::new(&config.program);
        command
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-s"])
            .arg(format!("{}x{}", size[0], size[1]))
            .arg("-r")
            .arg(format!("{}/{}", fps.0, fps.1))
            .args(["-i", "-", "-an"])
            .args(&config.args)
            .args(["-f", muxer, "fd:"])
            .stdin(Stdio::piped())
            .stdout(output.file.try_clone().map_err(folder_error)?)
            .stderr(log.file.try_clone().map_err(folder_error)?);
        let (child, stdin) =
            EncoderChild::spawn(command, timeout).map_err(EncoderStartError::Encoder)?;
        let pipe = Self {
            stdin: Some(stdin),
            child: Some(child),
            size,
            budget: None,
        };
        // The take is claimed only after launch. Publish files individually so
        // Windows can keep the encoder's handles open without moving their directory.
        let folder = destination()?;
        log.publish_to(folder.borrow()).map_err(folder_error)?;
        output.publish_to(folder.borrow()).map_err(folder_error)?;
        Ok((folder, pipe))
    }

    fn wait_for_exit(&mut self) -> io::Result<ExitStatus> {
        self.stdin.take();
        self.child.take().ok_or_else(closed_stdin)?.finish()
    }

    fn write_frame(
        &mut self,
        size: [u32; 2],
        rgb: &[u8],
        progress: Option<&Progress>,
    ) -> io::Result<u64> {
        if size != self.size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "encoder frame size changed",
            ));
        }
        check_frame(size, rgb)?;
        let (stdin, child) = self
            .stdin
            .as_mut()
            .zip(self.child.as_ref())
            .ok_or_else(closed_stdin)?;
        let reservation = self
            .budget
            .as_ref()
            .map(|budget| budget.reserve(rgb.len() as u64))
            .transpose()?;
        let result = write_rgb(stdin, rgb, || child.writing(), reservation, progress);
        if child.idle() {
            match result {
                Ok(()) => return Ok(rgb.len() as u64),
                Err(err) if err.kind() != io::ErrorKind::BrokenPipe => return Err(err),
                Err(_) => {}
            }
        }
        let status = self.wait_for_exit()?;
        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            format!("encoder closed stdin ({status})"),
        ))
    }
}

impl FrameSink for EncoderPipe {
    fn write(&mut self, _index: u32, size: [u32; 2], rgb: &[u8]) -> io::Result<u64> {
        self.write_frame(size, rgb, None)
    }

    fn write_tracked(
        &mut self,
        _index: u32,
        size: [u32; 2],
        rgb: &[u8],
        progress: &Progress,
    ) -> io::Result<u64> {
        self.write_frame(size, rgb, Some(progress))
    }

    fn finish(mut self: Box<Self>) -> io::Result<()> {
        let status = self.wait_for_exit()?;
        if !status.success() {
            return Err(io::Error::other(format!("encoder failed ({status})")));
        }
        Ok(())
    }
}

fn write_rgb(
    stdin: &mut impl io::Write,
    mut rgb: &[u8],
    writing: impl FnMut() -> bool,
    reservation: Option<Reservation<'_>>,
    progress: Option<&Progress>,
) -> io::Result<()> {
    let size = rgb.len();
    let result = write_rgb_remaining(stdin, &mut rgb, writing);
    if let Some(mut reservation) = reservation {
        reservation.spend((size - rgb.len()) as u64);
    }
    if rgb.is_empty()
        && let Some(progress) = progress
    {
        progress.record_frame(size as u64);
    }
    result
}

fn write_rgb_remaining(
    stdin: &mut impl io::Write,
    rgb: &mut &[u8],
    mut writing: impl FnMut() -> bool,
) -> io::Result<()> {
    if !writing() {
        return Err(closed_stdin());
    }
    while !rgb.is_empty() {
        // Bound individual blocking writes so returning from one is evidence of progress.
        let chunk = &rgb[..rgb.len().min(64 * 1024)];
        match stdin.write(chunk) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => {
                *rgb = &rgb[n..];
                if !writing() {
                    return Err(closed_stdin());
                }
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

fn closed_stdin() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "encoder stdin is closed")
}

fn check_frame([width, height]: [u32; 2], rgb: &[u8]) -> io::Result<()> {
    let len = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(3));
    if rgb.is_empty() || len != Some(rgb.len() as u64) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frame is not packed RGB8 at the stated size",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[test]
fn a_partial_encoder_write_charges_only_the_bytes_the_pipe_accepted() {
    struct BrokenPipe(Vec<u8>);
    impl io::Write for BrokenPipe {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if !self.0.is_empty() {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            self.0.extend_from_slice(&bytes[..3]);
            Ok(3)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let budget = ByteBudget::new(12);
    let rgb = [1; 12];
    let mut pipe = BrokenPipe(Vec::new());
    let reservation = budget.reserve(12).unwrap();
    let progress = Progress::default();
    assert_eq!(
        write_rgb(&mut pipe, &rgb, || true, Some(reservation), Some(&progress))
            .unwrap_err()
            .kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(progress.snapshot(), (0, 0));
    assert_eq!(pipe.0, [1; 3]);
    let mut another_pipe = Vec::new();
    let reservation = budget.reserve(9).unwrap();
    write_rgb(
        &mut another_pipe,
        &rgb[..9],
        || true,
        Some(reservation),
        None,
    )
    .unwrap();
    assert_eq!(another_pipe, [1; 9]);
    assert!(!budget.fits(1));
}

#[cfg(test)]
#[test]
fn a_full_encoder_transfer_is_counted_when_the_pipe_then_closes() {
    let rgb = [1; 12];
    let mut pipe = Vec::new();
    let budget = ByteBudget::new(12);
    let progress = Progress::default();
    let mut health = [true, false].into_iter();
    let error = write_rgb(
        &mut pipe,
        &rgb,
        || health.next().unwrap(),
        Some(budget.reserve(12).unwrap()),
        Some(&progress),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(pipe, rgb);
    assert_eq!(progress.snapshot(), (1, 12));
    assert!(!budget.fits(1));
}

#[cfg(test)]
#[test]
fn failed_encoder_setup_rolls_back_only_its_own_files() {
    use std::fs;

    let game = tempfile::tempdir().unwrap();
    let folder = ExportFolder::create(game.path(), "take").unwrap();
    let config = EncoderConfig {
        program: game.path().join("missing-encoder.exe"),
        args: Vec::new(),
        container: "mp4",
    };
    let start =
        || EncoderPipe::spawn_verified(&folder, &config, [16, 16], (60, 1), Duration::from_secs(1));
    let log = folder.path().join("encoder.log");
    fs::write(&log, b"previous log").unwrap();

    let error = start().err().expect("existing log blocks setup");
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert!(!folder.path().join("video.mp4").exists());
    assert_eq!(fs::read(&log).unwrap(), b"previous log");

    fs::remove_file(log).unwrap();
    let error = start().err().expect("encoder program is missing");
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert_eq!(fs::read_dir(folder.path()).unwrap().count(), 0);
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::export::Writer;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::sync::mpsc::{RecvTimeoutError, channel};
    use std::thread;
    use std::time::Instant;

    const SUPPORTS_FD: &str = "printf 'Input:\n  file\nOutput:\n  fd\n'";

    fn encoder_fixture(game: &Path, probe: &str, encode: &str, options: &str) -> EncoderConfig {
        let script = game.join("test-encoder");
        fs::write(
            &script,
            format!("#!/bin/sh\nif [ \"$1\" = -protocols ]; then\n{probe}\nexit 0\nfi\n{encode}\n"),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(
            game.join("horizon-export.txt"),
            format!("encoder=test-encoder\n{options}"),
        )
        .unwrap();
        EncoderConfig::load(game, false).unwrap()
    }

    #[test]
    fn mod_encoder_feeds_only_frames_that_fit_the_budget() {
        let game = tempfile::tempdir().unwrap();
        let config = encoder_fixture(game.path(), SUPPORTS_FD, "exec cat", "");
        let spec = ExportSpec {
            name: "limited".into(),
            size: [16, 16],
            fps: (30, 1),
            frames: 2,
            format: super::super::Format::Encoder,
            hud: false,
        };
        let pixels = vec![19; 16 * 16 * 3];
        let budget = Arc::new(ByteBudget::new((1 << 20) + pixels.len() as u64));
        let charge = budget.reserve(1 << 20).unwrap();
        EncoderPipe::check_config(&config).unwrap();
        let (folder, mut output) =
            EncoderPipe::spawn_for_mod(game.path(), &spec, &config, Arc::clone(&budget), charge)
                .unwrap();
        assert_eq!(
            output.write(0, [16, 16], &pixels).unwrap(),
            pixels.len() as u64
        );
        assert!(
            output
                .write(1, [16, 16], &pixels)
                .unwrap_err()
                .to_string()
                .contains("budget")
        );
        Box::new(output).finish().unwrap();
        assert_eq!(fs::read(folder.path().join("video.mp4")).unwrap(), pixels);
        assert!(folder.path().join("encoder.log").is_file());
    }

    #[test]
    fn an_encoder_loses_a_take_name_claimed_after_preflight() {
        let game = tempfile::tempdir().unwrap();
        let config = encoder_fixture(game.path(), SUPPORTS_FD, "exec cat", "");
        let destination = ExportDestination::open(game.path(), "take").unwrap();
        destination.check_absent().unwrap();
        let staging = destination.stage().unwrap();
        let budget = ByteBudget::new(1 << 20);
        let charge = budget.reserve(1 << 20).unwrap();

        let result = EncoderPipe::spawn_staged(
            &staging,
            &config,
            [16, 16],
            (30, 1),
            Duration::from_secs(5),
            || {
                let winner = ExportFolder::create(game.path(), "take").unwrap();
                fs::write(winner.path().join("video.mp4"), b"existing video").unwrap();
                destination
                    .create(Some(charge))
                    .map_err(EncoderStartError::Destination)
            },
        );
        assert!(matches!(
            result,
            Err(EncoderStartError::Destination(DirError::Exists))
        ));
        assert!(budget.fits(1 << 20));
        drop(staging);

        let exports = game.path().join("exports");
        assert_eq!(fs::read_dir(&exports).unwrap().count(), 1);
        let winner = exports.join("take");
        assert_eq!(fs::read_dir(&winner).unwrap().count(), 1);
        assert_eq!(
            fs::read(winner.join("video.mp4")).unwrap(),
            b"existing video"
        );
    }

    #[test]
    fn a_claimed_take_stays_charged_when_output_publication_fails() {
        let game = tempfile::tempdir().unwrap();
        let config = encoder_fixture(game.path(), SUPPORTS_FD, "exec cat", "");
        let destination = ExportDestination::open(game.path(), "take").unwrap();
        let staging = destination.stage().unwrap();
        let budget = ByteBudget::new(1 << 20);
        let charge = budget.reserve(1 << 20).unwrap();

        let result = EncoderPipe::spawn_staged(
            &staging,
            &config,
            [16, 16],
            (30, 1),
            Duration::from_secs(5),
            || {
                let folder = destination
                    .create(Some(charge))
                    .map_err(EncoderStartError::Destination)?;
                folder
                    .create_new("video.mp4")
                    .unwrap()
                    .write_all(b"existing video")
                    .unwrap();
                Ok(folder)
            },
        );
        assert!(
            matches!(result, Err(EncoderStartError::Folder(err)) if err.kind() == io::ErrorKind::AlreadyExists)
        );
        drop(staging);

        let exports = game.path().join("exports");
        assert_eq!(fs::read_dir(&exports).unwrap().count(), 1);
        assert!(exports.join("take/encoder.log").is_file());
        assert_eq!(
            fs::read(exports.join("take/video.mp4")).unwrap(),
            b"existing video"
        );
        assert!(!budget.fits(1));
    }

    #[test]
    #[ignore = "daily"]
    fn encoder_early_exit_reports_status_from_write_and_finish() {
        let game = tempfile::tempdir().unwrap();
        let config = encoder_fixture(game.path(), SUPPORTS_FD, "exit 1", "");
        let folder = ExportFolder::create(game.path(), "write").unwrap();
        let size = [1024, 1024];
        let mut pipe = EncoderPipe::spawn(&folder, &config, size, (60, 1)).unwrap();
        let err = pipe.write(0, size, &vec![0; 1024 * 1024 * 3]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
        assert!(err.to_string().contains("exit status: 1"), "{err}");

        let folder = ExportFolder::create(game.path(), "finish").unwrap();
        let pipe = EncoderPipe::spawn(&folder, &config, size, (60, 1)).unwrap();
        let err = Writer::encoder(pipe).unwrap().finish().unwrap_err();
        assert!(err.to_string().contains("exit status: 1"), "{err}");
    }

    #[test]
    #[ignore = "daily"]
    fn slow_encoder_applies_backpressure_and_receives_frames_in_order() {
        let game = tempfile::tempdir().unwrap();
        // The gate keeps the pipe full until the submitter has filled two queue slots.
        let config = encoder_fixture(
            game.path(),
            SUPPORTS_FD,
            r#"printf '%s\n' "$@" >&2
while [ ! -f "${0%/*}/read-now" ]; do sleep .01; done
exec cat"#,
            "encoder_args=-metadata 'title=one take'\ncontainer=mkv\n",
        );
        let folder = ExportFolder::create(game.path(), "take").unwrap();
        let size = [1024, 1024];
        let frame_bytes = 1024 * 1024 * 3;
        let pipe = EncoderPipe::spawn(&folder, &config, size, (30_000, 1001)).unwrap();
        // Replace the ambient directory name before the child writes any output.
        let moved = folder.path().with_file_name("moved");
        fs::rename(folder.path(), &moved).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), folder.path()).unwrap();
        let writer = Writer::encoder(pipe).unwrap();
        let (progress, submitted) = channel();
        let worker = thread::spawn(move || {
            for index in 0..4 {
                writer.write(index, size, vec![index as u8; frame_bytes])?;
                let _ = progress.send(index);
            }
            writer.finish()
        });

        let first_three = [
            submitted.recv_timeout(Duration::from_secs(5)),
            submitted.recv_timeout(Duration::from_secs(5)),
            submitted.recv_timeout(Duration::from_secs(5)),
        ];
        let fourth = submitted.recv_timeout(Duration::from_millis(100));
        // Release the child before asserting so a failing test also cleans up.
        fs::write(game.path().join("read-now"), []).unwrap();
        assert_eq!(worker.join().unwrap().unwrap(), 4 * frame_bytes as u64);
        assert_eq!(first_three, [Ok(0), Ok(1), Ok(2)]);
        assert_eq!(fourth, Err(RecvTimeoutError::Timeout));
        let video = fs::read(moved.join("video.mkv")).unwrap();
        assert_eq!(video.len(), 4 * frame_bytes);
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        for (index, frame) in video.chunks_exact(frame_bytes).enumerate() {
            assert!(frame.iter().all(|pixel| *pixel == index as u8));
        }
        assert_eq!(
            fs::read_to_string(moved.join("encoder.log")).unwrap(),
            "-f\nrawvideo\n-pix_fmt\nrgb24\n-s\n1024x1024\n-r\n30000/1001\n-i\n-\n-an\n-metadata\ntitle=one take\n-f\nmatroska\nfd:\n"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_stalled_encoder_releases_submission_and_shutdown_with_a_timeout() {
        let game = tempfile::tempdir().unwrap();
        let config = encoder_fixture(game.path(), SUPPORTS_FD, "sleep 10\nexit 0", "");
        let folder = ExportFolder::create(game.path(), "take").unwrap();
        let size = [1024, 1024];
        let pipe = EncoderPipe::spawn_with_timeout(
            &folder,
            &config,
            size,
            (60, 1),
            Duration::from_millis(500),
        )
        .unwrap();
        let writer = Writer::encoder(pipe).unwrap();
        let (done, result) = channel();
        let worker = thread::spawn(move || {
            let write_error =
                (0..4).find_map(|index| writer.write(index, size, vec![0; 1024 * 1024 * 3]).err());
            let _ = done.send((write_error, writer.finish()));
        });

        let (write_error, finish) = result.recv_timeout(Duration::from_secs(5)).unwrap();
        worker.join().unwrap();
        assert_eq!(write_error.unwrap().kind(), io::ErrorKind::TimedOut);
        assert_eq!(finish.unwrap_err().kind(), io::ErrorKind::TimedOut);
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "daily"]
    fn successful_encoder_exit_kills_helpers_without_inherited_pipes() {
        let game = tempfile::tempdir().unwrap();
        let config = encoder_fixture(
            game.path(),
            SUPPORTS_FD,
            r#"sleep 10 </dev/null >/dev/null 2>&1 &
printf '%s' "$!" > "${0%/*}/helper-pid"
exit 0"#,
            "",
        );
        let folder = ExportFolder::create(game.path(), "take").unwrap();
        let pipe = EncoderPipe::spawn(&folder, &config, [16, 16], (60, 1)).unwrap();
        Box::new(pipe).finish().unwrap();

        let pid = fs::read_to_string(game.path().join("helper-pid")).unwrap();
        let stat = format!("/proc/{pid}/stat");
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            match fs::read_to_string(&stat) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => break,
                Ok(text)
                    if text
                        .rsplit_once(") ")
                        .is_some_and(|(_, state)| state.starts_with('Z')) =>
                {
                    break;
                }
                Ok(_) => assert!(Instant::now() < until, "helper {pid} is still running"),
                Err(error) => panic!("{error}"),
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    #[ignore = "daily"]
    fn missing_fd_output_leaves_the_export_directory_empty() {
        let game = tempfile::tempdir().unwrap();
        let config = encoder_fixture(
            game.path(),
            "printf 'Input:\n  fd\nOutput:\n  pipe\n'",
            "exit 1",
            "",
        );
        let folder = ExportFolder::create(game.path(), "unsupported").unwrap();
        let error = EncoderPipe::spawn(&folder, &config, [16, 16], (60, 1))
            .err()
            .expect("input-only fd support is insufficient");
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(error.to_string().contains("fd: output protocol"));
        assert_eq!(fs::read_dir(folder.path()).unwrap().count(), 0);
    }

    #[test]
    #[ignore = "daily"]
    fn protocol_probe_times_out_when_a_helper_keeps_stdout_open() {
        let game = tempfile::tempdir().unwrap();
        let config = encoder_fixture(game.path(), "sleep 10 &", "exit 1", "");
        let started = Instant::now();
        let error = config
            .verify_fd_output(Duration::from_millis(500))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("fd: output protocol"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
