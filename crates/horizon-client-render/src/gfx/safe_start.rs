//! What a start that lost its graphics device leaves for the next one (#19206). The window's
//! device loss writes [`MARKER`] beside `horizon-graphics.txt` (`Game::recover_device`); a
//! recovery that keeps presenting frames for [`HEALTHY_AFTER_RECOVERY`], or a clean exit, removes
//! it again. The next window client's start ([`take`]) turns a loss with HDR output on into a hold
//! kept in that file (`Graphics::safe_start`, key `safeStart`): HDR output stays off on that
//! driver until an HDR mode is chosen or the driver changes. A loss with HDR output off only says
//! so in the log. Offscreen renderers, split-screen guests, dedicated servers and scripted starts
//! never write the marker or take it.
use crate::graphics::{FILE, Graphics};
use std::time::{Duration, Instant};

/// Written beside [`FILE`] when the window's graphics device is lost.
pub const MARKER: &str = "horizon-device-lost";
/// How long a recovered device keeps presenting frames before its loss is forgiven: the same
/// minute in which `Game::recover_device` gives up on a second loss.
pub const HEALTHY_AFTER_RECOVERY: Duration = Duration::from_secs(60);

/// The window's graphics device was lost.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loss {
    /// The window's graphics driver (`output::window_driver`).
    pub driver: String,
    /// HDR output was on.
    pub hdr: bool,
}

/// The marker's text for a loss on a window whose driver is `driver`; `None` offscreen, which
/// never writes one.
pub fn loss_text(driver: Option<&str>, hdr: bool) -> Option<String> {
    Some(format!("driver:{}\nhdr:{hdr}\n", driver?))
}

/// What a marker's text says; `None` for one it can't read.
pub fn parse_marker(text: &str) -> Option<Loss> {
    let field = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix(':'))
    };
    Some(Loss {
        driver: field("driver")?.trim().to_owned(),
        hdr: field("hdr")?.trim() == "true",
    })
}

/// Takes the [`MARKER`] in `dir` into the settings file there, for a window client's start, and
/// returns the driver HDR output is held back on: a loss with HDR output on holds it there for
/// this start even when the file can't be saved, as `hdrFailedStart` does. The marker goes only
/// once the hold is saved, and stays when the settings file can't be read, so a later start
/// tries again.
pub fn take(dir: &std::path::Path) -> Option<String> {
    let marker = dir.join(MARKER);
    let text = std::fs::read_to_string(&marker).ok()?;
    let Some(loss) = parse_marker(&text) else {
        let _ = std::fs::remove_file(&marker);
        return None;
    };
    if !loss.hdr {
        log::warn!(
            "Horizon renderer: the last start lost the graphics device with HDR output off, so HDR output was not the cause"
        );
        let _ = std::fs::remove_file(&marker);
        return None;
    }
    let file = dir.join(FILE);
    let settings = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            log::warn!(
                "Horizon renderer: the last start lost the graphics device with HDR output on, but {FILE} can't be read ({e}); HDR output is not held back"
            );
            return None;
        }
    };
    log::warn!(
        "Horizon renderer: the last start lost the graphics device with HDR output on: HDR output is held back on that graphics driver ({})",
        loss.driver
    );
    let mut g = Graphics::parse(&settings);
    g.safe_start = Some(loss.driver.clone());
    if std::fs::write(&file, g.serialize()).is_ok() {
        let _ = std::fs::remove_file(&marker);
    }
    Some(loss.driver)
}

impl Graphics {
    /// Whether the device-loss hold keeps HDR output back on a window whose driver is `driver`.
    pub(crate) fn safe_start_holds_hdr(&self, driver: &str) -> bool {
        self.safe_start.as_deref() == Some(driver)
    }

    /// The window's renderer runs on `driver`: a device-loss hold on another driver (an
    /// update, another GPU) is dropped, so HDR output is tried again. True when it changed.
    pub(crate) fn settle_safe_start(&mut self, driver: &str) -> bool {
        let other = self
            .safe_start
            .as_deref()
            .is_some_and(|held| held != driver);
        if other {
            self.safe_start = None;
        }
        other
    }
}

/// Removes the [`MARKER`] beside `options` (the player's `options.txt`).
pub fn clear(options: &std::path::Path) {
    let _ = std::fs::remove_file(options.with_file_name(MARKER));
}

