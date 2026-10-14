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

thread_local! {
    /// Buttons drawn since the last `end_frame`, and the last frame's.
    static WIDGETS: RefCell<(Vec<Widget>, Vec<Widget>, u64)> = const { RefCell::new((Vec::new(), Vec::new(), 0)) };
    /// Every interactive rectangle of Horizon-style menus (sidebar items,
    /// switches, sliders, segments), labelled or not: where a controller's
    /// D-pad can move focus. Drawn since `end_frame`, and the last frame's.
    static TARGETS: RefCell<(Vec<[f32; 4]>, Vec<[f32; 4]>)> = const { RefCell::new((Vec::new(), Vec::new())) };
}

/// An interactive rectangle (GUI pixels) a controller can focus.
pub fn target(rect: [f32; 4]) {
    TARGETS.with_borrow_mut(|(current, _)| {
        if current.len() < 1024 {
            current.push(rect);
        }
    });
}

/// The focus targets of the last drawn frame.
pub fn last_targets() -> Vec<[f32; 4]> {
    TARGETS.with_borrow(|(_, last)| last.clone())
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
            TARGETS.with_borrow_mut(|(current, last)| *last = std::mem::take(current));
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
        end_frame(1);
        assert_eq!(last().len(), 1);
        end_frame(1);
        assert_eq!(last().len(), 1);
        end_frame(2);
        assert!(last().is_empty());
    }
}
