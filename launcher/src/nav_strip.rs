//! Arrows for the top bar's page tabs when they overflow, each on an edge that fades into the
//! plate.
use crate::theme::Theme;
use crate::{glass_ui as ui, motion};
use gpui::{div, prelude::*, px, Div, ScrollHandle, Window};
use horizon_theme::glass::{fill, ink, radius};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Instant,
};

/// An arrow's button.
const ARROW: f32 = 32.;
/// The fading edge an arrow sits on: solid for its first 60%, then clear.
const EDGE: f32 = 52.;

/// The tabs' scroll, the ends it last drew arrows for, and the arrows' and the scroll's motion.
#[derive(Clone, Default)]
pub struct NavStrip {
    scroll: ScrollHandle,
    drawn: Rc<Cell<(bool, bool)>>,
    motion: Rc<RefCell<Motion>>,
    ticking: Rc<Cell<bool>>,
    /// Whether changes are instant: Performance mode or reduced motion, set on each render.
    instant: Rc<Cell<bool>>,
}

#[derive(Clone, Copy)]
struct Tween {
    from: f32,
    to: f32,
    began: Instant,
}

impl Tween {
    fn sample(self, now: Instant, curve: horizon_theme::Motion) -> (f32, bool) {
        let progress = now.saturating_duration_since(self.began).as_secs_f32() / curve.secs();
        if progress >= 1. {
            (self.to, false)
        } else {
            (
                self.from + (self.to - self.from) * curve.ease(progress),
                true,
            )
        }
    }
}

#[derive(Default)]
struct Fade {
    shown: bool,
    tween: Option<Tween>,
}

impl Fade {
    fn sample(&self, now: Instant) -> (f32, bool) {
        self.tween
            .map_or((if self.shown { 1. } else { 0. }, false), |tween| {
                tween.sample(now, horizon_theme::QUICK)
            })
    }

    fn show(&mut self, shown: bool, now: Instant, instant: bool) -> f32 {
        if self.shown != shown {
            self.tween = Some(Tween {
                from: self.sample(now).0,
                to: if shown { 1. } else { 0. },
                began: now,
            });
            self.shown = shown;
        }
        if instant {
            self.tween = None;
        }
        self.sample(now).0
    }
}

#[derive(Default)]
struct Motion {
    cues: [Fade; 2],
    slide: Option<Tween>,
}

impl Motion {
    fn advance(&mut self, offset: f32, max: f32, width: f32, right: bool, now: Instant) {
        // Repeated clicks add to the destination; reversing starts from what is visible.
        let base = self
            .slide
            .filter(|slide| (slide.to < slide.from) == right)
            .map_or(offset, |slide| slide.to);
        let to = next_offset(base, max, width, right);
        if self.slide.is_some_and(|slide| slide.to == to) {
            return;
        }
        self.slide = (offset != to).then_some(Tween {
            from: offset,
            to,
            began: now,
        });
    }

    fn step(&mut self, now: Instant, max: f32, instant: bool) -> Option<f32> {
        let slide = self.slide?;
        let (offset, running) = if instant {
            (slide.to, false)
        } else {
            slide.sample(now, horizon_theme::MOVE)
        };
        if !running || max <= 0. {
            self.slide = None;
        }
        Some(offset.clamp(-max.max(0.), 0.))
    }

    fn active(&self, now: Instant) -> bool {
        self.slide.is_some() || self.cues.iter().any(|cue| cue.sample(now).1)
    }
}

/// GPUI may briefly report an offset past an end while handling the wheel.
fn edges(offset: f32, max: f32) -> (bool, bool) {
    let max = max.max(0.);
    let position = (-offset).clamp(0., max);
    (position > 0.5, max - position > 0.5)
}

fn next_offset(offset: f32, max: f32, width: f32, right: bool) -> f32 {
    let step = (width * 0.75).max(32.);
    (offset + if right { -step } else { step }).clamp(-max.max(0.), 0.)
}

impl NavStrip {
    fn edges(&self) -> (bool, bool) {
        edges(
            self.scroll.offset().x.into(),
            self.scroll.max_offset().x.into(),
        )
    }

    fn changed(&self) -> bool {
        self.edges() != self.drawn.get()
    }

