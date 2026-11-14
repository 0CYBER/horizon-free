// game-module-visibility: pub(crate)
//! Mods' bars and lines on the startup loading screen (`RegistrationContext::progress` and
//! `message`, NeoForge's `StartupNotificationManager`): kept here, drawn under the stage bar by
//! the startup menu, or logged on a dedicated server, which has no screen.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// How long a mod's line stays on the screen.
const LINE_TIME: Duration = Duration::from_secs(5);
/// The most bars drawn, newest first; older ones wait for a newer one to close.
const SHOWN_BARS: usize = 4;
/// The most lines drawn, newest last.
const SHOWN_LINES: usize = 3;
/// Bars open at once: past this, opening a bar returns `TooManyBars`.
const MAX_BARS: usize = 64;
/// Lines kept for drawing.
const MAX_LINES: usize = 32;
/// A label or line is cut to this many bytes when it is kept.
const MAX_TEXT: usize = 256;

struct Bar {
    id: u32,
    label: String,
    done: u32,
    steps: u32,
    /// The tenths of the steps a dedicated server has logged.
    logged: u64,
}

#[derive(Default)]
struct State {
    next: u32,
    bars: Vec<Bar>,
    lines: VecDeque<(String, Instant)>,
}

pub(crate) struct ModProgress {
    dedicated: bool,
    state: Mutex<State>,
}

static PROGRESS: OnceLock<Arc<ModProgress>> = OnceLock::new();

/// Hand mods the startup screen, before they register.
pub(crate) fn install(dedicated: bool) {
    let progress = PROGRESS.get_or_init(|| Arc::new(ModProgress::new(dedicated)));
    if let Err(error) = mod_api::__private::game::install_startup_screen(progress.clone()) {
        log::warn!("Horizon: {error}");
    }
}

/// The bars to draw, newest first, as their label and done fraction, and the lines still shown.
pub(crate) fn shown() -> (Vec<(String, f32)>, Vec<String>) {
    PROGRESS
        .get()
        .map(|progress| progress.shown(Instant::now()))
        .unwrap_or_default()
}

/// `text` cut to [`MAX_TEXT`] bytes, on one line.
fn one_line(text: &str) -> String {
    let mut end = text.len().min(MAX_TEXT);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end]
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

