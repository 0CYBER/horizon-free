//! `MobRef::set_ai_paused` and `MobRef::ai_paused` from a native and a
//! wasm mod reach the server's backend with the mob and the flag intact, and
//! each named refusal comes back as the native call returns it (arismoko/horizon#4478).
use super::*;
use mod_api::__private::game::{BackendMutRefOf, BackendRefOf};
use mod_api::{EntityRef, MobAiPauseRefusal, MobRef, WorldEpoch};

thread_local! {
    static ASKED: std::cell::RefCell<Vec<(MobRef, bool)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Every refusal, in the order the asks' ids 3 to 7 get them.
const REFUSALS: [MobAiPauseRefusal; 5] = [
    MobAiPauseRefusal::Stale,
    MobAiPauseRefusal::Client,
    MobAiPauseRefusal::HeldOut,
    MobAiPauseRefusal::NoAiStep,
    MobAiPauseRefusal::NotAMob,
];

/// What the server answers: 1 and 2 pause or resume, 3 to 7 are refused.
fn answer(mob: MobRef) -> Result<(), MobAiPauseRefusal> {
    match mob.entity().id() {
        1 | 2 => Ok(()),
        id => Err(REFUSALS[id as usize - 3]),
    }
}

/// The read: a mob whose id is even is paused, 3 and up name no mob.
fn paused(mob: MobRef) -> Option<bool> {
    let id = mob.entity().id();
    (id < 3).then_some(id % 2 == 0)
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
    fn mob_ai_paused(&self, mob: MobRef) -> Option<bool> {
        paused(mob)
    }
}
impl mod_api::BackendMut for Server {}
mod_api::impl_backend_mut_areas!(Server, except(mobs));
impl mod_api::prelude::MobsBackendMut for Server {
    fn mob_set_ai_paused(&mut self, mob: MobRef, paused: bool) -> Result<(), MobAiPauseRefusal> {
        ASKED.with_borrow_mut(|asked| asked.push((mob, paused)));
        answer(mob)
    }
}

struct NoBackend;
impl mod_api::Host for NoBackend {}
mod_api::impl_host_areas!(NoBackend);
impl mod_api::ServerHost for NoBackend {}
mod_api::impl_server_host_areas!(NoBackend);

/// The asks: a pause, a resume, and each refusal a port must tell apart.
fn asks() -> Vec<(MobRef, bool)> {
    let epoch = WorldEpoch::new(9);
    (1..=7)
        .map(|id| (MobRef::new(EntityRef::new(epoch, id)), id % 2 == 1))
        .collect()
}

#[test]
fn a_native_mods_ai_pause_dispatches_to_the_server_backend() {
    ASKED.with_borrow_mut(Vec::clear);
    let mut server = Server;
    for (mob, wanted) in asks() {
        assert_eq!(mob.set_ai_paused(&mut server, wanted), answer(mob));
        assert_eq!(mob.ai_paused(&server), paused(mob));
    }
    assert_eq!(ASKED.with_borrow_mut(std::mem::take), asks());
}

#[test]
fn a_wasm_mods_ai_pause_crosses_whole_and_its_refusal_comes_back() {
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
                mob.set_ai_paused(&mut guest::Guest, wanted),
                answer(mob),
                "mob {} came back as the server answered",
                mob.entity().id()
            );
            assert_eq!(mob.ai_paused(&guest::Guest), paused(mob));
        }
        guest::set_transport(None);
    }
    // The server was handed exactly what the guest asked, in order.
    assert_eq!(ASKED.with_borrow_mut(std::mem::take), asks());
}

#[test]
fn an_ai_pause_with_no_backend_is_refused_as_client() {
    let mut absent = NoBackend;
    for (mob, wanted) in asks() {
        assert_eq!(
            mob.set_ai_paused(&mut absent, wanted),
            Err(MobAiPauseRefusal::Client)
        );
        assert_eq!(mob.ai_paused(&absent), None);
    }
}

#[test]
fn every_ai_pause_refusal_round_trips_on_the_wire() {
    for refusal in REFUSALS {
        let mut w = Writer::new();
        w.put(&Err::<(), _>(refusal));
        let bytes = w.into_bytes();
        let mut r = Reader::new(&bytes);
        let back: Result<(), MobAiPauseRefusal> = r.take().unwrap();
        assert_eq!(back, Err(refusal));
    }
}
