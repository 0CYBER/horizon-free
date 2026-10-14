//! The labelled buttons drawn in the last frame, for the control socket: a
//! script clicks a button by its label (`horizon-ctl click-button Respawn`)
//! whatever screen or menu style drew it.
use std::cell::RefCell;

/// A button in GUI pixels (the coordinates screens take input in).
#[derive(Clone, Debug)]
pub struct Widget {
    pub label: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub enabled: bool,
}

/// What a controller does with a focus target (docs/controllers.md, "Focus
/// navigation").
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// Clicked with A (buttons, switches, segments, sidebar items).
    Button,
    /// Left and right move the value instead of focus; A does nothing (a
    /// click would jump the handle to the middle). `value` is 0..1 as drawn;
    /// `track` is where a click sets 0 and the width a click spans to 1.
    Slider { value: f32, track: [f32; 2] },
    /// A text field: A focuses it and opens an on-screen keyboard.
    Field,
    /// A list row (worlds, servers, packs, languages): A selects it.
    Row,
    /// An inventory slot: A/X/Y click, right click and quick-move it.
    Slot,
    /// A tab (`true` is the selected one): the bumpers step through them.
    Tab(bool),
}

/// A focus target: a rectangle in GUI pixels and what it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub rect: [f32; 4],
    pub kind: Kind,
}
impl Target {
    pub fn center(&self) -> (f32, f32) {
        let [x, y, w, h] = self.rect;
        (x + w / 2.0, y + h / 2.0)
    }
}

thread_local! {
    /// Buttons drawn since the last `end_frame`, and the last frame's.
    static WIDGETS: RefCell<(Vec<Widget>, Vec<Widget>, u64)> = const { RefCell::new((Vec::new(), Vec::new(), 0)) };
    /// Every interactive rectangle (Horizon-style menus' sidebar items,
    /// switches, sliders, segments; classic sliders, fields and list rows),
    /// labelled or not: where a controller can move focus. Drawn since
    /// `end_frame`, the last frame's, and how many frames rotated.
    static TARGETS: RefCell<(Vec<Target>, Vec<Target>, u64)> = const { RefCell::new((Vec::new(), Vec::new(), 0)) };
}

/// An interactive rectangle (GUI pixels) a controller can focus and click.
pub fn target(rect: [f32; 4]) {
    target_as(rect, Kind::Button);
}

/// An interactive rectangle of a given kind.
pub fn target_as(rect: [f32; 4], kind: Kind) {
    if !rect.iter().all(|v| v.is_finite()) || rect[2] <= 0.0 || rect[3] <= 0.0 {
        return;
    }
    TARGETS.with_borrow_mut(|(current, _, _)| {
        if current.len() < 1024 {
            current.push(Target { rect, kind });
        }
    });
}

/// How many targets the frame has so far, for [`clip_targets`].
pub fn targets_mark() -> usize {
    TARGETS.with_borrow(|(current, _, _)| current.len())
}

/// Drop the targets registered since `mark` that aren't inside `y0..y1`
/// (GUI pixels): rows a scrolling list clipped away.
pub fn clip_targets(mark: usize, y0: f32, y1: f32) {
    TARGETS.with_borrow_mut(|(current, _, _)| {
        let mut i = mark.min(current.len());
        while i < current.len() {
            let [_, y, _, h] = current[i].rect;
            if y < y0 - 0.5 || y + h > y1 + 0.5 {
                current.remove(i);
            } else {
                i += 1;
            }
        }
    });
}

/// The focus targets of the last drawn frame.
pub fn last_targets() -> Vec<Target> {
    TARGETS.with_borrow(|(_, last, _)| last.clone())
}

/// Counts drawn frames: focus waits for a new one after scrolling a list.
pub fn generation() -> u64 {
    TARGETS.with_borrow(|(_, _, generation)| *generation)
}

