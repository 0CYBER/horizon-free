//! The clip stack of one GUI drawing hook call: `DrawContext::push_clip` and `pop_clip`
//! (`GuiGraphics.enableScissor`, `disableScissor`), cut on the batch as the game's own
//! screens cut theirs (`HudBatch::clip_since`).
use crate::hud::HudBatch;
use mod_api::GuiClipRefusal;

/// The most clips one hook call holds open at once.
pub(crate) const MAX_OPEN: usize = 64;

/// One hook call's open clips: where each began in the batch's vertices and its rectangle
/// (x0, y0, x1, y1), already cut to the clips around it. Empty, and unallocated, until a mod
/// clips, so a call that never clips costs nothing.
#[derive(Default)]
pub(crate) struct GuiClips(Vec<(usize, [f32; 4])>);

impl GuiClips {
    /// Opens a clip of `rect` (`[x, y, w, h]`, GUI pixels) at the batch's next vertex.
    pub(crate) fn push(&mut self, batch: &HudBatch, rect: [f32; 4]) -> Result<(), GuiClipRefusal> {
        if !rect.iter().all(|v| v.is_finite()) {
            return Err(GuiClipRefusal::NotFinite);
        }
        let [x, y, w, h] = rect;
        let (x1, y1) = (x + w.max(0.0), y + h.max(0.0));
        if ![x1, y1].iter().all(|v| v.is_finite()) {
            return Err(GuiClipRefusal::NotFinite);
        }
        if self.0.len() >= MAX_OPEN {
            return Err(GuiClipRefusal::TooDeep);
        }
        let mut clip = [x, y, x1, y1];
        if let Some(&(_, outer)) = self.0.last() {
            clip = [
                clip[0].max(outer[0]),
                clip[1].max(outer[1]),
                clip[2].min(outer[2]),
                clip[3].min(outer[3]),
            ];
            // Disjoint from the outer clip: an empty one, clipping everything.
            clip[2] = clip[2].max(clip[0]);
            clip[3] = clip[3].max(clip[1]);
        }
        self.0.push((batch.vertices.len(), clip));
        Ok(())
    }
    /// Closes the newest clip, cutting what was drawn since it opened.
    pub(crate) fn pop(&mut self, batch: &mut HudBatch) -> Result<(), GuiClipRefusal> {
        let (start, [x0, y0, x1, y1]) = self.0.pop().ok_or(GuiClipRefusal::NoneOpen)?;
        batch.clip_since(start, x0, y0, x1, y1);
        Ok(())
    }
    /// Closes every clip still open: the end of the hook call.
    pub(crate) fn close_all(&mut self, batch: &mut HudBatch) {
        while self.pop(batch).is_ok() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(batch: &mut HudBatch, x: f32, y: f32, size: f32) {
        let p = [[x, y], [x + size, y], [x + size, y + size], [x, y + size]];
        batch.quad(
            p,
            [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            u32::MAX,
        );
    }
    fn bounds(batch: &HudBatch) -> [f32; 4] {
        batch.vertices.iter().fold(
            [
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ],
            |[a, b, c, d], v| {
                [
                    a.min(v.pos[0]),
                    b.min(v.pos[1]),
                    c.max(v.pos[0]),
                    d.max(v.pos[1]),
                ]
            },
        )
    }

    #[test]
    fn a_clip_cuts_only_what_was_drawn_inside_it() {
        let mut batch = HudBatch::scratch([640, 480]);
        let mut clips = GuiClips::default();
        square(&mut batch, 0.0, 0.0, 300.0);
        let before: Vec<_> = batch.vertices.iter().map(|v| v.pos).collect();
        clips.push(&batch, [10.0, 10.0, 100.0, 100.0]).unwrap();
        square(&mut batch, 0.0, 0.0, 300.0);
        clips.pop(&mut batch).unwrap();
        let kept: Vec<_> = batch.vertices[..before.len()]
            .iter()
            .map(|v| v.pos)
            .collect();
        assert_eq!(kept, before, "drawn before the clip");
        let mut inside = HudBatch::scratch([640, 480]);
        inside.vertices = batch.vertices[before.len()..].to_vec();
        assert_eq!(bounds(&inside), [10.0, 10.0, 110.0, 110.0]);
        square(&mut batch, 0.0, 0.0, 300.0);
        assert_eq!(
            bounds(&batch),
            [0.0, 0.0, 300.0, 300.0],
            "drawn after the pop"
        );
    }

    #[test]
    fn nested_clips_intersect_and_a_disjoint_one_clips_everything() {
        let mut batch = HudBatch::scratch([640, 480]);
        let mut clips = GuiClips::default();
        clips.push(&batch, [0.0, 0.0, 100.0, 100.0]).unwrap();
        clips.push(&batch, [50.0, 50.0, 200.0, 200.0]).unwrap();
        square(&mut batch, 0.0, 0.0, 300.0);
        clips.pop(&mut batch).unwrap();
        clips.pop(&mut batch).unwrap();
        assert_eq!(bounds(&batch), [50.0, 50.0, 100.0, 100.0]);

        let mut batch = HudBatch::scratch([640, 480]);
        clips.push(&batch, [0.0, 0.0, 10.0, 10.0]).unwrap();
        clips.push(&batch, [20.0, 20.0, 10.0, 10.0]).unwrap();
        square(&mut batch, 0.0, 0.0, 300.0);
        clips.close_all(&mut batch);
        assert!(batch.vertices.is_empty(), "{:?}", batch.vertices.len());
        clips.push(&batch, [0.0, 0.0, -5.0, 10.0]).unwrap();
        square(&mut batch, 0.0, 0.0, 300.0);
        clips.pop(&mut batch).unwrap();
        assert!(
            batch.vertices.is_empty(),
            "a negative width clips everything"
        );
    }

    #[test]
    fn refusals_are_named_and_leave_the_stack_as_it_was() {
        let mut batch = HudBatch::scratch([640, 480]);
        let mut clips = GuiClips::default();
        assert_eq!(clips.pop(&mut batch), Err(GuiClipRefusal::NoneOpen));
        for rect in [
            [f32::NAN, 0.0, 1.0, 1.0],
            [0.0, 0.0, f32::INFINITY, 1.0],
            [0.0, 0.0, f32::NAN, 1.0],
            [0.0, 0.0, f32::NEG_INFINITY, 1.0],
            [0.0, 0.0, 1.0, f32::NAN],
            [0.0, 0.0, 1.0, f32::NEG_INFINITY],
            [f32::MAX, 0.0, f32::MAX, 1.0],
        ] {
            assert_eq!(clips.push(&batch, rect), Err(GuiClipRefusal::NotFinite));
        }
        assert!(clips.0.is_empty());
        for _ in 0..MAX_OPEN {
            clips.push(&batch, [0.0, 0.0, 10.0, 10.0]).unwrap();
        }
        assert_eq!(
            clips.push(&batch, [0.0, 0.0, 10.0, 10.0]),
            Err(GuiClipRefusal::TooDeep)
        );
        assert_eq!(clips.0.len(), MAX_OPEN);
        clips.close_all(&mut batch);
        assert!(clips.0.is_empty());
    }
}

/// Through the game's generated dispatch: the context a drawing hook gets clips what the mod
/// draws, names its refusals, and closes a clip the mod left open when the call returns.
#[cfg(test)]
mod dispatch_tests {
    use horizon_client::hud::HudBatch;
    use horizon_client::text::{Style, Text};
    use mod_api::{ComponentHoverEvent, GuiClipRefusal, Hooks, ModEntry};
    use mod_platform::testing::with_mods;
    use std::sync::Mutex;

    /// What `pop_clip` answered past the last open clip, each call.
    static REFUSED: Mutex<Vec<Result<(), GuiClipRefusal>>> = Mutex::new(Vec::new());

    /// Draws a large square inside a closed clip, and another inside one it leaves open.
    struct Clipper;
    #[mod_api::__subscriptions]
    impl Hooks for Clipper {
        fn component_hover(
            &self,
            _: &dyn mod_api::ClientHost,
            event: &mut ComponentHoverEvent<'_>,
        ) -> mod_api::Flow {
            event.draw.push_clip([10.0, 10.0, 5.0, 5.0]).unwrap();
            event.draw.fill([0.0, 0.0, 100.0, 100.0], 0xff00ff00);
            event.draw.pop_clip().unwrap();
            REFUSED.lock().unwrap().push(event.draw.pop_clip());
            event.draw.push_clip([20.0, 20.0, 4.0, 4.0]).unwrap();
            event.draw.fill([0.0, 0.0, 100.0, 100.0], 0xff0000ff);
            mod_api::Flow::Cancel
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_hook_call_clips_what_its_mod_draws_and_closes_what_it_leaves_open() {
        let text = Text::from(serde_json::json!({
            "text": "x", "hover_event": { "action": "show_text", "value": "hi" }
        }));
        let style: Style = text.spans[0].style.clone();
        let mods: mod_api::Mods = vec![ModEntry::new("clipper", Box::new(Clipper))].into();
        let mut batch = HudBatch::scratch([320, 240]);
        with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                game.component_hover_effect(&mut batch, &style, (40.0, 30.0));
            });
        });
        assert_eq!(*REFUSED.lock().unwrap(), [Err(GuiClipRefusal::NoneOpen)]);
        let inside = |color: u32, [x0, y0, x1, y1]: [f32; 4]| {
            let quads: Vec<_> = batch.vertices.iter().filter(|v| v.color == color).collect();
            assert_eq!(quads.len(), 6, "{color:x}");
            assert!(
                quads
                    .iter()
                    .all(|v| (x0..=x1).contains(&v.pos[0]) && (y0..=y1).contains(&v.pos[1]))
            );
        };
        inside(batch.vertices[0].color, [10.0, 10.0, 15.0, 15.0]);
        inside(batch.vertices[6].color, [20.0, 20.0, 24.0, 24.0]);
    }
}
