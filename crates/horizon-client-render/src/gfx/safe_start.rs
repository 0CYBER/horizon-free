//! What a start that lost its graphics device leaves for the next one (#19206). The window's
//! device loss writes [`MARKER`] beside `horizon-graphics.txt` (`Game::recover_device`); a
//! recovery that keeps presenting frames for [`HEALTHY_AFTER_RECOVERY`], or a clean exit, removes
//! it again. The next window client's start ([`take`]) turns a loss with HDR output on into a hold
//! kept in that file (`Graphics::safe_start`, key `safeStart`): HDR output stays off on that
//! driver until an HDR mode is chosen or the driver changes. A loss with HDR output off only says
//! so in the log. Offscreen renderers, split-screen guests, dedicated servers and scripted starts
//! never write the marker or take it.
use crate::graphics::{FILE, Graphics, GraphicsApi};
use std::time::{Duration, Instant};

/// Written beside [`FILE`] when the window's graphics device is lost.
pub const MARKER: &str = "horizon-device-lost";
/// Written beside [`FILE`] when [`super::device::arm_search_watchdog`] times out (#19573).
pub const WATCHDOG_MARKER: &str = "horizon-gpu-watchdog-hung";

static WATCHDOG_DIR: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

/// Records the settings directory for [`mark_watchdog_hang`] / [`clear_watchdog_hang`].
pub fn arm_watchdog_dir(dir: &std::path::Path) {
    if let Ok(mut guard) = WATCHDOG_DIR.lock() {
        *guard = Some(dir.to_path_buf());
    }
}

/// Writes [`WATCHDOG_MARKER`] with the hung `api` when a watchdog directory was armed,
/// `HORIZON_GRAPHICS_API` is not overriding the backend, and the search has not already completed.
/// Serialized with [`clear_watchdog_hang`] under `WATCHDOG_DIR`.
pub fn mark_watchdog_hang(searched: &std::sync::atomic::AtomicBool, api: GraphicsApi) {
    mark_watchdog_hang_with_env(searched, api, GraphicsApi::from_env().is_some());
}

fn mark_watchdog_hang_with_env(
    searched: &std::sync::atomic::AtomicBool,
    api: GraphicsApi,
    env_override: bool,
) {
    if env_override {
        return;
    }
    let Ok(guard) = WATCHDOG_DIR.lock() else {
        return;
    };
    if searched.load(std::sync::atomic::Ordering::Acquire) {
        return;
    }
    if let Some(dir) = guard.as_ref() {
        let _ = std::fs::write(dir.join(WATCHDOG_MARKER), format!("api:{}\n", api.name()));
    }
}

/// Removes [`WATCHDOG_MARKER`] after a successful GPU device search, serialized with
/// [`mark_watchdog_hang`] under `WATCHDOG_DIR`.
pub fn clear_watchdog_hang() {
    let Ok(guard) = WATCHDOG_DIR.lock() else {
        return;
    };
    if let Some(dir) = guard.as_ref() {
        let _ = std::fs::remove_file(dir.join(WATCHDOG_MARKER));
    }
}

/// Next graphics API to try after `hung` timed out inside the driver on `os`.
pub fn fallback_api_for_hang(hung: GraphicsApi, os: &str) -> Option<GraphicsApi> {
    if os == "windows" {
        match hung {
            GraphicsApi::Auto | GraphicsApi::Vulkan => Some(GraphicsApi::Dx12),
            GraphicsApi::Dx12 => Some(GraphicsApi::OpenGl),
            GraphicsApi::OpenGl => None,
        }
    } else {
        match hung {
            GraphicsApi::Auto | GraphicsApi::Vulkan => Some(GraphicsApi::OpenGl),
            GraphicsApi::OpenGl | GraphicsApi::Dx12 => None,
        }
    }
}

/// Consumes [`WATCHDOG_MARKER`] in `dir` and, if `horizon-graphics.txt` still asks for the hung
/// API (or `auto`) and `HORIZON_GRAPHICS_API` is not set, records a `watchdogApi` hold in
/// `horizon-graphics.txt` without changing `graphicsApi` (#19573).
pub fn take_watchdog(dir: &std::path::Path, os: &str) -> Option<GraphicsApi> {
    take_watchdog_with_env(dir, os, GraphicsApi::from_env().is_some())
}

