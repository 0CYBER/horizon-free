//! Long lists in the page's scroll area (the hundred releases on Versions and Patch notes) draw
//! only the rows near what is on screen. A row far off screen becomes an empty block as tall as
//! it was the last time it was drawn, so the page keeps its length and the scrollbar its place.
use gpui::{div, px, Bounds, Div, IntoElement, ParentElement, Pixels, ScrollHandle, Styled};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone, Default)]
pub struct Cull(Rc<RefCell<Drawn>>);

/// Where each row was in the last frame, and the scroll offset and the viewport's width then
/// (a narrower window wraps text, and the heights no longer hold).
#[derive(Default)]
struct Drawn {
    rows: Vec<Bounds<Pixels>>,
    offset: Pixels,
    width: Pixels,
}

impl Cull {
    /// A column for `rows` rows that remembers where they were drawn. Rows come from `row(i)`, or
    /// an empty block when row `i` was more than a screen away from the viewport last frame and
    /// `keep(i)` is false (a row being installed or showing an error is always drawn).
    pub fn column<E: IntoElement>(
        &self,
        handle: &ScrollHandle,
        rows: usize,
        gap: f32,
        keep: impl Fn(usize) -> bool,
        mut row: impl FnMut(usize) -> E,
    ) -> Div {
        let far = self.far(handle, rows);
        let skipped: Vec<Option<Pixels>> = (0..rows).map(|i| far(i).filter(|_| !keep(i))).collect();
        let gaps = skipped.iter().any(Option::is_some);
        let state = self.0.clone();
        let scroll = handle.clone();
        div()
            .flex()
            .flex_col()
            .children(skipped.iter().enumerate().map(|(i, skip)| match skip {
                Some(height) => div().flex_none().h(*height).mb(px(gap)).into_any_element(),
                None => row(i).into_any_element(),
            }))
            .on_children_prepainted(move |bounds, window, _| {
                let view = scroll.bounds();
                // A jump of more than a screen (the scrollbar dragged) can bring an empty block
                // into view: draw again, with the rows where they are now.
                if gaps
                    && bounds
                        .iter()
                        .zip(&skipped)
                        .any(|(b, skip)| skip.is_some() && !away(*b, px(0.), view))
                {
                    window.refresh();
                }
                let mut drawn = state.borrow_mut();
                drawn.rows = bounds;
                drawn.offset = scroll.offset().y;
                drawn.width = view.size.width;
            })
    }

    /// For row `i`: its height when it was more than a screen away from the viewport.
    fn far(&self, handle: &ScrollHandle, rows: usize) -> impl Fn(usize) -> Option<Pixels> {
        let drawn = self.0.borrow();
        let view = handle.bounds();
        // The rows moved by however far the page scrolled since they were drawn.
        let moved = handle.offset().y - drawn.offset;
        let known =
            drawn.rows.len() == rows && view.size.height > px(0.) && drawn.width == view.size.width;
        let rows = if known {
            drawn.rows.clone()
        } else {
            Vec::new()
        };
        move |i| {
            let b = *rows.get(i)?;
            away(b, moved, view).then_some(b.size.height)
        }
    }
}

/// Whether a row drawn at `row`, since moved down by `moved`, is more than a screen above or
/// below `view`.
fn away(row: Bounds<Pixels>, moved: Pixels, view: Bounds<Pixels>) -> bool {
    row.bottom() + moved < view.top() - view.size.height
        || row.top() + moved > view.bottom() + view.size.height
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, size};

    fn at(y: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(0.), px(y)), size(px(10.), px(h)))
    }

    #[test]
    fn only_rows_more_than_a_screen_away_are_skipped() {
        let view = at(100., 500.);
        assert!(!away(at(100., 100.), px(0.), view));
        assert!(!away(at(1000., 100.), px(0.), view));
        assert!(away(at(1200., 100.), px(0.), view));
        assert!(away(at(-600., 100.), px(0.), view));
        // Scrolled 700 px down since: the row that was far below is now on screen.
        assert!(!away(at(1200., 100.), px(-700.), view));
    }

    #[test]
    fn nothing_is_skipped_before_the_rows_are_known() {
        let cull = Cull::default();
        cull.0.borrow_mut().rows = (0..10).map(|i| at(i as f32 * 2000., 100.)).collect();
        // No viewport yet (the first frame), or another number of rows: every row is drawn.
        let far = cull.far(&ScrollHandle::new(), 10);
        assert!((0..10).all(|i| far(i).is_none()));
        let far = cull.far(&ScrollHandle::new(), 11);
        assert!((0..11).all(|i| far(i).is_none()));
    }
}
