use super::*;
use mod_api::__private::game::{BackendMutRefOf, BackendRefOf};
use mod_api::prelude::*;

struct World;
impl mod_api::Backend for World {}
mod_api::impl_backend_areas!(World, except(items));
impl ItemsBackend for World {
    fn item_bundle_slots(
        &self,
        stack: &mod_api::ItemStack,
    ) -> Option<Vec<Option<mod_api::ItemStack>>> {
        Some(vec![Some(stack.clone())])
    }
    fn item_bundle_capacity(
        &self,
        _: &mod_api::ItemStack,
        slot: usize,
        _: &mod_api::ItemStack,
    ) -> u32 {
        if slot == 0 { 13 } else { 0 }
    }
    fn item_bundle_insert_slot(
        &self,
        stack: &mod_api::ItemStack,
        slot: usize,
        item: &mod_api::ItemStack,
    ) -> Option<(u32, mod_api::ItemStack)> {
        (slot == 0).then(|| (item.count().min(13), stack.clone()))
    }
    fn item_bundle_extract(
        &self,
        stack: &mod_api::ItemStack,
        slot: usize,
        max: u32,
    ) -> Option<(Option<mod_api::ItemStack>, mod_api::ItemStack)> {
        (slot == 0 && max > 0).then(|| (Some(stack.clone()), stack.clone()))
    }
}
impl mod_api::Host for World {
    fn backend(&self) -> Option<mod_api::BackendRef<'_>> {
        Some(mod_api::BackendRef::of(self))
    }
}
mod_api::impl_host_areas!(World);

impl mod_api::BackendMut for World {}
mod_api::impl_backend_mut_areas!(World, except(items));
impl ItemsBackendMut for World {
    fn game_stack(&mut self, _: &mod_api::ItemPlace) -> Option<mod_api::ItemStack> {
        mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 3)
    }
}
impl mod_api::ServerHost for World {
    fn backend_mut(&mut self) -> Option<mod_api::BackendMutRef<'_>> {
        Some(mod_api::BackendMutRef::of(self))
    }
}
mod_api::impl_server_host_areas!(World);

#[test]
fn bundle_backend_operations_round_trip() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    install_counter();
    let stack = mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 3).unwrap();
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
    assert_eq!(
        guest::Guest.item_bundle_slots(&stack),
        Some(vec![Some(stack.clone())])
    );
    assert_eq!(guest::Guest.item_bundle_capacity(&stack, 0, &stack), 13);
    assert_eq!(guest::Guest.item_bundle_capacity(&stack, 1, &stack), 0);
    assert_eq!(
        guest::Guest.item_bundle_insert_slot(&stack, 0, &stack),
        Some((3, stack.clone()))
    );
    assert_eq!(
        guest::Guest.item_bundle_extract(&stack, 0, 1),
        Some((Some(stack.clone()), stack.clone()))
    );
    use mod_api::ResourceHandler;
    let handler = mod_api::BundleItems::new(mod_api::ItemPlace::PlayerSlot { player: 7, slot: 0 });
    assert_eq!(handler.size(&mut guest::Guest), 1);
    assert_eq!(handler.amount(&mut guest::Guest, 0), 3);
    assert_eq!(
        handler.resource(&mut guest::Guest, 0),
        Some(mod_api::ItemResource::of(&stack))
    );
    guest::set_transport(None);
}