impl ModProgress {
    fn new(dedicated: bool) -> Self {
        Self {
            dedicated,
            state: Mutex::default(),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn shown(&self, now: Instant) -> (Vec<(String, f32)>, Vec<String>) {
        let mut state = self.state();
        state
            .lines
            .retain(|(_, at)| now.saturating_duration_since(*at) < LINE_TIME);
        let bars = state
            .bars
            .iter()
            .rev()
            .take(SHOWN_BARS)
            .map(|bar| {
                let done = if bar.steps == 0 {
                    0.0
                } else {
                    bar.done as f32 / bar.steps as f32
                };
                (bar.label.clone(), done)
            })
            .collect();
        let old = state.lines.len().saturating_sub(SHOWN_LINES);
        let lines = state
            .lines
            .iter()
            .skip(old)
            .map(|(line, _)| line.clone())
            .collect();
        (bars, lines)
    }

    /// Change bar `id` and, on a dedicated server, log each tenth of its steps it passes.
    fn update(&self, id: u32, change: impl FnOnce(&mut Bar)) {
        let mut state = self.state();
        let Some(bar) = state.bars.iter_mut().find(|bar| bar.id == id) else {
            return;
        };
        change(bar);
        bar.done = bar.done.min(bar.steps);
        let mut percentage = None;
        if self.dedicated && bar.steps > 0 {
            let tenths = u64::from(bar.done) * 10 / u64::from(bar.steps);
            if tenths > bar.logged {
                bar.logged = tenths;
                percentage = Some((bar.label.clone(), tenths * 10));
            }
        }
        drop(state);
        if let Some((label, percent)) = percentage {
            log::info!("{label}: {percent}%");
        }
    }
}

impl mod_api::StartupScreen for ModProgress {
    fn open(&self, label: String, steps: u32) -> Result<u32, mod_api::StartupProgressError> {
        let label = one_line(&label);
        let mut state = self.state();
        if state.bars.len() >= MAX_BARS {
            return Err(mod_api::StartupProgressError::TooManyBars);
        }
        if self.dedicated {
            log::info!("{label}");
        }
        loop {
            state.next = state.next.wrapping_add(1).max(1);
            if !state.bars.iter().any(|bar| bar.id == state.next) {
                break;
            }
        }
        let id = state.next;
        state.bars.push(Bar {
            id,
            label,
            done: 0,
            steps,
            logged: 0,
        });
        Ok(id)
    }
    fn step(&self, bar: u32) {
        self.update(bar, |bar| bar.done = bar.done.saturating_add(1));
    }
    fn set(&self, bar: u32, done: u32) {
        self.update(bar, |bar| bar.done = done);
    }
    fn label(&self, bar: u32, label: String) {
        let label = one_line(&label);
        let mut state = self.state();
        let Some(bar) = state.bars.iter_mut().find(|entry| entry.id == bar) else {
            return;
        };
        if bar.label == label {
            return;
        }
        bar.label = label;
        let logged = self.dedicated.then(|| bar.label.clone());
        drop(state);
        if let Some(label) = logged {
            log::info!("{label}");
        }
    }
    fn close(&self, bar: u32) {
        self.state().bars.retain(|kept| kept.id != bar);
    }
    fn message(&self, text: String) {
        let text = one_line(&text);
        if self.dedicated {
            log::info!("{text}");
            return;
        }
        let mut state = self.state();
        state.lines.push_back((text, Instant::now()));
        if state.lines.len() > MAX_LINES {
            state.lines.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::StartupScreen;

    #[test]
    fn multibyte_labels_stop_at_the_last_complete_character() {
        let screen = ModProgress::new(false);
        screen.open(format!("{}界", "é".repeat(127)), 1).unwrap();
        let bars = screen.shown(Instant::now()).0;
        assert_eq!(bars[0].0, "é".repeat(127));
        screen.message("é".repeat(128));
        assert_eq!(screen.shown(Instant::now()).1, ["é".repeat(128)]);
    }

    #[test]
    fn sixty_fifth_bar_is_refused_and_ids_do_not_alias_after_wrap() {
        let screen = ModProgress::new(false);
        let first = screen.open("first".into(), 1).unwrap();
        screen.state().next = u32::MAX;
        let second = screen.open("second".into(), 1).unwrap();
        assert_eq!(first, 1);
        assert_eq!(second, 2);
        for _ in 2..64 {
            screen.open("kept".into(), 1).unwrap();
        }
        let before = screen.shown(Instant::now()).0;
        assert_eq!(
            screen.open("refused".into(), 1),
            Err(mod_api::StartupProgressError::TooManyBars)
        );
        assert_eq!(screen.state().bars.len(), 64);
        screen.step(0);
        screen.close(0);
        assert_eq!(screen.shown(Instant::now()).0, before);
        screen.close(first);
        screen.open("replacement".into(), 1).unwrap();
    }

    #[test]
    fn zero_step_bar_remains_finite_and_clamped() {
        let screen = ModProgress::new(false);
        let bar = screen.open("empty".into(), 0).unwrap();
        screen.step(bar);
        screen.set(bar, u32::MAX);
        assert_eq!(screen.shown(Instant::now()).0, [("empty".into(), 0.0)]);
    }

    #[test]
    fn thirty_third_message_keeps_the_newest_lines_and_expires() {
        let screen = ModProgress::new(false);
        for n in 0..33 {
            screen.message(format!("message {n}"));
        }
        assert_eq!(screen.state().lines.len(), 32);
        let now = Instant::now();
        assert_eq!(
            screen.shown(now).1,
            ["message 30", "message 31", "message 32"]
        );
        assert!(screen.shown(now + Duration::from_secs(5)).1.is_empty());
    }

    #[test]
    fn bars_read_their_steps_newest_first_and_leave_when_closed() {
        let screen = ModProgress::new(false);
        let now = Instant::now;
        let first = screen.open("Baking\nmodels".into(), 10).unwrap();
        for _ in 0..4 {
            screen.step(first);
        }
        assert_eq!(
            screen.shown(now()).0,
            [("Baking models".to_owned(), 0.4_f32)]
        );
        screen.set(first, 99);
        assert_eq!(screen.shown(now()).0[0].1, 1.0);
        // A fifth bar at once: the oldest is not drawn until a newer one closes.
        let more: Vec<u32> = (0..4)
            .map(|i| screen.open(format!("bar {i}"), 2).unwrap())
            .collect();
        let labels = |screen: &ModProgress| -> Vec<String> {
            screen
                .shown(now())
                .0
                .into_iter()
                .map(|(label, _)| label)
                .collect()
        };
        assert_eq!(labels(&screen), ["bar 3", "bar 2", "bar 1", "bar 0"]);
        screen.close(more[3]);
        assert_eq!(
            labels(&screen),
            ["bar 2", "bar 1", "bar 0", "Baking models"]
        );
        for bar in more.into_iter().chain([first]) {
            screen.close(bar);
        }
        assert!(screen.shown(now()).0.is_empty());
        // Steps on a closed bar do nothing.
        screen.step(first);
        screen.message("Hello".into());
        let at = now();
        assert_eq!(screen.shown(at).1, ["Hello"]);
        assert!(screen.shown(at + LINE_TIME).1.is_empty());
    }
}
