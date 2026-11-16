//! The `sound_events` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `starting` is true for starts and loop replays, false for a live channel refresh.
    /// A live refresh applies only volume and pitch transiently; `None` stops the channel.
    /// Invalid edited numeric values refuse that refresh.
    /// A sound is about to play (`PlaySoundEvent`, vanilla's `SoundEngine.play`): `original` is the sound as asked for, `sound` starts as a copy of it. Edit `sound` (its volume, pitch, source, position, identifier, looping, delay, attenuation or relativeness) to change what plays, or set it to `None` to play nothing. It is asked before the engine's own checks (output open, sound resolved, volume, channel pool), for every sound the client plays, so answer fast; later mods see earlier mods' edits in `sound` and the sound as asked for in `original`. A new `identifier` is resolved again, with its own weighted pick. `context` is the client's `LoopContext`: no game is at hand inside the sound engine. Client-side only; a server has no sound.
    #[hook(
        side = "client",
        contract = "listen",
        context = "loop",
        neoforge = "PlaySoundEvent",
        cancellable = false,
        target = "horizon_client_render::sound_engine::hooks::native_play_sound"
    )]
    /// `instance` is an engine-local sound instance id, unchanged across its live updates.
    fn play_sound();
    /// A sound that is not streamed has started on `channel` (`PlaySoundSourceEvent`), after the engine set its pitch, volume, attenuation, looping and position: `sound` is what is playing, after `play_sound`'s edits. A notice: the channel is an id only and gives no control. Streamed sounds (music, records) fire `play_streaming_source` instead. Client-side only.
    #[hook(
        side = "client",
        contract = "listen",
        context = "loop",
        neoforge = "PlaySoundSourceEvent",
        cancellable = false,
        target = "horizon_client_render::sound_engine::hooks::native_play_sound_source"
    )]
    fn play_sound_source();
    /// A streamed sound (music, a record) has started on `channel` (`PlayStreamingSourceEvent`): `sound` is what is playing, after `play_sound`'s edits. A notice: the channel is an id only and gives no control. Other sounds fire `play_sound_source` instead. Client-side only.
    #[hook(
        side = "client",
        contract = "listen",
        context = "loop",
        neoforge = "PlayStreamingSourceEvent",
        cancellable = false,
        target = "horizon_client_render::sound_engine::hooks::native_play_streaming_source"
    )]
    fn play_streaming_source();
    /// The sound engine opened its output (`SoundEngineLoadEvent`): once at window start, after each resource reload (with the `context` of the new resource generation) and when the output device changes. Nothing fires with no sound output (no device, or audio turned off). Client-side only.
    #[hook(
        side = "client",
        contract = "listen",
        context = "loop",
        neoforge = "SoundEngineLoadEvent",
        cancellable = false,
        target = "horizon_client_render::sound_engine::hooks::native_sound_engine_load"
    )]
    fn sound_engine_load();
}
