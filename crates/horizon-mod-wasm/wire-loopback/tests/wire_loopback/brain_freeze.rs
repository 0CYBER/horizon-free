//! `MobRef::set_brain_frozen` and `MobRef::brain_frozen` from a native and a
//! wasm mod reach the server's backend with the mob and the flag intact, and
//! each named refusal comes back as the native call returns it (#4478).
use super::*;
use mod_api::__private::game::{BackendMutRefOf, BackendRefOf};
use mod_api::{BrainFreezeRefusal, EntityRef, MobRef, WorldEpoch};

thread_local! {
    static ASKED: std::cell::RefCell<Vec<(MobRef, bool)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// What the server answers: the mob's id picks the refusal, 1 and 2 freeze.
fn answer(mob: MobRef) -> Result<(), BrainFreezeRefusal> {
    match mob.entity().id() {
        1 | 2 => Ok(()),
        3 => Err(BrainFreezeRefusal::NotAMob),
        _ => Err(BrainFreezeRefusal::NoMob),
    }
}

/// The read: a mob whose id is even is frozen, 4 and up name no mob.
fn frozen(mob: MobRef) -> Option<bool> {
    let id = mob.entity().id();
    (id < 4).then_some(id % 2 == 0)
}

struct Server;
impl mod_api::Host for Server {
    fn backend(&self) -> Option<mod_api::BackendRef<'_>> {
        Some(mod_api::BackendRef::of(self))
    }
}
mod_api::impl_host_areas!(Server);
impl mod_api::ServerHost for Server {
    fn backend_mut(&mut self) -> Option<mod_api::BackendMutRef<'_>> {
        Some(mod_api::BackendMutRef::of(self))
    }
}
mod_api::impl_server_host_areas!(Server);
impl mod_api::Backend for Server {}
mod_api::impl_backend_areas!(Server, except(mobs));
impl mod_api::prelude::MobsBackend for Server {
    fn mob_brain_frozen(&self, mob: MobRef) -> Option<bool> {
        frozen(mob)
    }
}
impl mod_api::BackendMut for Server {}
mod_api::impl_backend_mut_areas!(Server, except(mobs));
impl mod_api::prelude::MobsBackendMut for Server {
    fn mob_set_brain_frozen(
        &mut self,
        mob: MobRef,
        frozen: bool,
    ) -> Result<(), BrainFreezeRefusal> {
        ASKED.with_borrow_mut(|asked| asked.push((mob, frozen)));
        answer(mob)
    }
}

struct NoBackend;
impl mod_api::Host for NoBackend {}
mod_api::impl_host_areas!(NoBackend);
impl mod_api::ServerHost for NoBackend {}
mod_api::impl_server_host_areas!(NoBackend);

/// The asks: a freeze, a thaw, and the refusals a port must tell apart.
fn asks() -> Vec<(MobRef, bool)> {
    let epoch = WorldEpoch::new(9);
    (1..=5)
        .map(|id| (MobRef::new(EntityRef::new(epoch, id)), id % 2 == 1))
        .collect()
}

#[test]
fn a_native_mods_brain_freeze_dispatches_to_the_server_backend() {
    ASKED.with_borrow_mut(Vec::clear);
    let mut server = Server;
    for (mob, wanted) in asks() {
        assert_eq!(mob.set_brain_frozen(&mut server, wanted), answer(mob));
        assert_eq!(mob.brain_frozen(&server), frozen(mob));
    }
    assert_eq!(ASKED.with_borrow_mut(std::mem::take), asks());
}

#[test]
fn a_wasm_mods_brain_freeze_crosses_whole_and_its_refusal_comes_back() {
    // The neighbours' guards: per-thread journal, the phase scope and `TransportScope`.
    ASKED.with_borrow_mut(Vec::clear);
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut server = Server;
    {
        let host = &mut server;
        let mut frame = boundary::Frame::default();
        frame.set_server_host_mut(host);
        let mut cx = Box::new(boundary::Cx::default());
        cx.frames.push(frame);
        let pointer = &mut *cx as *mut boundary::Cx;
        // SAFETY: the context and borrowed host outlive synchronous proxy calls.
        let transport = unsafe { host_dispatch::transport(pointer) };
        let previous =
            LOOPBACK_TRANSPORT.with(|active| active.borrow_mut().replace(transport.clone()));
        let _restore = TransportScope(previous);
        guest::set_transport(Some(transport));
        for (mob, wanted) in asks() {
            assert_eq!(
                mob.set_brain_frozen(&mut guest::Guest, wanted),
                answer(mob),
                "mob {} came back as the server answered",
                mob.entity().id()
            );
            assert_eq!(mob.brain_frozen(&guest::Guest), frozen(mob));
        }
        guest::set_transport(None);
    }
    // The server was handed exactly what the guest asked, in order.
    assert_eq!(ASKED.with_borrow_mut(std::mem::take), asks());
}

#[test]
fn brain_freeze_with_no_backend_is_refused_as_no_mob() {
    let mut absent = NoBackend;
    for (mob, wanted) in asks() {
        assert_eq!(
            mob.set_brain_frozen(&mut absent, wanted),
            Err(BrainFreezeRefusal::NoMob)
        );
        assert_eq!(mob.brain_frozen(&absent), None);
    }
}

#[test]
fn every_brain_freeze_refusal_round_trips_on_the_wire() {
    for refusal in [BrainFreezeRefusal::NoMob, BrainFreezeRefusal::NotAMob] {
        let mut w = Writer::new();
        w.put(&Err::<(), _>(refusal));
        let bytes = w.into_bytes();
        let mut r = Reader::new(&bytes);
        let back: Result<(), BrainFreezeRefusal> = r.take().unwrap();
        assert_eq!(back, Err(refusal));
    }
}
