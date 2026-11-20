//! `ClientHost::request_screenshot`: a client mod takes a screenshot through F2's
//! own path (`Game::take_screenshot`), so the `screenshot` hook, the file name and
//! the chat line are F2's. Unlike F2's, its files never replace one
//! (`PendingScreenshot::no_replace`), and all mods share one budget per session.
use super::Game;
use mod_api::ScreenshotError;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Uncompressed bytes all mods' screenshots may write per game session: about 500
/// at 1920 by 1080, far past what a screenshot mod takes in play, but a bound on a
/// mod that captures on every tick.
pub(super) const MOD_SCREENSHOT_BUDGET: u64 = 4 << 30;

/// The least a mod's capture is charged, whatever size the hook left its image: the
/// budget allows at most 4096 captures (and so as many new folders) per session.
pub(super) const MOD_SCREENSHOT_MIN_CHARGE: u64 = 1 << 20;

/// What a mod's capture of `raw_bytes` uncompressed bytes costs the budget.
pub(super) fn mod_screenshot_charge(raw_bytes: u64) -> u64 {
    raw_bytes.max(MOD_SCREENSHOT_MIN_CHARGE)
}

/// Whether a capture costing `charge` still fits after `written` bytes.
pub(super) fn within_budget(written: u64, charge: u64) -> bool {
    written.saturating_add(charge) <= MOD_SCREENSHOT_BUDGET
}

/// Marks the `screenshot` hook as running until dropped, an unwind included.
pub(super) struct HookRunning(Arc<AtomicBool>);
impl HookRunning {
    pub(super) fn start(flag: &Arc<AtomicBool>) -> Self {
        flag.store(true, Ordering::Relaxed);
        Self(flag.clone())
    }
}
impl Drop for HookRunning {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

impl Game<'_> {
    pub(in crate::game) fn request_screenshot(&mut self) -> Result<(), ScreenshotError> {
        // A server thread's `Game` has no window to grab, as `narrate` says for
        // the narrator.
        let available = self.options.execution_role.is_client() && self.renderer.is_some();
        // Everything that makes a mod's request wait lives in the renderer or ends on
        // its own: a capture queued or read back (dropped with the renderer when a
        // reload replaces it), the hook's run, a write in progress, a sequence. F2's
        // replace branch would otherwise drop a queued capture (another F2, a world
        // icon, an agent capture).
        let busy = self.ui.screenshot_hook.load(Ordering::Relaxed)
            || !self.ui.screenshot_writes.is_empty()
            || self.screenshot_sequence.is_some()
            || self
                .renderer
                .as_ref()
                .is_some_and(|r| r.screenshot_in_flight());
        if let Some(refusal) = screenshot_refusal(available, busy, self.ui.mod_screenshot_bytes) {
            return Err(refusal);
        }
        // Past the checks above, `take_screenshot` only declines when it cannot
        // create the screenshots folder.
        let path = self.take_screenshot().ok_or(ScreenshotError::Folder)?;
        self.ui.mod_screenshot = Some(path);
        Ok(())
    }
}

/// Why a mod's screenshot is refused before anything is queued, if it is: no
/// window, one still in progress, or the session's budget used up.
pub(super) fn screenshot_refusal(
    available: bool,
    busy: bool,
    written: u64,
) -> Option<ScreenshotError> {
    if !available {
        Some(ScreenshotError::Unavailable)
    } else if busy {
        Some(ScreenshotError::Busy)
    } else if !within_budget(written, MOD_SCREENSHOT_MIN_CHARGE) {
        Some(ScreenshotError::BudgetExhausted)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_with_nothing_in_progress_and_budget_left_is_accepted() {
        assert_eq!(screenshot_refusal(true, false, 0), None);
        assert_eq!(
            screenshot_refusal(
                true,
                false,
                MOD_SCREENSHOT_BUDGET - MOD_SCREENSHOT_MIN_CHARGE
            ),
            None
        );
        assert_eq!(
            screenshot_refusal(false, false, 0),
            Some(ScreenshotError::Unavailable)
        );
        assert_eq!(
            screenshot_refusal(true, true, 0),
            Some(ScreenshotError::Busy)
        );
        assert_eq!(
            screenshot_refusal(
                true,
                false,
                MOD_SCREENSHOT_BUDGET - MOD_SCREENSHOT_MIN_CHARGE + 1
            ),
            Some(ScreenshotError::BudgetExhausted)
        );
    }

    /// A 1 by 1 image costs the minimum, so a hook that shrinks every capture gets at
    /// most 4096 of them; a capture that would pass the budget is refused before writing.
    #[test]
    fn a_shrunk_capture_is_charged_the_minimum_and_the_last_one_must_fit() {
        assert_eq!(mod_screenshot_charge(4), MOD_SCREENSHOT_MIN_CHARGE);
        let full_hd = 1920 * 1080 * 4;
        assert_eq!(mod_screenshot_charge(full_hd), full_hd);
        assert_eq!(MOD_SCREENSHOT_BUDGET / MOD_SCREENSHOT_MIN_CHARGE, 4096);
        assert!(within_budget(MOD_SCREENSHOT_BUDGET - full_hd, full_hd));
        assert!(!within_budget(MOD_SCREENSHOT_BUDGET - full_hd + 1, full_hd));
        assert!(!within_budget(u64::MAX, 1));
    }

    #[test]
    fn the_hook_flag_is_cleared_when_a_listener_panics() {
        let flag = Arc::new(AtomicBool::new(false));
        let caught = std::panic::catch_unwind(|| {
            let _hook = HookRunning::start(&flag);
            assert!(flag.load(Ordering::Relaxed));
            panic!("a listener panicked");
        });
        assert!(caught.is_err());
        assert!(!flag.load(Ordering::Relaxed));
    }
}
