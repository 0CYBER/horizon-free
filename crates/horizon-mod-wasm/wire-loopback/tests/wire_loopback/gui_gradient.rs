use super::*;
use mod_api::DrawContext;
use mod_api::client::hud_layout::{GuiDraw, PoseOp};

/// The game's side: records each gradient and each painted buffer it is asked for.
#[derive(Default)]
struct Draw {
    gradients: Vec<([f32; 4], u32, u32)>,
    painted: Vec<Vec<GuiDraw>>,
}
impl DrawContext for Draw {
    fn fill_gradient(&mut self, rect: [f32; 4], top: u32, bottom: u32) {
        self.gradients.push((rect, top, bottom));
    }
    fn gui_paint(&mut self, draws: &[GuiDraw]) -> Result<(), mod_api::text::TextFormError> {
        self.painted.push(draws.to_vec());
        Ok(())
    }
}

fn gradient() -> GuiDraw {
    GuiDraw::FillGradient {
        x0: -3,
        y0: 4,
        x1: 50,
        y1: -6,
        top: 0x80FF_0000,
        bottom: 0x0000_00FF,
    }
}

#[test]
fn a_fill_gradient_op_round_trips_on_the_wire_field_for_field() {
    let draws = vec![
        GuiDraw::Push(vec![PoseOp::Translate(1.5, 2.0)]),
        gradient(),
        GuiDraw::Pop,
    ];
    let mut out = Writer::new();
    out.put(&draws);
    assert_eq!(Reader::decode::<Vec<GuiDraw>>(out.bytes()).unwrap(), draws);
}

#[test]
fn wire_loopback_a_guests_gradients_reach_the_draw_context_with_their_colour_order() {
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
    guest::Guest.fill_gradient([1.0, 2.0, 30.0, 40.0], 0xFF11_2233, 0x0044_5566);
    guest::Guest.fill_gradient([5.0, 6.0, -7.0, 0.0], 1, 2);
    assert_eq!(guest::Guest.gui_paint(&[gradient()]), Ok(()));
    guest::set_transport(None);
    drop(cx);
    assert_eq!(
        draw.gradients,
        [
            ([1.0, 2.0, 30.0, 40.0], 0xFF11_2233, 0x0044_5566),
            ([5.0, 6.0, -7.0, 0.0], 1, 2),
        ]
    );
    assert_eq!(draw.painted, [vec![gradient()]]);
}
