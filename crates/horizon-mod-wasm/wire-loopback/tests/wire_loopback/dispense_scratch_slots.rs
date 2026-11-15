use super::*;
use mod_api::prelude::*;
use mod_api::{ItemPlace, ItemStack, StackWriteError, Transaction, wire::Wire};

struct World;
impl mod_api::Host for World {}
mod_api::impl_host_areas!(World);
impl mod_api::ServerHost for World {}
mod_api::impl_server_host_areas!(World, except(items));
impl ItemsServerHost for World {
    fn scratch_slots(
        &mut self,
        _: &Transaction,
        stack: ItemStack,
    ) -> Result<Option<ItemPlace>, StackWriteError> {
        assert_eq!(stack.count(), 3);
        Ok(Some(ItemPlace::ScratchSlots(43)))
    }
    fn scratch_take_slots(
        &mut self,
        _: &Transaction,
        place: &ItemPlace,
    ) -> Option<(Option<ItemStack>, Option<ItemStack>)> {
        assert_eq!(place, &ItemPlace::ScratchSlots(43));
        let key = REGISTERED_ITEM.get().unwrap();
        Some((ItemStack::new(key, 2), ItemStack::new(key, 1)))
    }
}

#[test]
fn two_scratch_slots_round_trip_through_the_guest_dispatcher() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    install_counter();
    let stack = ItemStack::new(REGISTERED_ITEM.get().unwrap(), 3).unwrap();
    let tx = Transaction::take(&mut Reader::new(&17u64.to_le_bytes())).unwrap();
    let mut frame = boundary::Frame::default();
    let mut world = World;
    frame.set_server_host_mut(&mut world);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = std::cell::RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    let place = guest::Guest.scratch_slots(&tx, stack).unwrap().unwrap();
    assert_eq!(place, ItemPlace::ScratchSlots(43));
    let (first, overflow) = guest::Guest.scratch_take_slots(&tx, &place).unwrap();
    assert_eq!(first.unwrap().count(), 2);
    assert_eq!(overflow.unwrap().count(), 1);
    guest::set_transport(None);
}
