use super::*;
use mod_api::{DrawContext, GuiClipRefusal};

#[test]
fn gui_clip_refusals_round_trip_by_ordinal() {
    for (refusal, ordinal) in [
        (GuiClipRefusal::NotFinite, 0_u32),
        (GuiClipRefusal::TooDeep, 1),
        (GuiClipRefusal::NoneOpen, 2),
    ] {
        let mut out = Writer::new();
        out.put(&refusal);
        assert_eq!(out.bytes(), ordinal.to_le_bytes());
        assert_eq!(
            Reader::decode::<GuiClipRefusal>(out.bytes()).unwrap(),
            refusal
        );
    }
}

/// The game's side: records each clip it is asked for and pops what it holds, refusing as the
/// game does.
#[derive(Default)]
struct Draw {
    open: Vec<[f32; 4]>,
    asked: Vec<[f32; 4]>,
}
impl DrawContext for Draw {
    fn push_clip(&mut self, rect: [f32; 4]) -> Result<(), GuiClipRefusal> {
        self.asked.push(rect);
        if !rect.iter().all(|v| v.is_finite()) {
            return Err(GuiClipRefusal::NotFinite);
        }
        self.open.push(rect);
        Ok(())
    }
    fn pop_clip(&mut self) -> Result<(), GuiClipRefusal> {
        self.open.pop().map(drop).ok_or(GuiClipRefusal::NoneOpen)
    }
}

#[test]
fn wire_loopback_a_guest_clip_reaches_the_draw_context_and_its_refusals_come_back() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut draw = Draw::default();
    let mut cx = Box::new(boundary::Cx::default());
    let mut frame = boundary::Frame::default();
    frame.set_draw_context_mut(&mut draw);
    cx.frames.push(frame);
    let pointer = &mut *cx as *mut boundary::Cx;
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        // SAFETY: the context and its frame outlive this synchronous transport.
        unsafe { boundary::serve(id, pointer, bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    assert_eq!(guest::Guest.push_clip([1.0, 2.0, 30.0, 40.0]), Ok(()));
    assert_eq!(
        guest::Guest.push_clip([f32::NAN, 0.0, 1.0, 1.0]),
        Err(GuiClipRefusal::NotFinite)
    );
    assert_eq!(guest::Guest.pop_clip(), Ok(()));
    assert_eq!(guest::Guest.pop_clip(), Err(GuiClipRefusal::NoneOpen));
    guest::set_transport(None);
    drop(cx);
    assert!(draw.open.is_empty());
    assert_eq!(draw.asked[0], [1.0, 2.0, 30.0, 40.0]);
    assert!(draw.asked[1][0].is_nan());
}
