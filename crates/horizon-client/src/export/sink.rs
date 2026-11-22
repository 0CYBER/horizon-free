use super::process::EncoderChild;
use super::{EncoderConfig, ExportFolder};
use crate::gfx::FRAME_EXPORT_STALL_TIMEOUT;
use std::io::{self, BufWriter, Write as _};
use std::process::{ChildStdin, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::time::Duration;

const ENCODER_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

pub trait FrameSink: Send {
    /// Write one packed RGB8 frame. Returns bytes written to the file or pipe.
    fn write(&mut self, index: u32, size: [u32; 2], rgb: &[u8]) -> io::Result<u64>;
    fn finish(self: Box<Self>) -> io::Result<()>;
}

pub struct PngSequence {
    folder: Arc<ExportFolder>,
}

impl PngSequence {
    pub fn new(folder: Arc<ExportFolder>) -> Self {
        Self { folder }
    }
}

impl FrameSink for PngSequence {
    fn write(&mut self, index: u32, size: [u32; 2], rgb: &[u8]) -> io::Result<u64> {
        check_frame(size, rgb)?;
        let mut file = self.folder.create_new(&format!("frame_{index:06}.png"))?;
        let mut encoder = png::Encoder::new(BufWriter::new(&mut file), size[0], size[1]);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(rgb)?;
        writer.finish()?;
        Ok(file.metadata()?.len())
    }

    fn finish(self: Box<Self>) -> io::Result<()> {
        Ok(())
    }
}

pub struct EncoderPipe {
    stdin: Option<ChildStdin>,
    child: Option<EncoderChild>,
    size: [u32; 2],
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
        config.validate_output_args()?;
        config.verify_fd_output(ENCODER_PROBE_TIMEOUT)?;
        Self::spawn_verified(folder, config, size, fps, timeout)
    }

    fn spawn_verified(
        folder: &ExportFolder,
        config: &EncoderConfig,
        size: [u32; 2],
        fps: (u32, u32),
        timeout: Duration,
    ) -> io::Result<Self> {
        let video_name = format!("video.{}", config.container);
        folder.check_absent("encoder.log")?;
        folder.check_absent(&video_name)?;
        let staging = folder.stage()?;
        let mut log = staging.create_new("encoder.log")?;
        let mut output = staging.create_new(&video_name)?;
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
            .stdout(output.file.try_clone()?)
            .stderr(log.file.try_clone()?);
        let (child, stdin) = EncoderChild::spawn(command, timeout)?;
        // Keep failures confined to staging. If publication races an existing
        // destination, leave any log already published in place.
        log.publish()?;
        output.publish()?;
        Ok(Self {
            stdin: Some(stdin),
            child: Some(child),
            size,
        })
    }

    fn wait_for_exit(&mut self) -> io::Result<ExitStatus> {
        self.stdin.take();
        self.child.take().ok_or_else(closed_stdin)?.finish()
    }
}

impl FrameSink for EncoderPipe {
    fn write(&mut self, _index: u32, size: [u32; 2], rgb: &[u8]) -> io::Result<u64> {
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
        let result = write_rgb(stdin, child, rgb);
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

    fn finish(mut self: Box<Self>) -> io::Result<()> {
        let status = self.wait_for_exit()?;
        if !status.success() {
            return Err(io::Error::other(format!("encoder failed ({status})")));
        }
        Ok(())
    }
}

fn write_rgb(stdin: &mut ChildStdin, child: &EncoderChild, mut rgb: &[u8]) -> io::Result<()> {
    if !child.writing() {
        return Err(closed_stdin());
    }
    while !rgb.is_empty() {
        // Bound individual blocking writes so returning from one is evidence of progress.
        let chunk = &rgb[..rgb.len().min(64 * 1024)];
        match stdin.write(chunk) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => {
                rgb = &rgb[n..];
                if !child.writing() {
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