fn take_watchdog_with_env(
    dir: &std::path::Path,
    os: &str,
    env_override: bool,
) -> Option<GraphicsApi> {
    if env_override {
        return None;
    }
    let marker = dir.join(WATCHDOG_MARKER);
    let text = std::fs::read_to_string(&marker).ok()?;
    let hung = text
        .lines()
        .find_map(|line| line.strip_prefix("api:"))
        .and_then(|v| GraphicsApi::from_name(v.trim()));
    let Some(hung) = hung else {
        let _ = std::fs::remove_file(&marker);
        return None;
    };
    let Some(next) = fallback_api_for_hang(hung, os) else {
        let _ = std::fs::remove_file(&marker);
        return None;
    };
    let file = dir.join(FILE);
    let settings = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            log::warn!(
                "Horizon renderer: the last start hung while picking a GPU ({}), but {FILE} can't be read ({e}); graphicsApi is unchanged",
                hung.name()
            );
            return None;
        }
    };
    let mut g = Graphics::parse(&settings);
    let current = g.effective_api();
    if current != GraphicsApi::Auto && current != hung {
        let _ = std::fs::remove_file(&marker);
        return None;
    }
    log::warn!(
        "Horizon renderer: the last start hung while picking a GPU with {}; holding graphics backend at {} for this driver",
        hung.name(),
        next.name()
    );
    g.watchdog_api = Some(next);
    g.watchdog_driver = None;
    if std::fs::write(&file, g.serialize()).is_ok() {
        let _ = std::fs::remove_file(&marker);
    }
    Some(next)
}
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

    /// The window's renderer runs on `driver`: a device-loss or watchdog-step-down hold on another
    /// driver (an update, another GPU) is dropped, so HDR output and the configured `graphicsApi`
    /// are tried again. True when it changed.
    pub(crate) fn settle_safe_start(&mut self, driver: &str) -> bool {
        let mut changed = false;
        if self
            .safe_start
            .as_deref()
            .is_some_and(|held| held != driver)
        {
            self.safe_start = None;
            changed = true;
        }
        if self.watchdog_api.is_some() {
            match self.watchdog_driver.as_deref() {
                None => {
                    self.watchdog_driver = Some(driver.to_owned());
                    changed = true;
                }
                Some(held) if held != driver => {
                    self.watchdog_api = None;
                    self.watchdog_driver = None;
                    changed = true;
                }
                _ => {}
            }
        }
        changed
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

    #[test]
    fn watchdog_hang_steps_down_graphics_api_and_respects_manual_override() {
        let dir = scratch("watchdog_step_down");
        std::fs::write(dir.join(WATCHDOG_MARKER), "api:auto\n").unwrap();
        assert_eq!(
            take_watchdog_with_env(&dir, "windows", false),
            Some(GraphicsApi::Dx12)
        );
        assert!(!dir.join(WATCHDOG_MARKER).exists());
        let mut saved = Graphics::parse(&std::fs::read_to_string(dir.join(FILE)).unwrap());
        assert_eq!(saved.api, GraphicsApi::Auto);
        assert_eq!(saved.watchdog_api, Some(GraphicsApi::Dx12));
        assert_eq!(saved.effective_api(), GraphicsApi::Dx12);

        // Settling on the active driver keeps the hold; changing drivers drops it.
        assert!(saved.settle_safe_start(NV));
        assert_eq!(saved.watchdog_driver.as_deref(), Some(NV));
        assert!(!saved.settle_safe_start(NV));
        assert_eq!(saved.watchdog_api, Some(GraphicsApi::Dx12));
        assert!(saved.settle_safe_start(NV_NEW));
        assert_eq!(saved.watchdog_api, None);
        assert_eq!(saved.watchdog_driver, None);
        assert_eq!(saved.effective_api(), GraphicsApi::Auto);

        // Second hang on dx12 steps down to opengl on Windows without mutating graphicsApi.
        std::fs::write(dir.join(WATCHDOG_MARKER), "api:dx12\n").unwrap();
        assert_eq!(
            take_watchdog_with_env(&dir, "windows", false),
            Some(GraphicsApi::OpenGl)
        );
        let saved2 = Graphics::parse(&std::fs::read_to_string(dir.join(FILE)).unwrap());
        assert_eq!(saved2.api, GraphicsApi::Auto);
        assert_eq!(saved2.watchdog_api, Some(GraphicsApi::OpenGl));

        // Third hang on opengl has no further fallback.
        std::fs::write(dir.join(WATCHDOG_MARKER), "api:opengl\n").unwrap();
        assert_eq!(take_watchdog_with_env(&dir, "windows", false), None);

        // Manual override in horizon-graphics.txt is preserved.
        let manual = Graphics {
            api: GraphicsApi::OpenGl,
            ..Graphics::default()
        };
        manual.save(&dir);
        std::fs::write(dir.join(WATCHDOG_MARKER), "api:vulkan\n").unwrap();
        assert_eq!(take_watchdog_with_env(&dir, "windows", false), None);
        let after_manual = Graphics::parse(&std::fs::read_to_string(dir.join(FILE)).unwrap());
        assert_eq!(after_manual.api, GraphicsApi::OpenGl);
        assert_eq!(after_manual.watchdog_api, None);

        // Linux steps down from auto/vulkan to opengl without mutating graphicsApi.
        let linux_g = Graphics::default();
        linux_g.save(&dir);
        std::fs::write(dir.join(WATCHDOG_MARKER), "api:auto\n").unwrap();
        assert_eq!(
            take_watchdog_with_env(&dir, "linux", false),
            Some(GraphicsApi::OpenGl)
        );
        let after_linux = Graphics::parse(&std::fs::read_to_string(dir.join(FILE)).unwrap());
        assert_eq!(after_linux.api, GraphicsApi::Auto);
        assert_eq!(after_linux.watchdog_api, Some(GraphicsApi::OpenGl));

        // HORIZON_GRAPHICS_API override skips both marking and stepping down the file setting.
        arm_watchdog_dir(&dir);
        let not_done = std::sync::atomic::AtomicBool::new(false);
        mark_watchdog_hang_with_env(&not_done, GraphicsApi::Vulkan, true);
        assert!(!dir.join(WATCHDOG_MARKER).exists());
        std::fs::write(dir.join(WATCHDOG_MARKER), "api:vulkan\n").unwrap();
        assert_eq!(take_watchdog_with_env(&dir, "windows", true), None);
        assert!(dir.join(WATCHDOG_MARKER).exists());
        let _ = std::fs::remove_file(dir.join(WATCHDOG_MARKER));

        // Delayed timeout callback after search completed must not leave a stale marker.
        let done = std::sync::atomic::AtomicBool::new(true);
        mark_watchdog_hang_with_env(&done, GraphicsApi::Vulkan, false);
        assert!(!dir.join(WATCHDOG_MARKER).exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
