use super::*;
use mod_api::{
    Compound, DrawContext, EntityRef, GuiEntity, GuiEntityError, GuiEntityLook, GuiEntityView, Tag,
    WorldEpoch,
};

/// A screen's drawing that keeps each entity it is asked to draw, refusing a handle of id 0.
#[derive(Default)]
struct Draw(Vec<(GuiEntity, GuiEntityView)>);
impl DrawContext for Draw {
    fn entity(&mut self, entity: &GuiEntity, view: GuiEntityView) -> Result<(), GuiEntityError> {
        self.0.push((entity.clone(), view));
        match entity {
            GuiEntity::Existing(handle) if handle.id() == 0 => Err(GuiEntityError::NoEntity),
            _ => Ok(()),
        }
    }
}

/// What a data model's screen draws: a baby zombie turned toward the cursor, a spinning
/// golem of a mod's, an entity of the level, and one whose handle is refused.
fn asks() -> Vec<(GuiEntity, GuiEntityView, Result<(), GuiEntityError>)> {
    let mut data = Compound::new();
    data.insert("IsBaby".into(), Tag::Byte(1));
    data.insert("CustomName".into(), Tag::String("\"Bob\"".into()));
    let rect = [26.0, 18.0, 78.0, 70.0];
    let epoch = WorldEpoch::new(3);
    vec![
        (
            GuiEntity::with_data("minecraft:zombie", data),
            GuiEntityView::new(rect, 30.0, 0.0625, GuiEntityLook::At([40.5, 12.0])),
            Ok(()),
        ),
        (
            GuiEntity::kind("wire_test:golem"),
            GuiEntityView::new(
                rect,
                17.0,
                0.25,
                GuiEntityLook::Turned {
                    yaw: 137.5,
                    pitch: -10.0,
                },
            ),
            Ok(()),
        ),
        (
            GuiEntity::existing(EntityRef::new(epoch, 42)),
            GuiEntityView::new(rect, 25.0, 0.0, GuiEntityLook::At([0.0, 0.0])),
            Ok(()),
        ),
        (
            GuiEntity::existing(EntityRef::new(epoch, 0)),
            GuiEntityView::new(rect, 25.0, 0.0, GuiEntityLook::At([0.0, 0.0])),
            Err(GuiEntityError::NoEntity),
        ),
    ]
}

/// A native mod's screen draw: the hook's draw context is the game's, called directly.
struct Screen;
#[mod_api::__subscriptions]
impl Hooks for Screen {
    fn gui_render_post(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::GuiRenderPostEvent<'_>,
    ) {
        for (entity, view, answer) in asks() {
            assert_eq!(event.batch.entity(&entity, view), answer);
        }
    }
}

struct Client;
impl mod_api::Host for Client {}
mod_api::impl_host_areas!(Client);
impl mod_api::ClientHost for Client {}
mod_api::impl_client_host_areas!(Client);

#[test]
fn a_native_mods_screen_draw_reaches_the_draw_contexts_entity() {
    let mods = vec![mod_api::ModEntry::new("wire_test", Box::new(Screen))];
    mod_api::testing::with_mods(mods.into(), || {
        let mut draw = Draw::default();
        mod_api::dispatch_gui_render_post(&Client, &mut draw, false, true);
        let sent: Vec<_> = asks().into_iter().map(|(e, v, _)| (e, v)).collect();
        assert_eq!(draw.0, sent);
    });
}

#[test]
fn a_wasm_mods_gui_entity_crosses_whole_and_its_refusal_comes_back() {
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
        // SAFETY: cx and its frame's draw remain live for this synchronous transport.
        unsafe { boundary::serve(id, pointer, bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    let mut sent = Vec::new();
    for (entity, view, answer) in asks() {
        assert_eq!(
            DrawContext::entity(&mut guest::Guest, &entity, view),
            answer
        );
        sent.push((entity, view));
    }
    guest::set_transport(None);
    drop(cx);
    // The host was handed exactly what the guest drew: type, data, handle and view.
    assert_eq!(draw.0, sent);
}