/// Called by the button drawers.
pub fn record(label: &str, [x, y, w, h]: [f32; 4], enabled: bool) {
    let label = label.trim();
    if label.is_empty() {
        return;
    }
    WIDGETS.with_borrow_mut(|(current, _, _)| {
        if current.len() < 512 {
            current.push(Widget {
                label: label.to_owned(),
                x,
                y,
                w,
                h,
                enabled,
            });
        }
    });
}

/// Once per event loop turn with the count of rendered frames: when a new
/// frame was drawn since the last call, its buttons become the last frame's.
pub fn end_frame(frames: u64) {
    WIDGETS.with_borrow_mut(|(current, last, seen)| {
        if *seen != frames {
            *seen = frames;
            *last = std::mem::take(current);
            TARGETS.with_borrow_mut(|(current, last, generation)| {
                *last = std::mem::take(current);
                *generation += 1;
            });
        }
    });
}

/// The buttons of the last drawn frame, top-most last.
pub fn last() -> Vec<Widget> {
    WIDGETS.with_borrow(|(_, last, _)| last.clone())
}

/// The button `label` names: an exact match (case-insensitive), else the only
/// button whose label contains it. The top-most wins among equals.
pub fn find(widgets: &[Widget], label: &str) -> Result<Widget, String> {
    let want = label.trim().to_lowercase();
    let exact: Vec<&Widget> = widgets
        .iter()
        .filter(|w| w.label.to_lowercase() == want)
        .collect();
    if let Some(w) = exact.last() {
        return Ok((*w).clone());
    }
    let partial: Vec<&Widget> = widgets
        .iter()
        .filter(|w| w.label.to_lowercase().contains(&want))
        .collect();
    match partial.as_slice() {
        [w] => Ok((*w).clone()),
        [] => Err(format!(
            "no button labelled {label:?}; buttons: {:?}",
            widgets.iter().map(|w| w.label.as_str()).collect::<Vec<_>>()
        )),
        many => Err(format!(
            "{label:?} matches several buttons: {:?}",
            many.iter().map(|w| w.label.as_str()).collect::<Vec<_>>()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(label: &str) -> Widget {
        Widget {
            label: label.into(),
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
            enabled: true,
        }
    }

    #[test]
    fn finds_exact_then_unique_partial() {
        let list = [w("Play Selected World"), w("Create New World"), w("Back")];
        assert_eq!(find(&list, "back").unwrap().label, "Back");
        assert_eq!(find(&list, "create").unwrap().label, "Create New World");
        assert!(find(&list, "world").is_err());
        assert!(find(&list, "options").is_err());
    }

    #[test]
    fn frames_rotate_only_when_drawn() {
        record("Respawn", [1.0, 2.0, 3.0, 4.0], true);
        target_as(
            [1.0, 2.0, 3.0, 4.0],
            Kind::Slider {
                value: 0.5,
                track: [1.0, 3.0],
            },
        );
        target_as([1.0, 2.0, 0.0, 4.0], Kind::Row);
        let before = generation();
        end_frame(1);
        assert_eq!(last().len(), 1);
        assert_eq!(last_targets().len(), 1, "an empty rectangle isn't a target");
        assert!(matches!(last_targets()[0].kind, Kind::Slider { value, .. } if value == 0.5));
        assert_eq!(last_targets()[0].center(), (2.5, 4.0));
        end_frame(1);
        assert_eq!(last().len(), 1);
        assert_eq!(generation(), before + 1);
        end_frame(2);
        assert!(last().is_empty() && last_targets().is_empty());
        target([0.0, 0.0, 5.0, 5.0]);
        let mark = targets_mark();
        target([0.0, 10.0, 5.0, 5.0]);
        target([0.0, 40.0, 5.0, 5.0]);
        clip_targets(mark, 8.0, 30.0);
        end_frame(3);
        assert_eq!(
            last_targets().len(),
            2,
            "the clipped row is gone, earlier ones stay"
        );
    }
}
