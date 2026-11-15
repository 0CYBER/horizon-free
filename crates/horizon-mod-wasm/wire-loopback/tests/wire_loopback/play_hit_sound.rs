use super::*;
thread_local! {
    static HANDLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
struct Quiet;
#[mod_api::horizon_part]
impl mod_api::BlockPart for Quiet {
    fn play_hit_sound(
        &self,
        _: &mut dyn mod_api::ClientHost,
        event: &mut mod_api::PlayHitSoundEvent,
    ) -> mod_api::Flow {
        assert_eq!(event.pos, mod_api::BlockPos::new(5, 63, 4));
        assert_eq!(event.face, mod_api::Direction::West);
        if HANDLED.get() {
            mod_api::Flow::Cancel
        } else {
            mod_api::Flow::Continue
        }
    }
}
#[test]
fn wire_loopback_play_hit_sound_handled_and_default() {
    struct Client;
    impl mod_api::Host for Client {}
    mod_api::impl_host_areas!(Client);
    impl mod_api::ClientHost for Client {}
    mod_api::impl_client_host_areas!(Client);
    install_counter();
    // The platform phase this scope sets is process-wide state another test may not have installed yet.
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let index = guest::add_handler(vec![guest::view::<dyn mod_api::BlockPart>(Arc::new(Quiet))]);
    let proxy = WasmHandler {
        index,
        module: WasmMod {
            group: 0,
            subscribed: &[],
            listeners: &[],
            corrupt: false,
        },
    };
    for (handled, expected) in [
        (true, mod_api::Flow::Cancel),
        (false, mod_api::Flow::Continue),
    ] {
        HANDLED.set(handled);
        let mut event = mod_api::PlayHitSoundEvent {
            state: mod_api::BlockStateRecord::new(REGISTERED_BLOCK.get().unwrap(), 0).into(),
            pos: mod_api::BlockPos::new(5, 63, 4),
            face: mod_api::Direction::West,
        };
        assert_eq!(
            mod_api::BlockPart::play_hit_sound(&proxy, &mut Client, &mut event),
            expected
        );
    }
}
