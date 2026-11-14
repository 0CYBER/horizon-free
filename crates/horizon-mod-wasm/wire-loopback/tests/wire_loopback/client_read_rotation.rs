use super::*;
use mod_api::prelude::{ClientSession, ClientViewClientHost, ItemsClientHost, ServicesClientHost};
use mod_api::{Hand, ItemStack, ItemUse, LocalRotationError};
use std::cell::Cell;

struct Client {
    world: Cell<bool>,
    look: Cell<(f32, f32)>,
}
impl mod_api::Host for Client {}
mod_api::impl_host_areas!(Client);
// These are boundary reply fixtures, not an implementation of the game's name resolver.
impl ItemsClientHost for Client {
    fn hover_name(&self, stack: &ItemStack) -> String {
        match stack.count() {
            1 => "Stone".into(),
            2 => "Named stone".into(),
            _ => String::new(),
        }
    }
}
impl mod_api::ClientHost for Client {}
mod_api::impl_client_host_areas!(Client, except(client_view, services, items));
impl ServicesClientHost for Client {
    fn client_session(&self) -> &dyn ClientSession {
        self
    }
}
impl ClientSession for Client {
    fn client_latency(&self) -> Option<u32> {
        self.world.get().then_some(87)
    }
}
impl ClientViewClientHost for Client {
    fn local_item_use(&self) -> Option<ItemUse> {
        self.world.get().then(|| ItemUse::new(Hand::Main, 7, 71993))
    }
    fn set_local_rotation(&mut self, yaw: f32, pitch: f32) -> Result<(), LocalRotationError> {
        if !self.world.get() {
            return Err(LocalRotationError::NoWorld);
        }
        if !yaw.is_finite() || !pitch.is_finite() {
            return Err(LocalRotationError::NotFinite);
        }
        self.look.set((yaw, (pitch % 360.0).clamp(-90.0, 90.0)));
        Ok(())
    }
}

#[test]
fn client_reads_and_rotation_keep_literal_values_across_the_host_call() {
    install_counter();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut client = Client {
        world: Cell::new(true),
        look: Cell::new((0.0, 0.0)),
    };
    let mut frame = boundary::Frame::default();
    frame.set_client_host_mut(&mut client);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = std::cell::RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        // SAFETY: the client and its frame outlive this synchronous transport.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    for (count, expected) in [(1, "Stone"), (2, "Named stone"), (3, "")] {
        let stack = ItemStack::new(REGISTERED_ITEM.get().unwrap(), count).unwrap();
        assert_eq!(guest::Guest.hover_name(&stack), expected);
    }
    assert_eq!(guest::Guest.client_latency(), Some(87));
    assert_eq!(
        guest::Guest.local_item_use(),
        Some(ItemUse::new(Hand::Main, 7, 71993))
    );
    assert_eq!(guest::Guest.set_local_rotation(450.0, 100.0), Ok(()));
    assert_eq!(client.look.get(), (450.0, 90.0));
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(
            guest::Guest.set_local_rotation(invalid, 0.0),
            Err(LocalRotationError::NotFinite)
        );
        assert_eq!(client.look.get(), (450.0, 90.0));
    }
    client.world.set(false);
    assert_eq!(guest::Guest.client_latency(), None);
    assert_eq!(guest::Guest.local_item_use(), None);
    assert_eq!(
        guest::Guest.set_local_rotation(0.0, 0.0),
        Err(LocalRotationError::NoWorld)
    );
    assert_eq!(client.look.get(), (450.0, 90.0));
    guest::set_transport(None);
}
