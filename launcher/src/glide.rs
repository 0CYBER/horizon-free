//! A mouse wheel's notch glides instead of jumping. GPUI moves a scroll area by a whole notch
//! (three lines on Windows) at once; the launcher takes the notch before any scroll area sees it
//! and hands it on in fractions of a line, one a frame, over [`motion::GLIDE`]. Each step is an
//! ordinary wheel event, so every scroll area glides, with a handle or without, turns the lines
//! into pixels with its own line height as it did the whole notch, and a list inside a page still
//! gives the wheel to the page at its end ([`crate::scrollbar::TakeWheel`]).
//!
//! The glide stays where it began, as a browser's wheel scrolling does: its steps go to the point
//! of its first notch, so the pointer moving on to another list does not take the rest with it,
//! and the pointer is put back after each step, so hovers and the cursor stay with it.
//!
//! Trackpads (pixel deltas) pass as they are. Under reduced motion ([`motion::still`]) and in
//! Performance mode a notch jumps, as before. A press ends the glide where it is, and while a
//! button is held (a drag, the scrollbar's thumb) notches jump: a glide would move the pointer
//! back and forth under it.
use crate::motion;
use gpui::{
    canvas, prelude::*, App, DispatchPhase, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    PlatformInput, Point, ScrollDelta, ScrollEvent, ScrollSource, TouchPhase, Window,
};
use std::cell::Cell;
use std::time::Instant;

/// The notches still gliding, in lines: what they add up to since the run began on its curve, and
/// how much of it the steps so far scrolled; and the point they scroll, where the first one was.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Run {
    began: Instant,
    whole: Point<f32>,
    done: Point<f32>,
    at: Point<Pixels>,
}

impl Run {
    /// A notch of `by` lines at `now`, the pointer `at`: what `run` has left plus the notch, from
    /// here, on a fresh curve, still where `run` began. A notch the other way first takes back
    /// what is left, so the glide turns at once.
    fn add(run: Option<Run>, by: Point<f32>, at: Point<Pixels>, now: Instant) -> Run {
        let (left, at) = run.map_or((Point::default(), at), |run| (run.whole - run.done, run.at));
        Run {
            began: now,
            whole: left + by,
            done: Point::default(),
            at,
        }
    }

    /// The lines to scroll at `now` since the last step, and whether the run ends with them. A
    /// frame late enough to be past the curve's end scrolls all that is left.
    fn step(&mut self, now: Instant) -> (Point<f32>, bool) {
        let t = now.saturating_duration_since(self.began).as_secs_f32() / motion::GLIDE.secs();
        let over = t >= 1.;
        let at = if over {
            self.whole
        } else {
            self.whole * motion::GLIDE.ease(t)
        };
        let by = at - self.done;
        self.done = at;
        (by, over)
    }
}

thread_local! {
    /// The wheel's run, while it glides.
    static RUN: Cell<Option<Run>> = const { Cell::new(None) };
    /// Whether steps are being handed on, one a frame ([`step`]).
    static TICKING: Cell<bool> = const { Cell::new(false) };
    /// While a step is being handed on: the listener lets it through.
    static HANDING: Cell<bool> = const { Cell::new(false) };
    /// Whether a mouse button is held.
    static HELD: Cell<bool> = const { Cell::new(false) };
    /// Whether notches glide: off in Performance mode ([`set_on`]).
    static ON: Cell<bool> = const { Cell::new(true) };
}

/// Notches glide unless `on` is false (Performance mode); set on each frame.
pub fn set_on(on: bool) {
    ON.set(on);
}

fn gliding() -> bool {
    ON.get() && !motion::still()
}

/// Takes the window's wheel notches before any scroll area sees them. The launcher's root paints
/// it first, so its listener runs before all others.
pub fn wheel() -> impl IntoElement {
    canvas(
        |_, _, _| (),
        |_, _, window, _| {
            window.on_mouse_event(|ev: &ScrollEvent, phase, window, cx| {
                let ScrollDelta::Lines(lines) = ev.delta else {
                    return;
                };
                if phase != DispatchPhase::Capture || HANDING.get() || HELD.get() || !gliding() {
                    return;
                }
                RUN.set(Some(Run::add(
                    RUN.get(),
                    lines,
                    ev.position,
                    Instant::now(),
                )));
                cx.stop_propagation();
                if !TICKING.replace(true) {
                    // The first step right after this event, the rest a frame apart.
                    window.defer(cx, step);
                }
            });
            window.on_mouse_event(|_: &MouseDownEvent, phase, _, _| {
                // A press (the scrollbar's thumb, a page in the sidebar) stops the glide there.
                if phase.capture() {
                    RUN.set(None);
                    HELD.set(true);
                }
            });
            window.on_mouse_event(|_: &MouseUpEvent, phase, _, _| {
                if phase.capture() {
                    HELD.set(false);
                }
            });
            window.on_mouse_event(|ev: &MouseMoveEvent, phase, _, _| {
                // Also a press that began outside the window, or a release it missed.
                if phase.capture() {
                    HELD.set(ev.pressed_button.is_some());
                }
            });
        },
    )
    .absolute()
    .size_0()
}

