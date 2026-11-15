//! Read-back screenshots waiting for the game's client hook, then PNG encoding.
use super::{Result, hdr_icc, hdr_screenshot_path};
use std::path::{Path, PathBuf};

/// CPU-side frame returned to the game before any screenshot file is written.
pub struct PendingScreenshot {
    pub path: PathBuf,
    /// F2 and world-icon requests finish asynchronously; control captures do not.
    pub background: bool,
    pub(super) image: mod_api::ScreenshotImage,
    pub(super) hdr: Option<HdrScreenshot>,
}

pub(super) struct HdrScreenshot {
    pub size: [u32; 2],
    pub pixels: Vec<u8>,
}

/// Files successfully written for a requested screenshot.
pub struct SavedScreenshot {
    pub size: [u32; 2],
    pub hdr_path: Option<PathBuf>,
}

type SaveOutcome = (PathBuf, std::result::Result<[u32; 2], String>);

/// A screenshot being encoded after the owning-thread hook has returned.
pub struct ScreenshotSave {
    pub original: PathBuf,
    receiver: std::sync::mpsc::Receiver<SaveOutcome>,
}
impl ScreenshotSave {
    fn spawn(original: PathBuf, write: impl FnOnce() -> SaveOutcome + Send + 'static) -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        let completed = sender.clone();
        let spawned = std::thread::Builder::new()
            .name("Screenshot writer".into())
            .spawn(move || {
                let _ = completed.send(write());
            });
        if let Err(error) = spawned {
            let _ = sender.send((original.clone(), Err(error.to_string())));
        }
        Self { original, receiver }
    }

    /// Poll without waiting for the PNG encoder or disk.
    pub fn try_complete(&self) -> Option<SaveOutcome> {
        match self.receiver.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some((
                self.original.clone(),
                Err("screenshot writer stopped".into()),
            )),
        }
    }
}
impl PendingScreenshot {
    /// Encode the approved pixels on a worker. A mod-supplied destination is
    /// validated by the same write boundary as a synchronous save.
    pub fn save_background(mut self, destination: Option<(PathBuf, String)>) -> ScreenshotSave {
        ScreenshotSave::spawn(self.path.clone(), move || {
            let result = match destination {
                Some((root, name)) => self.save_in(&root, &name),
                None => self.save(),
            }
            .map_err(|e| e.to_string());
            (self.path, result)
        })
    }
}

impl PendingScreenshot {
    /// The frame the screenshot hook may read and change (`ScreenshotEvent.getImage`).
    pub fn image_mut(&mut self) -> &mut mod_api::ScreenshotImage {
        &mut self.image
    }

    /// `width * height * 4` RGBA8 bytes, or a refusal that writes no file.
    fn rgba(&self) -> Result<&mod_api::ScreenshotImage> {
        let image = &self.image;
        let bytes = (image.width as usize)
            .checked_mul(image.height as usize)
            .and_then(|pixels| pixels.checked_mul(4));
        if image.width == 0 || image.height == 0 || bytes != Some(image.rgba.len()) {
            return Err("screenshot image must be width by height RGBA8 pixels".into());
        }
        Ok(image)
    }

    /// Save an agent capture or world icon to its requested destination.
    pub fn save(&self) -> Result<[u32; 2]> {
        self.save_with_details().map(|saved| saved.size)
    }

    /// Save a capture and report the HDR sibling only when its write succeeds.
    pub fn save_with_details(&self) -> Result<SavedScreenshot> {
        let image = self.rgba()?;
        let mut hdr_path = None;
        if let Some(hdr) = &self.hdr {
            let path = hdr_screenshot_path(&self.path);
            match hdr.save(&path) {
                Ok(()) => {
                    log::info!("Saved HDR screenshot to {}", path.display());
                    hdr_path = Some(path);
                }
                Err(e) => log::warn!("Horizon renderer: HDR screenshot {}: {e}", path.display()),
            }
        }
        let mut encoder = png::Encoder::new(
            std::io::BufWriter::new(std::fs::File::create(&self.path)?),
            image.width,
            image.height,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&image.rgba)?;
        Ok(SavedScreenshot {
            size: [image.width, image.height],
            hdr_path,
        })
    }