/// Whether this run wrote the [`MARKER`], and when it may go as frames are presented:
/// [`HEALTHY_AFTER_RECOVERY`] after the recovery. A run never removes another one's.
#[derive(Clone, Copy, Debug, Default)]
pub struct Proof {
    wrote: bool,
    healthy_at: Option<Instant>,
}
impl Proof {
    /// This run wrote the marker of a device loss.
    pub fn lost(&mut self) {
        *self = Self {
            wrote: true,
            healthy_at: None,
        };
    }
    pub fn recovered(&mut self, now: Instant) {
        self.healthy_at = Some(now + HEALTHY_AFTER_RECOVERY);
    }
    /// A frame was presented at `now`: true, once, when this run's marker can go.
    pub fn presented(&mut self, now: Instant) -> bool {
        let due = self.wrote && self.healthy_at.is_some_and(|at| now >= at);
        if due {
            *self = Self::default();
        }
        due
    }
    /// The run ends cleanly: true when it has a marker of its own to remove.
    pub fn exit(&mut self) -> bool {
        std::mem::take(self).wrote
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NV: &str = "Dx12 10de:25a9 NVIDIA GeForce RTX 2050; ; 32.0.16.2124; os";
    const NV_NEW: &str = "Dx12 10de:25a9 NVIDIA GeForce RTX 2050; ; 32.0.16.2200; os";

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("horizon-safe-start-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Offscreen writes no marker; a window's reads back.
    #[test]
    fn offscreen_writes_no_marker_and_a_window_s_reads_back() {
        assert_eq!(loss_text(None, true), None);
        let text = loss_text(Some(NV), true).unwrap();
        let loss = Loss {
            driver: NV.into(),
            hdr: true,
        };
        assert_eq!(parse_marker(&text), Some(loss));
        assert_eq!(parse_marker("driver:x\n"), None);
    }

    /// A loss with HDR output on is saved as a hold on its driver, keeping the file's other
    /// lines, and the marker goes; with HDR output off nothing is held.
    #[test]
    fn a_loss_with_hdr_on_is_held_once_saved() {
        let dir = scratch("take");
        let marker = dir.join(MARKER);
        std::fs::write(&marker, loss_text(Some(NV), false).unwrap()).unwrap();
        assert_eq!(take(&dir), None);
        assert!(!marker.exists());
        assert!(!dir.join(FILE).exists());

        std::fs::write(dir.join(FILE), "hdrFailedStart:other\n").unwrap();
        std::fs::write(&marker, loss_text(Some(NV), true).unwrap()).unwrap();
        assert_eq!(take(&dir).as_deref(), Some(NV));
        assert!(!marker.exists());
        let saved = Graphics::parse(&std::fs::read_to_string(dir.join(FILE)).unwrap());
        assert_eq!(saved.safe_start.as_deref(), Some(NV));
        assert_eq!(saved.hdr_failed_start.as_deref(), Some("other"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A settings file that can't be read (a folder in its place, on every system) is left
    /// alone, nothing is held, and the marker stays for a later start.
    #[test]
    fn an_unreadable_settings_file_keeps_the_marker() {
        let dir = scratch("unreadable");
        std::fs::create_dir(dir.join(FILE)).unwrap();
        std::fs::write(dir.join(MARKER), loss_text(Some(NV), true).unwrap()).unwrap();
        assert_eq!(take(&dir), None);
        assert!(dir.join(MARKER).exists());
        assert!(dir.join(FILE).is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The hold applies on its driver only, never offscreen (no driver), and goes when the
    /// window starts on another driver.
    #[test]
    fn the_hold_goes_with_the_driver() {
        let mut g = Graphics {
            safe_start: Some(NV.into()),
            ..Graphics::default()
        };
        assert!(super::super::output::held(&g, Some(NV)));
        assert!(!super::super::output::held(&g, None));
        assert!(!g.safe_start_holds_hdr(NV_NEW));
        assert!(!g.settle_safe_start(NV));
        assert!(g.settle_safe_start(NV_NEW));
        assert_eq!(g.safe_start, None);
    }

    /// Only the run that wrote the marker removes it: after a minute of frames following its
    /// recovery, or at a clean exit.
    #[test]
    fn a_run_removes_only_its_own_marker() {
        let t = Instant::now();
        let mut other = Proof::default();
        assert!(!other.presented(t + HEALTHY_AFTER_RECOVERY));
        assert!(!other.exit());

        let mut own = Proof::default();
        own.lost();
        own.recovered(t);
        assert!(!own.presented(t + Duration::from_secs(59)));
        assert!(own.presented(t + HEALTHY_AFTER_RECOVERY));
        assert!(!own.presented(t + Duration::from_secs(120)));
        assert!(!own.exit());

        own.lost();
        assert!(own.exit());
    }
}
