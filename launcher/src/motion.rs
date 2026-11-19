//! The launcher's motion, as the website's launcher plays it (horizon-site's `launcher.css` and
//! Tailwind's defaults): hovers ease over the design's quick curve (150 ms,
//! `cubic-bezier(0.4, 0, 0.2, 1)`); toasts and a new page arrive as `hzl-pop` does (a fade and a
//! 6 px rise over 200 ms); a switch's knob slides for 200 ms; a mouse wheel's notch glides over
//! `move` ([`GLIDE`], [`crate::glide`]). Under reduced motion
//! ([`crate::sky::motion`]) nothing moves: everything is drawn where it ends. Menus open at once:
//! an animation inside a `deferred` menu kept its first frame (invisible) under GPUI Fast.
use gpui::{px, Animation, AnimationExt, AnyElement, Div, ElementId, IntoElement, Styled};
use horizon_theme as tokens;
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Instant;

/// `hzl-pop`: `0.2s cubic-bezier(0.2, 0.7, 0.2, 1)`, from transparent and 6 px lower.
pub const POP: tokens::Motion = tokens::Motion {
    name: "pop",
    ms: 200,
    curve: [0.2, 0.7, 0.2, 1.],
};
pub const POP_RISE: f32 = 6.;

/// A new page: as [`POP`], a little longer and from further down, so the switch reads.
pub const PAGE: tokens::Motion = tokens::Motion {
    name: "page",
    ms: 260,
    curve: [0.2, 0.7, 0.2, 1.],
};
pub const PAGE_RISE: f32 = 12.;

/// A switch's knob (`transition-[translate] duration-200`, Tailwind's default curve).
const SLIDE: tokens::Motion = tokens::Motion {
    name: "slide",
    ms: 200,
    curve: tokens::QUICK.curve,
};

/// A mouse wheel's notch: `move`, the design's ease-out with a long tail (240 ms), so most of the
/// notch is there within a few frames.
pub const GLIDE: tokens::Motion = tokens::MOVE;

/// The site's card lift: a spring that settles from a little past where it lands
/// (`cubic-bezier(.22,.9,.3,1.24)`; `bezier` does not clamp y, so the overshoot plays).
pub const SPRING: tokens::Motion = tokens::Motion {
    name: "spring",
    ms: 260,
    curve: [0.22, 0.9, 0.3, 1.24],
};

/// How far a lifting card rises, and how far a pressed control sinks, in px.
pub const LIFT: f32 = 3.;
pub const PRESS: f32 = 1.;

/// Whether the player's system asks for reduced motion (or `HORIZON_LAUNCHER_MOTION=off`).
pub fn still() -> bool {
    crate::sky::motion() == crate::sky::Motion::Still
}

/// `motion` as GPUI plays it: its length on its own curve; at its end from the first frame under
/// reduced motion.
pub fn eased(motion: tokens::Motion) -> Animation {
    eased_as(motion, still())
}

fn eased_as(motion: tokens::Motion, still: bool) -> Animation {
    let animation = Animation::new(motion.duration());
    if still {
        return animation.with_easing(|_| 1.);
    }
    animation.with_easing(move |t| motion.ease(t))
}

thread_local! {
    /// Each switch's last state, and when it last changed.
    static SWITCHES: RefCell<HashMap<String, (bool, Option<Instant>)>> = RefCell::new(HashMap::new());
}

/// When switch `id`'s knob started sliding towards `on`, while it still slides: a switch seen
/// for the first time, or changed long ago, is already there.
fn slide(id: &str, on: bool, now: Instant) -> Option<Instant> {
    SWITCHES.with_borrow_mut(|switches| {
        let seen = switches.entry(id.to_string()).or_insert((on, None));
        if seen.0 != on {
            *seen = (on, Some(now));
        }
        seen.1
            .filter(|since| now.duration_since(*since) < SLIDE.duration())
    })
}

/// A switch's knob at `left` px when off and `left + travel` when on, sliding there when the
/// switch `id` changed in the last 200 ms.
pub fn knob(id: &str, on: bool, knob: Div, left: f32, travel: f32) -> AnyElement {
    let at = move |p: f32| left + travel * if on { p } else { 1. - p };
    match slide(id, on, Instant::now()) {
        Some(since) => {
            // Each slide gets its own animation id, from when it started, so it plays once.
            static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
            let start = *START.get_or_init(Instant::now);
            let key = since.saturating_duration_since(start).as_micros() as usize;
            knob.with_animation(
                ElementId::named_usize(format!("{id}-knob"), key),
                eased(SLIDE),
                move |d, p| d.left(px(at(p))),
            )
            .into_any_element()
        }
        None => knob.left(px(at(1.))).into_any_element(),
    }
}

/// How long each later item of a list waits before it comes in ([`enter`]), and the most it waits.
const STAGGER_MS: u64 = 40;
const STAGGER_MAX: usize = 8;

/// `el` coming in as a new page does (a fade and a rise, [`PAGE`]), `order` items after the first
/// of its list, so a list or a column of cards cascades in. `key` names this entrance: it plays
/// once per key, so put what should replay it (the page switch count, the tab) in the key. Under
/// reduced motion it is simply there.
pub fn enter(key: impl Into<ElementId>, order: usize, el: Div) -> AnyElement {
    if still() {
        return el.relative().into_any_element();
    }
    let delay = STAGGER_MS * order.min(STAGGER_MAX) as u64;
    let total = PAGE.ms + delay;
    let start = delay as f32 / total as f32;
    let animation = Animation::new(std::time::Duration::from_millis(total)).with_easing(move |t| {
        if t <= start {
            0.
        } else {
            PAGE.ease((t - start) / (1. - start))
        }
    });
    el.relative()
        .with_animation(key, animation, |d, p| {
            d.opacity(p).top(px(PAGE_RISE * (1. - p)))
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn reduced_motion_draws_the_end_from_the_first_frame() {
        let still = eased_as(PAGE, true);
        assert_eq!((still.easing)(0.), 1.);
        let moving = eased_as(PAGE, false);
        assert_eq!((moving.easing)(0.), 0.);
        assert!(((moving.easing)(1.) - 1.).abs() < 1e-3);
    }

    #[test]
    fn a_switch_slides_only_after_it_changed() {
        let t0 = Instant::now();
        assert_eq!(slide("test-a", true, t0), None, "first seen: already there");
        assert_eq!(slide("test-a", true, t0 + Duration::from_millis(50)), None);
        let flip = t0 + Duration::from_millis(100);
        assert_eq!(slide("test-a", false, flip), Some(flip));
        assert_eq!(
            slide("test-a", false, flip + Duration::from_millis(150)),
            Some(flip),
            "the same slide while it runs"
        );
        assert_eq!(
            slide("test-a", false, flip + Duration::from_millis(250)),
            None
        );
    }
}