    /// Validate a mod's relative file name at the write boundary, then save
    /// both copies inside `root`. No destination may traverse a symbolic link.
    pub fn save_in(&mut self, root: &Path, file_name: &str) -> Result<[u32; 2]> {
        let path = screenshot_destination(root, file_name)?;
        if self.hdr.is_some() {
            // A `Path` puts the sibling's name after the OS's separator; the name stays portable.
            let hdr_name = hdr_screenshot_path(Path::new(file_name));
            let hdr_name = hdr_name
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            screenshot_destination(root, &hdr_name)?;
        }
        self.rgba()?;
        std::fs::create_dir_all(path.parent().expect("destination has a parent"))?;
        self.path = path;
        self.save()
    }
}

/// A relative, portable path. Refuse Windows separators/prefixes on every OS,
/// not just on Windows, and inspect every component including the output file.
fn screenshot_destination(root: &Path, file_name: &str) -> Result<PathBuf> {
    if file_name.is_empty()
        || file_name.contains(['\\', ':', '\0'])
        || file_name.split('/').any(|part| {
            let stem = part
                .split('.')
                .next()
                .unwrap_or_default()
                .to_ascii_uppercase();
            part.is_empty()
                || part.ends_with(['.', ' '])
                || matches!(
                    stem.as_str(),
                    "CON"
                        | "PRN"
                        | "AUX"
                        | "NUL"
                        | "COM1"
                        | "COM2"
                        | "COM3"
                        | "COM4"
                        | "COM5"
                        | "COM6"
                        | "COM7"
                        | "COM8"
                        | "COM9"
                        | "LPT1"
                        | "LPT2"
                        | "LPT3"
                        | "LPT4"
                        | "LPT5"
                        | "LPT6"
                        | "LPT7"
                        | "LPT8"
                        | "LPT9"
                )
        })
    {
        return Err("screenshot file name must be a relative path inside screenshots".into());
    }
    let mut path = root.to_path_buf();
    for part in file_name.split('/') {
        path.push(part);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err("screenshot destination must not traverse a symbolic link".into());
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(root.join(file_name))
}

impl HdrScreenshot {
    /// HDR10 as a 16-bit PNG, retaining BT.2020/PQ, cLLI and the ICC profile.
    fn save(&self, path: &Path) -> Result<()> {
        let mut info = png::Info::with_size(self.size[0], self.size[1]);
        info.color_type = png::ColorType::Rgb;
        info.bit_depth = png::BitDepth::Sixteen;
        info.icc_profile = Some(hdr_icc::profile().into());
        let encoder =
            png::Encoder::with_info(std::io::BufWriter::new(std::fs::File::create(path)?), info)?;
        let mut writer = encoder.write_header()?;
        writer.write_chunk(png::chunk::cICP, &[9, 16, 0, 1])?;
        writer.write_chunk(
            png::chunk::cLLI,
            &hdr_icc::content_light_levels(&self.pixels),
        )?;
        writer.write_image_data(&self.pixels)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(path: PathBuf, hdr: bool) -> PendingScreenshot {
        PendingScreenshot {
            path,
            background: true,
            image: mod_api::ScreenshotImage::new(1, 1, vec![12, 34, 56, 255]),
            hdr: hdr.then(|| HdrScreenshot {
                size: [1, 1],
                pixels: vec![0, 0, 255, 255, 128, 0],
            }),
        }
    }

    #[test]
    fn background_encoding_does_not_wait_on_the_frame_thread() {
        let (entered, started) = std::sync::mpsc::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let (queued, returned) = std::sync::mpsc::channel();
        let frame_thread = std::thread::spawn(move || {
            let owner = std::thread::current().id();
            let path = PathBuf::from("shot.png");
            let save = ScreenshotSave::spawn(path.clone(), move || {
                entered.send(std::thread::current().id()).unwrap();
                blocked.recv().unwrap();
                (path, Ok([1, 1]))
            });
            assert!(queued.send((owner, save)).is_ok());
        });
        let (owner, save) = returned
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        frame_thread.join().unwrap();
        assert_ne!(
            started
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap(),
            owner
        );
        assert!(save.try_complete().is_none());
        release.send(()).unwrap();
        assert_eq!(
            save.receiver
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap()
                .1,
            Ok([1, 1])
        );
    }

    #[test]
    fn background_save_keeps_hook_edits_and_renames_both_copies() {
        let root = tempfile::tempdir().unwrap();
        let mut pending = frame(root.path().join("unused.png"), true);
        pending.image_mut().rgba = vec![9, 8, 7, 255];
        let save = pending.save_background(Some((root.path().to_owned(), "changed.png".into())));
        let (path, result) = save
            .receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        assert_eq!(result, Ok([1, 1]));
        assert_eq!(path, root.path().join("changed.png"));
        assert!(root.path().join("changed-hdr.png").exists());
        assert!(!root.path().join("unused.png").exists());
        let mut reader =
            png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()))
                .read_info()
                .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!(&pixels[..info.buffer_size()], &[9, 8, 7, 255]);
    }

    #[test]
    fn pending_screenshot_writes_only_on_save_and_renames_both_copies() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("original.png");
        let mut pending = frame(original.clone(), true);
        assert!(!original.exists());
        assert_eq!(
            pending.save_in(root.path(), "album/changed.png").unwrap(),
            [1, 1]
        );
        assert!(!original.exists());
        let file = std::fs::File::open(root.path().join("album/changed.png")).unwrap();
        let mut reader = png::Decoder::new(std::io::BufReader::new(file))
            .read_info()
            .unwrap();
        let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut bytes).unwrap();
        assert_eq!(&bytes[..info.buffer_size()], &[12, 34, 56, 255]);
        let file = std::fs::File::open(root.path().join("album/changed-hdr.png")).unwrap();
        let reader = png::Decoder::new(std::io::BufReader::new(file))
            .read_info()
            .unwrap();
        assert_eq!(reader.info().bit_depth, png::BitDepth::Sixteen);
        assert!(reader.info().icc_profile.is_some());
    }

    #[test]
    fn pending_screenshot_writes_pixels_a_mod_changed() {
        let root = tempfile::tempdir().unwrap();
        let mut pending = frame(root.path().join("unused.png"), false);
        pending.image_mut().rgba = vec![9, 8, 7, 255];
        assert_eq!(pending.save_in(root.path(), "changed.png").unwrap(), [1, 1]);
        let file = std::fs::File::open(root.path().join("changed.png")).unwrap();
        let mut reader = png::Decoder::new(std::io::BufReader::new(file))
            .read_info()
            .unwrap();
        let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut bytes).unwrap();
        assert_eq!(&bytes[..info.buffer_size()], &[9, 8, 7, 255]);
    }

    #[test]
    fn pending_screenshot_refuses_a_short_image_without_writing() {
        let root = tempfile::tempdir().unwrap();
        let mut pending = frame(root.path().join("unused.png"), true);
        pending.image_mut().rgba.pop();
        assert!(pending.save_in(root.path(), "bad.png").is_err());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn pending_screenshot_refuses_zero_dimensions_without_touching_destinations() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("existing.png");
        let hdr_path = root.path().join("existing-hdr.png");
        std::fs::write(&path, b"original SDR").unwrap();
        std::fs::write(&hdr_path, b"original HDR").unwrap();
        for (width, height) in [(0, 1), (1, 0), (0, 0)] {
            let mut pending = frame(path.clone(), true);
            *pending.image_mut() = mod_api::ScreenshotImage::new(width, height, Vec::new());
            assert!(pending.save_in(root.path(), "existing.png").is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"original SDR");
            assert_eq!(std::fs::read(&hdr_path).unwrap(), b"original HDR");
            assert!(pending.save_in(root.path(), "new/invalid.png").is_err());
            assert!(!root.path().join("new").exists());
        }
    }

    #[test]
    fn pending_screenshot_noop_saves_the_requested_name() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("normal.png");
        let pending = frame(path.clone(), false);
        assert_eq!(pending.save().unwrap(), [1, 1]);
        assert!(path.exists());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn pending_screenshot_refuses_paths_outside_the_folder() {
        let root = tempfile::tempdir().unwrap();
        let mut pending = frame(root.path().join("unused.png"), true);
        for name in [
            "",
            "/absolute.png",
            "../escape.png",
            "a/../escape.png",
            "./dot.png",
            "a//b.png",
            "a/",
            "C:drive.png",
            "a\\escape.png",
            "nul\0.png",
            ".. /escape.png",
            "album./image.png",
            "NUL.png",
            "com1.png",
            "LPT9/log.png",
        ] {
            assert!(pending.save_in(root.path(), name).is_err(), "{name:?}");
        }
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn pending_screenshot_refuses_symlinks_including_the_hdr_sibling() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.path().join("album")).unwrap();
        symlink(outside.path().join("sdr.png"), root.path().join("sdr.png")).unwrap();
        symlink(
            outside.path().join("hdr.png"),
            root.path().join("image-hdr.png"),
        )
        .unwrap();
        let mut pending = frame(root.path().join("unused.png"), true);
        for name in ["album/escape.png", "sdr.png", "image.png"] {
            assert!(pending.save_in(root.path(), name).is_err());
        }
        assert!(!root.path().join("image.png").exists());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }
}