    /// Moves the tabs to where an arrow's scroll is at `now`.
    fn slide(&self, now: Instant) {
        let next = self.motion.borrow_mut().step(
            now,
            self.scroll.max_offset().x.into(),
            self.instant.get(),
        );
        if let Some(next) = next {
            let mut offset = self.scroll.offset();
            offset.x = px(next);
            self.scroll.set_offset(offset);
        }
    }

    fn animate(&self, window: &mut Window) {
        if self.ticking.replace(true) {
            return;
        }
        let strip = self.clone();
        window.on_next_frame(move |window, _| {
            strip.ticking.set(false);
            let now = Instant::now();
            strip.slide(now);
            window.refresh();
            if strip.motion.borrow().active(now) {
                strip.animate(window);
            }
        });
    }

    /// The strip around `tabs`; `performance` (Performance mode) makes its changes instant.
    pub fn render(&self, tabs: Div, t: &Theme, performance: bool, window: &mut Window) -> Div {
        let instant = performance || motion::still();
        self.instant.set(instant);
        let (left, right) = self.edges();
        self.drawn.set((left, right));
        let now = Instant::now();
        let (left_alpha, right_alpha, active) = {
            let mut state = self.motion.borrow_mut();
            let left_alpha = state.cues[0].show(left, now, instant);
            let right_alpha = state.cues[1].show(right, now, instant);
            (left_alpha, right_alpha, state.active(now))
        };
        if active {
            self.animate(window);
        }
        let measured = self.clone();
        let moved = self.clone();
        let tabs = tabs
            .on_children_prepainted(move |_, window, _| {
                // The first layout and resizing can change which ends have hidden tabs.
                if measured.changed() {
                    window.refresh();
                }
            })
            .id("nav-pages")
            .relative()
            .overflow_x_scroll()
            .track_scroll(&self.scroll)
            .on_scroll(move |_, window, cx| {
                moved.motion.borrow_mut().slide = None;
                cx.stop_propagation();
                let moved = moved.clone();
                // Retained scroll layers can move without rendering this view again.
                window.on_next_frame(move |window, _| {
                    if moved.changed() {
                        window.refresh();
                    }
                });
            });
        crate::glass::plate(radius::LARGE)
            .p(px(4.))
            .min_w_0()
            .flex_shrink(1.)
            .overflow_hidden()
            .child(tabs)
            .children((left_alpha > 0.).then(|| self.arrow(false, left_alpha, left, t)))
            .children((right_alpha > 0.).then(|| self.arrow(true, right_alpha, right, t)))
    }

