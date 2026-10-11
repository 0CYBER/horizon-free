//! Window sizing and frame pacing of 26.3: `Window.calculateScale` and
//! `setGuiScale`, `MouseHandler`'s scaled positions, `FramerateLimitTracker`,
//! `FramerateLimiter` and the vsync present-mode choice.

/// `Window.calculateScale`: the largest scale that leaves at least 320x240
/// GUI pixels, capped by `max_scale` (0 = automatic); an odd result is bumped
/// to even when unicode is enforced.
pub fn calculate_scale(framebuffer: [i32; 2], max_scale: i32, enforce_unicode: bool) -> i32 {
    let [width, height] = framebuffer;
    let mut scale = 1;
    while scale != max_scale
        && scale < width
        && scale < height
        && width / (scale + 1) >= 320
        && height / (scale + 1) >= 240
    {
        scale += 1;
    }
    if enforce_unicode && scale % 2 != 0 {
        scale += 1;
    }
    scale
}

/// `Window.setGuiScale`: the GUI-scaled size of a framebuffer.
pub fn gui_scaled_size(framebuffer: [i32; 2], gui_scale: i32) -> [i32; 2] {
    let scaled = |v: i32| (f64::from(v) / f64::from(gui_scale)).ceil() as i32;
    [scaled(framebuffer[0]), scaled(framebuffer[1])]
}

/// `MouseHandler.getScaledXPos`/`getScaledYPos`.
pub fn scaled_pos(pos: f64, gui_scaled: i32, screen: i32) -> f64 {
    pos * f64::from(gui_scaled) / f64::from(screen)
}

/// `InactivityFpsLimit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InactivityFpsLimit {
    Minimized,
    Afk,
}

/// `FramerateLimitTracker.FramerateThrottleReason`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThrottleReason {
    None,
    WindowIconified,
    LongAfk,
    ShortAfk,
    OutOfLevelMenu,
}

/// What `FramerateLimitTracker` looks at.
#[derive(Clone, Copy, Debug)]
pub struct FrameContext {
    pub inactivity: InactivityFpsLimit,
    pub iconified: bool,
    pub focused: bool,
    pub exclusive_fullscreen: bool,
    pub now_ms: i64,
    pub latest_input_ms: i64,
    pub has_level: bool,
    pub has_screen: bool,
    pub has_overlay: bool,
}

/// `getThrottleReason`.
pub fn throttle_reason(c: FrameContext) -> ThrottleReason {
    if !c.iconified && (c.focused || !c.exclusive_fullscreen) {
        if c.inactivity == InactivityFpsLimit::Afk {
            let afk = c.now_ms - c.latest_input_ms;
            if afk > 600_000 {
                return ThrottleReason::LongAfk;
            }
            if afk > 60_000 {
                return ThrottleReason::ShortAfk;
            }
        }
        if c.has_level || (!c.has_screen && !c.has_overlay) {
            ThrottleReason::None
        } else {
            ThrottleReason::OutOfLevelMenu
        }
    } else {
        ThrottleReason::WindowIconified
    }
}

/// `getFramerateLimit`.
pub fn framerate_limit(option_limit: i32, c: FrameContext) -> i32 {
    match throttle_reason(c) {
        ThrottleReason::None => option_limit,
        ThrottleReason::WindowIconified | ThrottleReason::LongAfk => 10,
        ThrottleReason::ShortAfk => option_limit.min(30),
        ThrottleReason::OutOfLevelMenu => 60,
    }
}

/// `isHeavilyThrottled`.
pub fn heavily_throttled(reason: ThrottleReason) -> bool {
    matches!(
        reason,
        ThrottleReason::WindowIconified | ThrottleReason::LongAfk
    )
}

/// Present modes the renderer can choose between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentMode {
    Fifo,
    Mailbox,
    Immediate,
}

/// `GpuSurface.PresentMode.getSupportedVsyncMode`: vsync means Fifo only,
/// otherwise the first of Immediate, Mailbox, Fifo that is supported.
pub fn vsync_present_mode(supported: &[PresentMode], vsync: bool) -> Option<PresentMode> {
    let preferred: &[PresentMode] = if vsync {
        &[PresentMode::Fifo]
    } else {
        &[
            PresentMode::Immediate,
            PresentMode::Mailbox,
            PresentMode::Fifo,
        ]
    };
    preferred.iter().copied().find(|m| supported.contains(m))
}

/// `FramerateLimiter.limitDisplayFPS` with an injectable clock so the sleep
/// and spin decisions are testable.
#[derive(Clone, Debug)]
pub struct FramerateLimiter {
    last_frame_time: i64,
    average_overshoot_ns: i64,
    last_framerate_limit: i32,
}

/// Time source of the limiter.
pub trait LimiterClock {
    fn nanos(&mut self) -> i64;
    /// `LockSupport.parkNanos`.
    fn park(&mut self, nanos: i64);
    /// `Thread.onSpinWait`.
    fn spin(&mut self);
    /// `Thread.interrupted`: an interrupted limiter skips that sleep.
    fn interrupted(&mut self) -> bool {
        false
    }
}

