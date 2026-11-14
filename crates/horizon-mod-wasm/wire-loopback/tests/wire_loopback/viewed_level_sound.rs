use super::*;

// The shared guest fixture forwards this hook to its per-item listener.
pub(super) fn listen(event: &mut mod_api::ViewedLevelSoundEvent<'_>) {
    assert_eq!(
        mod_api::__private::dimension_id(event.level.dimension()),
        Some("minecraft:the_nether")
    );
    assert_eq!(event.sound.to_string(), "minecraft:block.anvil.land");
    assert_eq!(event.source, mod_api::SoundSource::Blocks);
    assert_eq!((event.position, event.seed), ([56.5, 64.0, 8.25], 42));
    *event.play_at = Some([7.0, 64.0, 1.0]);
    event.volume /= 4.0;
    event.pitch = 1.5;
}

/// A wasm listener of `viewed_level_sound` reads the viewed level, the sound, its source,
/// position and seed, and the `play_at`, volume and pitch it leaves come back.
#[test]
fn wire_loopback_viewed_level_sound_places_the_sound() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    struct Client;
    impl mod_api::Host for Client {}
    mod_api::impl_host_areas!(Client);
    impl mod_api::ClientHost for Client {}
    mod_api::impl_client_host_areas!(Client);
    install_counter();
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    let level = mod_api::LevelRef::new(
        mod_api::WorldEpoch::new(1),
        mod_api::__private::dimension_key("minecraft:the_nether"),
    );
    let sound = mod_api::Identifier::parse("minecraft:block.anvil.land").unwrap();
    let mut play_at = None;
    let mut event = mod_api::ViewedLevelSoundEvent {
        level,
        sound: &sound,
        source: mod_api::SoundSource::Blocks,
        position: [56.5, 64.0, 8.25],
        volume: 2.0,
        pitch: 0.5,
        seed: 42,
        play_at: &mut play_at,
    };
    module.viewed_level_sound(&mut Client, &mut event);
    assert_eq!((event.volume, event.pitch), (0.5, 1.5));
    assert_eq!(play_at, Some([7.0, 64.0, 1.0]));
}

pub(super) struct ViewedLevelSoundHooks;
impl ViewedLevelSoundHooks {
    pub(super) fn viewed_level_sound(
        &self,
        _: &mut dyn mod_api::ClientHost,
        event: &mut mod_api::ViewedLevelSoundEvent<'_>,
    ) {
        self::listen(event);
    }
}
