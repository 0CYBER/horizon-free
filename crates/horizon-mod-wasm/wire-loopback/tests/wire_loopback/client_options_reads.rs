use super::*;
use mod_api::prelude::*;

/// A client whose GUI scale is the value a stubbed window resolved.
struct Client(Option<u32>);
impl mod_api::Host for Client {}
mod_api::impl_host_areas!(Client);
impl mod_api::ClientHost for Client {}
mod_api::impl_client_host_areas!(Client, except(client_options));
impl ClientOptionsClientHost for Client {
    fn gui_scale(&self) -> Option<u32> {
        self.0
    }
}

/// The same literal values the native resolution test asserts reach a wasm
/// mod unchanged through the generated wire proxy.
#[test]
fn a_wasm_mod_reads_the_gui_scale_the_window_applies() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    for expected in [Some(3), Some(4), Some(2), Some(1), None] {
        let client = Client(expected);
        let mut frame = boundary::Frame::default();
        frame.set_client_host(&client);
        let mut cx = boundary::Cx::default();
        cx.frames.push(frame);
        let cx = std::cell::RefCell::new(cx);
        guest::set_transport(Some(Rc::new(move |id, bytes| {
            let mut out = Writer::new();
            // SAFETY: client and its frame remain live for this synchronous transport.
            unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
            out.into_bytes()
        })));
        assert_eq!(guest::Guest.gui_scale(), expected);
    }
    guest::set_transport(None);
}

#[test]
fn a_wasm_mod_can_round_trip_a_resolved_input_glyph() {
    use mod_api::{ControllerButton, ControllerGlyphFamily, DrawContext, InputGlyph};

    struct GlyphHost(InputGlyph);
    impl mod_api::Host for GlyphHost {}
    mod_api::impl_host_areas!(GlyphHost);
    impl mod_api::ClientHost for GlyphHost {}
    mod_api::impl_client_host_areas!(GlyphHost, except(client_options));
    impl ClientOptionsClientHost for GlyphHost {
        fn input_glyph(&self, action: &str) -> Option<InputGlyph> {
            (action == "key.jump").then(|| self.0.clone())
        }
    }

    #[derive(Default)]
    struct Draw(Option<(InputGlyph, [f32; 2], u32)>);
    impl DrawContext for Draw {
        fn input_glyph(&mut self, glyph: &InputGlyph, at: [f32; 2], argb: u32) -> bool {
            self.0 = Some((glyph.clone(), at, argb));
            true
        }
    }

    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let client = GlyphHost(mod_api::__private::game::input_glyph(
        "A".into(),
        [12.0, 12.0],
        Some((ControllerGlyphFamily::Nintendo, ControllerButton::East)),
    ));
    let mut draw = Draw::default();
    let mut frame = boundary::Frame::default();
    frame.set_client_host(&client);
    frame.set_draw_context_mut(&mut draw);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = std::cell::RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        // SAFETY: the host and draw context remain live until this synchronous transport ends.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    assert_eq!(
        ClientOptionsClientHost::input_glyph(&guest::Guest, "unknown"),
        None
    );
    let glyph = ClientOptionsClientHost::input_glyph(&guest::Guest, "key.jump").unwrap();
    assert_eq!(glyph, client.0);
    let at = [21.0, 7.0];
    let argb = 0xFFFF_7788;
    assert!(DrawContext::input_glyph(&mut guest::Guest, &glyph, at, argb));
    guest::set_transport(None);
    assert_eq!(draw.0, Some((client.0, at, argb)));
}