/// Hands the run's next step to the window, as a wheel event where the run began.
fn step(window: &mut Window, cx: &mut App) {
    let Some(mut run) = RUN.get() else {
        TICKING.set(false);
        return;
    };
    let (by, over) = run.step(Instant::now());
    RUN.set((!over).then_some(run));
    if by != Point::default() {
        // GPUI takes a wheel event's point as the pointer's; one that moved on since the run
        // began is put back after the step.
        let pointer = window.mouse_position();
        let modifiers = window.modifiers();
        HANDING.set(true);
        window.dispatch_event(
            PlatformInput::Scroll(ScrollEvent {
                position: run.at,
                delta: ScrollDelta::Lines(by),
                modifiers,
                touch_phase: TouchPhase::Moved,
                source: ScrollSource::Pointer,
            }),
            cx,
        );
        HANDING.set(false);
        if pointer != run.at {
            window.dispatch_event(
                PlatformInput::MouseMove(MouseMoveEvent {
                    position: pointer,
                    pressed_button: None,
                    modifiers,
                }),
                cx,
            );
        }
    }
    if over {
        TICKING.set(false);
    } else {
        window.on_next_frame(step);
    }
}

#[cfg(test)]
mod tests {
    use super::{motion, Run};
    use gpui::{point, px, Pixels, Point};
    use std::time::{Duration, Instant};

    fn down(y: f32) -> Point<f32> {
        point(0., y)
    }

    /// Where the pointer was for a run's first notch.
    fn here() -> Point<Pixels> {
        point(px(400.), px(300.))
    }

    #[test]
    fn a_glide_stays_where_its_first_notch_was() {
        let t0 = Instant::now();
        let mut run = Run::add(None, down(-3.), here(), t0);
        run.step(t0 + Duration::from_millis(32));
        // The pointer moved on to another list and the wheel turned again: the rest, and this
        // notch, still scroll the first one.
        let elsewhere = point(px(900.), px(120.));
        let then = t0 + Duration::from_millis(32);
        let run = Run::add(Some(run), down(-3.), elsewhere, then);
        assert_eq!(run.at, here());
        // Once the run is over, the next notch begins one where the pointer is.
        assert_eq!(Run::add(None, down(-3.), elsewhere, then).at, elsewhere);
    }

    /// Steps add up to their run in floats: equal to within a thousandth of a line.
    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn a_notch_glides_its_whole_distance_and_ends_on_time() {
        let t0 = Instant::now();
        let mut run = Run::add(None, down(-3.), here(), t0);
        let mut scrolled = 0.;
        let mut steps = 0;
        let mut at = t0;
        loop {
            at += Duration::from_millis(16);
            let (by, over) = run.step(at);
            assert!(by.y <= 0., "every step goes the notch's way");
            scrolled += by.y;
            steps += 1;
            if over {
                break;
            }
        }
        assert!(close(scrolled, -3.), "{scrolled}");
        assert!(
            steps <= motion::GLIDE.ms as usize / 16 + 1,
            "{steps} frames"
        );
    }

    #[test]
    fn a_notch_moves_most_of_the_way_early() {
        let t0 = Instant::now();
        let mut run = Run::add(None, down(-3.), here(), t0);
        let (first, _) = run.step(t0 + motion::GLIDE.duration() / 4);
        assert!(
            first.y < -1.5,
            "ease-out: half the way in a quarter of the time"
        );
    }

    #[test]
    fn a_notch_during_a_glide_adds_to_what_is_left() {
        let t0 = Instant::now();
        let mut run = Run::add(None, down(-3.), here(), t0);
        let (moved, over) = run.step(t0 + Duration::from_millis(48));
        assert!(!over);
        let then = t0 + Duration::from_millis(48);
        let mut run = Run::add(Some(run), down(-3.), here(), then);
        assert!(close(run.whole.y, -6. - moved.y));
        let (rest, over) = run.step(then + motion::GLIDE.duration());
        assert!(over);
        assert!(close(moved.y + rest.y, -6.));
    }

    #[test]
    fn a_notch_the_other_way_turns_the_glide() {
        let t0 = Instant::now();
        let mut run = Run::add(None, down(-3.), here(), t0);
        let (moved, _) = run.step(t0 + Duration::from_millis(16));
        let then = t0 + Duration::from_millis(16);
        let mut run = Run::add(Some(run), down(3.), here(), then);
        let (by, _) = run.step(then + Duration::from_millis(16));
        assert!(by.y > 0., "the next step already goes back");
        let (rest, over) = run.step(then + motion::GLIDE.duration());
        assert!(over);
        assert!(close(moved.y + by.y + rest.y, 0.));
    }

    #[test]
    fn a_late_frame_scrolls_all_that_is_left() {
        let t0 = Instant::now();
        let mut run = Run::add(None, point(-1., -3.), here(), t0);
        let (by, over) = run.step(t0 + Duration::from_secs(2));
        assert!(over);
        assert_eq!(by, point(-1., -3.));
        let (none, _) = run.step(t0 + Duration::from_secs(3));
        assert_eq!(none, Point::default());
    }
}