/// The real clock: `System.nanoTime` from a fixed origin, sleeping for
/// `LockSupport.parkNanos` and a CPU spin hint for `Thread.onSpinWait`.
#[derive(Clone, Copy, Debug)]
pub struct SystemClock {
    origin: std::time::Instant,
}
impl Default for SystemClock {
    fn default() -> Self {
        Self {
            origin: std::time::Instant::now(),
        }
    }
}
impl LimiterClock for SystemClock {
    fn nanos(&mut self) -> i64 {
        i64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(i64::MAX)
    }
    fn park(&mut self, nanos: i64) {
        std::thread::sleep(std::time::Duration::from_nanos(nanos.max(0) as u64));
    }
    fn spin(&mut self) {
        std::hint::spin_loop();
    }
}

/// `Minecraft.runTick`'s frame-limiter step: only limits below 260
/// (the "unlimited" setting) pace frames.
pub fn end_frame(
    limiter: &mut FramerateLimiter,
    framerate_limit: i32,
    clock: &mut impl LimiterClock,
) {
    if framerate_limit < 260 {
        limiter.limit_display_fps(framerate_limit, clock);
    }
}

impl FramerateLimiter {
    pub fn new(now_ns: i64) -> Self {
        Self::with_state(now_ns, 0, 0)
    }
    /// The limiter's static state: `lastFrameTime`, `averageOvershootNs`
    /// and `lastFramerateLimit`.
    pub fn with_state(
        last_frame_time: i64,
        average_overshoot_ns: i64,
        last_framerate_limit: i32,
    ) -> Self {
        Self {
            last_frame_time,
            average_overshoot_ns,
            last_framerate_limit,
        }
    }
    pub fn average_overshoot_ns(&self) -> i64 {
        self.average_overshoot_ns
    }
    pub fn last_framerate_limit(&self) -> i32 {
        self.last_framerate_limit
    }
    pub fn limit_display_fps(&mut self, framerate_limit: i32, clock: &mut impl LimiterClock) {
        let target_time_per_frame = 1_000_000_000 / i64::from(framerate_limit);
        let target_time_ns = self.last_frame_time + target_time_per_frame;
        if framerate_limit != self.last_framerate_limit {
            self.average_overshoot_ns = 0;
            self.last_framerate_limit = framerate_limit;
        }
        loop {
            let remaining = target_time_ns - clock.nanos();
            if remaining <= 0 {
                break;
            }
            if remaining > self.average_overshoot_ns + 500_000 {
                let sleep_start = clock.nanos();
                let expected = remaining - self.average_overshoot_ns - 500_000;
                if !clock.interrupted() {
                    clock.park(expected);
                    let duration = clock.nanos() - sleep_start;
                    let overshoot = duration - expected;
                    if overshoot > 0 && overshoot < 25_000_000 {
                        self.average_overshoot_ns = (0.1 * overshoot as f64
                            + 0.9 * self.average_overshoot_ns as f64)
                            as i64;
                        self.average_overshoot_ns = self.average_overshoot_ns.min(2_000_000);
                    }
                }
            } else {
                clock.spin();
            }
        }
        self.last_frame_time = clock.nanos();
    }
    pub fn last_frame_time(&self) -> i64 {
        self.last_frame_time
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Virtual {
        now: i64,
        oversleep: i64,
    }
    impl LimiterClock for Virtual {
        fn nanos(&mut self) -> i64 {
            self.now
        }
        fn park(&mut self, nanos: i64) {
            self.now += nanos + self.oversleep;
        }
        fn spin(&mut self) {
            self.now += 1000;
        }
    }

    #[test]
    fn scale_matches_the_320_by_240_rule() {
        assert_eq!(calculate_scale([854, 480], 0, false), 2);
        assert_eq!(calculate_scale([1920, 1080], 0, false), 4);
        assert_eq!(calculate_scale([1920, 1080], 3, false), 3);
        assert_eq!(calculate_scale([1920, 1080], 3, true), 4);
        assert_eq!(gui_scaled_size([1921, 1081], 4), [481, 271]);
    }
    #[test]
    fn limiter_paces_frames_at_the_target() {
        let mut limiter = FramerateLimiter::new(0);
        let mut clock = Virtual {
            now: 1_000_000,
            oversleep: 200_000,
        };
        limiter.limit_display_fps(100, &mut clock);
        assert!(clock.now >= 10_000_000);
        assert!(clock.now < 10_000_000 + 1_000_000);
    }
    #[test]
    fn vsync_only_uses_fifo() {
        let all = [
            PresentMode::Mailbox,
            PresentMode::Fifo,
            PresentMode::Immediate,
        ];
        assert_eq!(vsync_present_mode(&all, true), Some(PresentMode::Fifo));
        assert_eq!(
            vsync_present_mode(&all, false),
            Some(PresentMode::Immediate)
        );
        assert_eq!(
            vsync_present_mode(&[PresentMode::Fifo], false),
            Some(PresentMode::Fifo)
        );
    }
}