    fn arrow(&self, right: bool, alpha: f32, enabled: bool, t: &Theme) -> Div {
        let strip = self.clone();
        let button = div()
            .id(if right {
                "nav-scroll-right"
            } else {
                "nav-scroll-left"
            })
            .flex_none()
            .size(px(ARROW))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(radius::CONTROL))
            .bg(ui::white(fill::BUTTON))
            .child(
                ui::icon("arrow-right-01", 18., ui::white(ink::FULL)).when(!right, |glyph| {
                    glyph.with_transformation(gpui::Transformation::rotate(gpui::radians(
                        std::f32::consts::PI,
                    )))
                }),
            );
        let button = if enabled {
            crate::keyboard(button, t)
                .block_mouse_except_scroll()
                .hover(|s| s.bg(ui::white(fill::BUTTON_HOVER)))
                .cursor_pointer()
                .on_click(move |_, window, cx| {
                    let now = Instant::now();
                    strip.motion.borrow_mut().advance(
                        strip.scroll.offset().x.into(),
                        strip.scroll.max_offset().x.into(),
                        strip.scroll.bounds().size.width.into(),
                        right,
                        now,
                    );
                    if strip.instant.get() {
                        strip.slide(now);
                    } else {
                        strip.animate(window);
                    }
                    cx.stop_propagation();
                    window.refresh();
                })
        } else {
            // Keep painting the fade-out without an invisible focus or click target: clicks
            // reach the tab under it.
            button
        };
        // The plate's own colour: its solid fill in Performance mode, else what the glass is
        // tinted with.
        let shade = crate::glass::plain_fill().unwrap_or_else(|| {
            if ui::light() {
                gpui::white()
            } else {
                ui::color(ui::look().base)
            }
        });
        div()
            .absolute()
            .top(px(4.))
            .bottom(px(4.))
            .w(px(EDGE))
            .flex()
            .items_center()
            // Square: the strip cuts the tabs straight at its edges (GPUI clips to rectangles),
            // and this hides the cut, a selected tab's included.
            .when(right, |d| d.right(px(4.)).justify_end())
            .when(!right, |d| d.left(px(4.)))
            .opacity(alpha)
            .bg(gpui::linear_gradient(
                if right { 270. } else { 90. },
                gpui::linear_color_stop(shade.opacity(0.98), 0.6),
                gpui::linear_color_stop(shade.opacity(0.), 1.),
            ))
            .child(button)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_click_glides_and_repeated_or_reversed_clicks_start_where_the_tabs_are() {
        let now = Instant::now();
        let mut state = Motion::default();
        state.advance(0., 600., 200., true, now);
        assert_eq!(state.step(now, 600., false), Some(0.));
        let midway = now + Duration::from_millis(60);
        let visible = state.step(midway, 600., false).unwrap();
        assert!(visible < 0. && visible > -150.);
        state.advance(visible, 600., 200., true, midway);
        assert_eq!(
            state.step(midway, 600., false),
            Some(visible),
            "no jump on another click"
        );
        assert_eq!(
            state.slide.unwrap().to,
            -300.,
            "continue towards the next group"
        );
        state.advance(visible, 600., 200., false, midway);
        assert_eq!(state.step(midway, 600., false), Some(visible));
        let reversed = state
            .step(midway + Duration::from_millis(60), 600., false)
            .unwrap();
        assert!(reversed > visible && reversed < 0., "turn back immediately");
        assert_eq!(
            state.step(midway + horizon_theme::MOVE.duration(), 600., false),
            Some(0.)
        );
        assert!(state.slide.is_none());

        state.advance(0., 600., 200., true, now);
        assert_eq!(
            state.step(now, 600., true),
            Some(-150.),
            "reduced motion jumps to the end"
        );
        assert!(state.slide.is_none());
        state.advance(0., 600., 200., true, now);
        assert_eq!(
            state.step(midway, 0., false),
            Some(0.),
            "resizing to fit cancels the glide"
        );
        assert!(state.slide.is_none());
    }

    #[test]
    fn cues_fade_both_ways_and_reverse_without_a_flash() {
        let now = Instant::now();
        let mut fade = Fade::default();
        assert_eq!(fade.show(true, now, false), 0.);
        let midway = now + Duration::from_millis(75);
        let visible = fade.show(true, midway, false);
        assert!(visible > 0. && visible < 1.);
        assert_eq!(
            fade.show(false, midway, false),
            visible,
            "fade out from the current opacity"
        );
        let later = midway + Duration::from_millis(40);
        let dimmer = fade.sample(later).0;
        assert!(dimmer < visible && dimmer > 0.);
        assert_eq!(
            fade.show(true, later, false),
            dimmer,
            "fade back in without flashing"
        );
        assert_eq!(
            fade.sample(later + horizon_theme::QUICK.duration()),
            (1., false)
        );
        assert_eq!(
            fade.show(false, later, true),
            0.,
            "reduced motion hides at once"
        );
        assert_eq!(fade.sample(later), (0., false));
    }

    #[test]
    fn arrows_reveal_more_tabs_and_disappear_at_each_end_or_when_the_tabs_fit() {
        assert_eq!(edges(0., 420.), (false, true));
        let mut offset = 0.;
        for expected in [-150., -300., -420.] {
            offset = next_offset(offset, 420., 200., true);
            assert_eq!(offset, expected);
            assert_eq!(edges(offset, 420.), (true, expected > -420.));
        }
        assert_eq!(next_offset(offset, 420., 200., true), -420.);
        for expected in [-270., -120., 0.] {
            offset = next_offset(offset, 420., 200., false);
            assert_eq!(offset, expected);
        }
        assert_eq!(edges(offset, 420.), (false, true));
        assert_eq!(edges(-500., 420.), (true, false), "wheel past the end");
        assert_eq!(edges(20., 420.), (false, true), "wheel past the start");
        assert_eq!(
            edges(-300., 0.),
            (false, false),
            "a wider window fits all tabs"
        );
        assert_eq!(next_offset(-300., 0., 800., false), 0.);
    }
}
