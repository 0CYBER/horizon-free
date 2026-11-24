use super::*;

/// A run of backend errors close together is a dead stream; one or two, or errors far apart,
/// are hiccups.
#[test]
fn a_run_of_stream_errors_is_a_dead_stream() {
    use std::time::{Duration, Instant};
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    let mut errors = StreamErrors::default();
    assert!(!errors.dead(at(0)));
    assert!(!errors.dead(at(500)));
    assert!(errors.dead(at(900)));
    // Spread out, they never add up.
    let mut apart = StreamErrors::default();
    for k in 0..10 {
        assert!(!apart.dead(at(k * 2500)), "error {k}");
    }
}

/// A stream that stops on a run of backend errors (ALSA's POLLERR after an underrun) is opened
/// again with the mixer's channels left playing where they were; a device that is gone reloads
/// the output, and wins over a stopped stream whichever came first.
#[test]
fn a_stopped_stream_keeps_its_sounds_and_a_lost_device_does_not() {
    use std::time::{Duration, Instant};
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    let mut m = Mixer::default();
    m.channels
        .insert(1, channel(sine(440.0), None, [0.0; 3], true));
    let mut data = [0.0f32; 512];
    mix(&mut m, &mut data, 2, 48_000);
    let played = m.channels[&1].position;
    assert!(played > 0.0);
    // One hiccup is nothing; a run of them stops the stream.
    m.stream_error(false, at(0));
    assert_eq!(m.lost, None);
    m.stream_error(false, at(10));
    m.stream_error(false, at(20));
    assert_eq!(m.lost, Some(Lost::Stream));
    assert!(
        !m.take_lost_device(),
        "a stopped stream is not a lost device"
    );
    assert!(m.take_stopped_stream());
    assert_eq!(m.lost, None);
    assert!(!m.take_stopped_stream(), "taken once");
    // The next stream mixes the same channel on from where it was.
    assert_eq!(m.channels[&1].state, State::Playing);
    assert_eq!(m.channels[&1].position, played);
    mix(&mut m, &mut data, 2, 48_000);
    assert!(m.channels[&1].position > played);
    // The device going away after (or before) the stream stopped: the output is reloaded.
    for device_first in [false, true] {
        let mut m = Mixer::default();
        if device_first {
            m.stream_error(true, at(0));
        }
        for k in 1..=5 {
            m.stream_error(false, at(k * 10));
        }
        m.stream_error(true, at(100));
        assert_eq!(m.lost, Some(Lost::Device));
        assert!(!m.take_stopped_stream());
        assert!(m.take_lost_device());
        assert_eq!(m.lost, None);
    }
}

/// Each stream stopped on underruns doubles the next one's buffer, from the short 1024
/// frames to 4096; a lost device doesn't.
#[test]
fn a_stream_stopped_on_underruns_reopens_with_a_longer_buffer() {
    use std::time::Instant;
    assert_eq!(buffer_frames(0), SHORT_BUFFER);
    assert_eq!(buffer_frames(1), 2048);
    assert_eq!(buffer_frames(2), LONGEST_BUFFER);
    assert_eq!(buffer_frames(40), LONGEST_BUFFER);
    let mut m = Mixer::default();
    let mut frames = Vec::new();
    for _ in 0..4 {
        // A reopened stream starts with no errors (`open_device`).
        m.stream_errors = StreamErrors::default();
        for _ in 0..StreamErrors::DEAD_AFTER {
            m.stream_error(false, Instant::now());
        }
        assert!(m.take_stopped_stream());
        frames.push(buffer_frames(m.buffer_steps));
    }
    assert_eq!(frames, [2048, 4096, 4096, 4096]);
    assert_eq!(m.buffer_steps, 2, "the steps stop at the longest buffer");
    let mut lost = Mixer::default();
    lost.stream_error(true, Instant::now());
    assert!(lost.take_lost_device());
    assert_eq!(lost.buffer_steps, 0);
}

/// A failed reopening is tried again every two seconds, five more times, then given up.
#[test]
fn a_failed_reopening_retries_a_few_times_then_stops() {
    let mut reopen = Reopen::default();
    assert!(!reopen.due(), "nothing to retry while the output plays");
    assert!(reopen.failed(false));
    let mut tries = 0;
    for _ in 0..1000 {
        if reopen.due() {
            tries += 1;
            if !reopen.failed(true) {
                break;
            }
        }
    }
    assert_eq!(tries, Reopen::TRIES);
    assert_eq!(reopen, Reopen::default());
    // The next lost device starts a fresh run.
    assert!(reopen.failed(false));
    assert_eq!(reopen.tries, Reopen::TRIES);
}

#[test]
fn reload_isolates_pending_decodes_from_the_new_resource_cache() {
    let source = ResourceSource::indexed(Default::default());
    let mut audio = Audio::silent(&source);
    let old = audio.backend.cache.clone();
    old.lock().unwrap().insert("old".into(), None);
    audio.reload(&source);
    assert!(!Arc::ptr_eq(&old, &audio.backend.cache));
    old.lock().unwrap().insert("late".into(), None);
    assert!(audio.backend.cache.lock().unwrap().entries.is_empty());
}

#[test]
#[ignore = "daily"]
fn archive_sound_decodes_like_the_original_without_unpacking() {
    use std::io::Write;
    let prepared =
        std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").expect("resources"));
    let source = ResourceSource::directory(prepared);
    let name = source
        .list("assets/minecraft/sounds/")
        .into_iter()
        .find(|name| name.ends_with(".ogg"))
        .expect("imported ogg");
    let bytes = source.read(&name).unwrap();
    let original = decode(&Resource::bytes(bytes.clone())).expect("vanilla ogg decodes");
    assert!(!original.0.is_empty());
    let folder = std::env::temp_dir().join(format!(
        "horizon-archive-sound-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&folder).unwrap();
    let jar = folder.join("sound.jar");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&jar).unwrap());
    writer
        .start_file(&name, zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(&bytes).unwrap();
    writer.finish().unwrap();
    let resource = Resource::file(ResourceSource::archive(&jar).unwrap(), name);
    std::fs::remove_file(&jar).unwrap();
    let archived = decode(&resource).expect("retained archive ogg decodes");
    assert_eq!(archived, original);
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
    std::fs::remove_dir(&folder).unwrap();
}

#[test]
fn explicit_category_seed_does_not_advance_local_sound_random() {
    let source = ResourceSource::indexed(Default::default());
    let mut audio = Audio::silent(&source);
    let mut expected = audio.random.clone();
    audio.capture = Some(Vec::new());
    audio.play_on_seeded(
        8,
        "minecraft:entity.cow.ambient",
        Some([1.0; 3]),
        0.5,
        1.5,
        -17,
    );
    assert_eq!(audio.random.next_long(), expected.next_long());
    let capture = audio.capture.as_ref().unwrap();
    assert_eq!(capture.len(), 1);
    assert_eq!(capture[0].source, 8);
    assert_eq!(capture[0].position, [1.0; 3]);
    assert_eq!((capture[0].volume, capture[0].pitch), (0.5, 1.5));
    assert!(audio.source_override.is_none());
    // A local request reserves one draw before dispatch, and playback reuses it.
    let seed = audio.next_sound_seed();
    assert_eq!(seed, expected.next_long());
    audio.play_on_seeded(
        7,
        "minecraft:entity.player.swim",
        Some([0.0; 3]),
        1.0,
        1.0,
        seed,
    );
    assert_eq!(audio.random.next_long(), expected.next_long());
}

#[test]
fn level_capture_keeps_order_category_entity_and_actor_exclusion() {
    let mut audio = Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
        (Path::new("/nonexistent/horizon-entity-sound-test")).to_path_buf(),
    ));
    audio.capture = Some(Vec::new());
    let mut expected = audio.random.clone();
    audio.play_on(
        7,
        "minecraft:block.amethyst_block.chime",
        Some([1.0; 3]),
        0.5,
        1.0,
    );
    let entity = crate::sound_engine::SoundEntity::new(37, [2.123456789, 64.0, -3.0], false);
    audio.play_bound(
        "minecraft:block.amethyst_block.chime",
        6,
        2.0,
        1.2,
        entity,
        Some(17),
    );
    let sounds = audio.capture.as_ref().unwrap();
    assert_eq!(sounds.len(), 2);
    assert_eq!(
        (sounds[0].source, sounds[0].entity, sounds[0].except_entity),
        (7, None, None)
    );
    let bound = &sounds[1];
    assert_eq!(
        (bound.source, bound.entity, bound.except_entity),
        (6, Some(37), Some(17))
    );
    assert_eq!(bound.position, [2.123456789, 64.0, -3.0]);
    assert_eq!((bound.volume, bound.pitch), (2.0, 1.2));
    expected.next_long();
    expected.next_long();
    assert_eq!(audio.random.next_long(), expected.next_long());
}

#[test]
fn entity_sound_packets_neither_rebroadcast_nor_advance_the_local_seed_generator() {
    let mut audio = Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
        (Path::new("/nonexistent/horizon-entity-sound-test")).to_path_buf(),
    ));
    audio.capture = Some(Vec::new());
    let mut expected = audio.random.clone();
    let entity = crate::sound_engine::SoundEntity::new(7, [1.0, 2.0, 3.0], false);
    audio.play_entity_seeded("minecraft:fixture.bound", 6, 0.7, 1.2, 42, entity);
    assert!(audio.capture.as_ref().unwrap().is_empty());
    assert_eq!(audio.random.next_long(), expected.next_long());
}
#[test]
fn removal_marks_only_existing_references_to_that_entity() {
    use crate::sound_engine::SoundEntity;
    let mut audio = Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
        (Path::new("/nonexistent/horizon-entity-sound-test")).to_path_buf(),
    ));
    let old = SoundEntity::new(7, [0.0; 3], false);
    let other = SoundEntity::new(8, [0.0; 3], false);
    for entity in [old.clone(), other.clone()] {
        audio.engine.add(
            SoundInstance::entity_bound(
                "minecraft:fixture.bound",
                6,
                1.0,
                1.0,
                LegacyRandom::new(42),
                entity,
            ),
            true,
        );
    }
    audio.remove_sound_entity(7);
    let replacement = SoundEntity::new(7, [10.0; 3], false);
    assert!(old.state().removed);
    assert!(!other.state().removed);
    assert!(!replacement.state().removed);
}

#[test]
fn button_click_uses_vanilla_ui_parameters_without_advancing_world_random() {
    let mut audio = Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
        (Path::new("/nonexistent/horizon-test-assets")).to_path_buf(),
    ));
    audio.engine.registry.add_file(
        "minecraft",
        br#"{"ui.button.click":{"sounds":["gui/button"]}}"#,
        &|_, _| true,
    );
    // Exercise the mixer without opening an output device. The missing
    // sample is harmless: this test checks the source the engine creates.
    audio.engine.loaded = true;
    audio.backend.live = true;
    let mut random = audio.random.clone();
    audio.play_button_click();
    let instance = audio.engine.instance(1).expect("click reached the engine");
    assert_eq!(instance.identifier, "minecraft:ui.button.click");
    assert_eq!(instance.source, crate::sound_engine::UI);
    assert_eq!(instance.volume, 0.25);
    assert_eq!(instance.pitch, 1.0);
    assert_eq!(instance.attenuation, Attenuation::None);
    assert_eq!(instance.position, [0.0; 3]);
    assert!(instance.relative);
    assert!(!instance.looping);
    assert_eq!(audio.random.next_long(), random.next_long());
    audio.pause();
    assert_eq!(
        audio.backend.mixer.lock().unwrap().channels[&1].state,
        State::Initial
    );
    audio.stop_all();
}

/// A client mod's `SoundPlay` reaches the engine as it says: an interface sound at once,
/// at the listener; a positioned, looping one on its own source, queued `after` ticks.
#[test]
fn a_mod_sound_plays_as_its_sound_play_says() {
    use mod_api::{SoundPlay, SoundSource};
    let mut audio = Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
        (Path::new("/nonexistent/horizon-test-assets")).to_path_buf(),
    ));
    audio.engine.registry.add_file(
        "minecraft",
        br#"{"ui.button.click":{"sounds":["gui/button"]}}"#,
        &|_, _| true,
    );
    audio.engine.registry.add_file(
        "mymod",
        br#"{"boom":{"sounds":["mymod:boom"]}}"#,
        &|_, _| true,
    );
    audio.engine.loaded = true;
    audio.backend.live = true;

    audio.play_mod_sound(
        &SoundPlay::ui("minecraft:ui.button.click").with_volume(0.5),
        0,
    );
    let click = audio
        .engine
        .instance(1)
        .expect("the click reached the engine");
    assert_eq!(click.source, crate::sound_engine::UI);
    assert_eq!((click.volume, click.pitch), (0.5, 1.0));
    assert_eq!(click.attenuation, Attenuation::None);
    assert!(click.relative && !click.looping);
    assert_eq!(audio.engine.census()["queued"], 0);

    let boom = SoundPlay::at("mymod:boom", SoundSource::Hostile, [1.0, 64.0, 2.0])
        .with_pitch(1.5)
        .looping(20);
    audio.play_mod_sound(&boom, 40);
    let queued = audio
        .engine
        .instance(2)
        .expect("the boom is held for later");
    assert_eq!(queued.source, SoundSource::Hostile as usize);
    assert_eq!(queued.position, [1.0, 64.0, 2.0]);
    assert_eq!((queued.pitch, queued.delay), (1.5, 20));
    assert_eq!(queued.attenuation, Attenuation::Linear);
    assert!(queued.looping && !queued.relative);
    assert_eq!(audio.engine.census()["queued"], 1, "it waits its 40 ticks");
    audio.stop_all();
}

fn channel(samples: Vec<f32>, linear: Option<f32>, at: [f32; 3], looping: bool) -> Channel {
    Channel {
        samples: Some((Arc::new(samples), 48_000, 1)),
        position: 0.0,
        state: State::Playing,
        gain: 1.0,
        pitch: 1.0,
        linear,
        looping,
        at,
        relative: false,
        owner: 0,
        world: 0,
        started: None,
        unmuffled: false,
    }
}

/// A channel whose buffer is interleaved stereo.
fn stereo_channel(samples: Vec<f32>, linear: Option<f32>, at: [f32; 3], looping: bool) -> Channel {
    let mut c = channel(Vec::new(), linear, at, looping);
    c.samples = Some((Arc::new(samples), 48_000, 2));
    c
}

#[test]
fn decoded_sounds_stay_within_the_budget_least_recently_used_out() {
    let mut cache = SoundCache::default();
    let quarter = CACHE_BYTES / 4 / std::mem::size_of::<f32>();
    let sound = || Some((Arc::new(vec![0.0f32; quarter]), 44100, 1));
    for key in ["a", "b", "c", "d"] {
        cache.insert(key.into(), sound());
    }
    assert!(cache.get("a").is_some());
    cache.insert("e".into(), sound());
    assert!(cache.bytes <= CACHE_BYTES);
    assert!(
        cache.get("b").is_none(),
        "the least recently used goes first"
    );
    assert!(cache.get("a").is_some() && cache.get("e").is_some());
    // A failed load is remembered without counting.
    cache.insert("missing".into(), None);
    assert_eq!(cache.get("missing"), Some(None));
}
#[test]
fn output_stays_off_on_a_virtual_screen_unless_asked() {
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |name: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| std::ffi::OsString::from(*v))
        }
    };
    assert!(output_disabled(env(&[("WAYLAND_DISPLAY", "wayland-1")])).is_none());
    assert!(output_disabled(env(&[])).is_none());
    const VSCREEN: &str = "/run/user/1000/vscreen/main/wayland-1";
    assert!(output_disabled(env(&[("WAYLAND_DISPLAY", VSCREEN)])).is_some());
    assert!(
        output_disabled(env(&[("WAYLAND_DISPLAY", VSCREEN), ("HORIZON_AUDIO", "1")])).is_none()
    );
    assert!(output_disabled(env(&[("HORIZON_NO_AUDIO", "1"), ("HORIZON_AUDIO", "1")])).is_some());
}
#[test]
#[cfg(feature = "audio")]
fn buffer_size_override_and_defaults() {
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |name: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| std::ffi::OsString::from(*v))
        }
    };
    let range = (256, 4096);
    assert_eq!(
        target_buffer_size(range, 0, env(&[("HORIZON_AUDIO_BUFFER", "default")])),
        Some(cpal::BufferSize::Default)
    );
    assert_eq!(
        target_buffer_size(range, 0, env(&[("HORIZON_AUDIO_BUFFER", "2048")])),
        Some(cpal::BufferSize::Fixed(2048))
    );
    assert_eq!(
        target_buffer_size(range, 0, env(&[("HORIZON_AUDIO_BUFFER", "100")])),
        Some(cpal::BufferSize::Fixed(256))
    );
    assert_eq!(
        target_buffer_size(range, 0, env(&[("HORIZON_AUDIO_BUFFER", "10000")])),
        Some(cpal::BufferSize::Fixed(4096))
    );
    #[cfg(target_os = "linux")]
    assert_eq!(
        target_buffer_size(range, 0, env(&[])),
        Some(cpal::BufferSize::Fixed(1024))
    );
    #[cfg(not(target_os = "linux"))]
    assert_eq!(target_buffer_size(range, 0, env(&[])), None);
    // After the stream stopped on underruns: twice as long each time, within the device's
    // range; an override stays as given.
    #[cfg(target_os = "linux")]
    {
        assert_eq!(
            target_buffer_size(range, 1, env(&[])),
            Some(cpal::BufferSize::Fixed(2048))
        );
        assert_eq!(
            target_buffer_size(range, 9, env(&[])),
            Some(cpal::BufferSize::Fixed(4096))
        );
        assert_eq!(
            target_buffer_size((256, 3000), 2, env(&[])),
            Some(cpal::BufferSize::Fixed(3000))
        );
    }
    assert_eq!(
        target_buffer_size(range, 2, env(&[("HORIZON_AUDIO_BUFFER", "512")])),
        Some(cpal::BufferSize::Fixed(512))
    );
}
#[test]
fn linear_attenuation_reaches_zero_at_the_max_distance() {
    assert_eq!(distance_gain(None, 100.0), 1.0);
    assert_eq!(distance_gain(Some(16.0), 0.0), 1.0);
    assert_eq!(distance_gain(Some(16.0), 8.0), 0.5);
    assert_eq!(distance_gain(Some(16.0), 16.0), 0.0);
    assert_eq!(distance_gain(Some(16.0), 40.0), 0.0);
}

#[test]
fn overlapping_voices_are_limited_without_flattening_or_changing_stereo_balance() {
    let mut m = Mixer::default();
    let wave = vec![0.25, 0.5, 0.75, 1.0, -1.0, -0.75, -0.5, -0.25, 0.0];
    for id in 1..=4 {
        m.channels
            .insert(id, channel(wave.clone(), None, [-1.0, 0.0, 0.0], false));
    }
    let mut data = [0.0; 16];
    mix(&mut m, &mut data, 2, 48_000);
    // West of a listener facing south: fully on the right.
    for (frame, sample) in data.as_chunks::<2>().0.iter().zip(wave) {
        assert!(frame[0].abs() < 1e-6, "{frame:?}");
        assert!((frame[1] - sample).abs() < 1e-6, "{frame:?}");
    }
    assert!(data.iter().all(|s| s.abs() <= 1.0));
}

#[test]
fn limiter_leaves_quiet_audio_alone_and_recovers_across_callbacks() {
    let mut m = Mixer::default();
    m.channels
        .insert(1, channel(vec![0.25; 2], None, [0.0; 3], true));
    let quiet = 0.25 * stereo_gains(0.0, 0.0, 1.0)[0];
    let mut data = [0.0; 16];
    mix(&mut m, &mut data, 2, 48_000);
    assert_eq!(data, [quiet; 16]);

    for id in 2..=5 {
        m.channels
            .insert(id, channel(vec![1.0; 2], None, [0.0; 3], true));
    }
    mix(&mut m, &mut data, 2, 48_000);
    assert!(data.iter().all(|s| s.abs() <= 1.0));
    m.channels.retain(|&id, _| id == 1);
    let mut recovery = vec![0.0; 2 * 48_000];
    mix(&mut m, &mut recovery, 2, 48_000);
    assert!(recovery[0] > 0.0 && recovery[0] < 0.1);
    assert!(recovery.windows(2).all(|w| w[1] >= w[0]));
    assert_eq!(*recovery.last().unwrap(), quiet);
}

#[test]
fn sources_play_attenuated_then_stop_or_loop() {
    let mut m = Mixer::default();
    m.channels
        .insert(1, channel(vec![1.0; 4], Some(16.0), [8.0, 0.0, 0.0], false));
    m.channels
        .insert(2, channel(vec![1.0; 4], None, [0.0; 3], true));
    let mut data = [0.0f32; 16];
    mix(&mut m, &mut data, 2, 48_000);
    assert_eq!(m.channels[&1].state, State::Stopped);
    assert_eq!(m.channels[&2].state, State::Playing);
    // Mono sum of the two at the first frame: the far one is half as loud.
    let first = data[0] + data[1];
    assert!(first > 1.4 && first < 3.1, "{first}");
}

#[test]
fn stereo_panning_matches_openal_soft() {
    // OpenAL Soft 1.25.2 (LWJGL 3.4.3's macOS build), stereo loopback
    // device with vanilla's attributes, 1 kHz sine 4 blocks away.
    let cases = [
        ((0.0, 0.0, 1.0), [0.5957, 0.5957]),
        ((-1.0, 0.0, 0.0), [1.0, 0.0]),
        ((1.0, 0.0, 0.0), [0.0, 1.0]),
        ((0.0, 0.0, -1.0), [0.4043, 0.4043]),
        ((0.0, 1.0, 0.0), [0.5, 0.5]),
        ((0.0, -1.0, 0.0), [0.5, 0.5]),
        ((-1.0, 0.0, 1.0), [0.9985, 0.0747]),
        ((-1.0, 0.0, -1.0), [0.7859, 0.0788]),
        ((0.0, 0.0, 0.0), [0.5957, 0.5957]),
    ];
    for ((right, up, forward), expected) in cases {
        let gains = stereo_gains(right, up, forward);
        for (g, e) in gains.into_iter().zip(expected) {
            assert!((g - e).abs() < 1e-4, "{right} {up} {forward}: {gains:?}");
        }
    }
}

#[test]
fn stereo_sounds_play_direct_without_panning_or_distance() {
    // OpenAL Soft plays a multi-channel buffer direct (its non-spatialized path), so vanilla's
    // stereo music and records keep their own left/right mix at full volume whichever way the
    // camera faces and however far it is. Only mono buffers are panned and attenuated.
    let wave = [0.1, -0.2, 0.3, -0.4, 0.5, -0.6]; // three stereo frames
    let expected = [wave[0], wave[1], wave[2], wave[3]];
    for (yaw, at, linear, listener) in [
        (0.0, [100.0, 0.0, 0.0], Some(16.0), [0.0; 3]),
        (180.0, [0.0, 0.0, -100.0], Some(16.0), [0.0; 3]),
        (90.0, [2.0, 1.0, -3.0], Some(16.0), [5.0, 60.0, 5.0]),
        (0.0, [0.0, 0.0, 0.0], None, [0.0; 3]),
    ] {
        let mut m = Mixer {
            listener,
            listener_yaw: yaw,
            ..Mixer::default()
        };
        m.channels
            .insert(1, stereo_channel(wave.to_vec(), linear, at, false));
        let mut data = [0.0; 4];
        mix(&mut m, &mut data, 2, 48_000);
        assert_eq!(data, expected, "yaw {yaw}, source {at:?}");
    }
}

#[test]
fn panning_follows_the_camera_rotation() {
    let gains = |yaw: f32, pitch: f32, at: [f32; 3]| {
        let mut m = Mixer {
            listener_yaw: yaw,
            listener_pitch: pitch,
            ..Mixer::default()
        };
        m.channels.insert(1, channel(vec![1.0; 4], None, at, true));
        let mut data = [0.0; 2];
        mix(&mut m, &mut data, 2, 48_000);
        data
    };
    let close = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4;
    // Facing south (yaw 0), east is on the left; facing north, on the right.
    assert!(close(gains(0.0, 0.0, [1.0, 0.0, 0.0]), [1.0, 0.0]));
    assert!(close(gains(180.0, 0.0, [1.0, 0.0, 0.0]), [0.0, 1.0]));
    // Looking straight down, a source below is ahead and one south is above.
    assert!(close(gains(0.0, 90.0, [0.0, -1.0, 0.0]), [0.5957, 0.5957]));
    assert!(close(gains(0.0, 90.0, [0.0, 0.0, 1.0]), [0.5, 0.5]));
}

#[test]
fn splitscreen_sounds_are_heard_from_the_nearest_camera() {
    // P1 far away, a second player beside the source: heard at full gain, centred.
    let mut m = Mixer {
        listener: [100.0, 0.0, 0.0],
        ..Mixer::default()
    };
    m.listeners.insert(1, ([0.0, 0.0, 1.0], 0.0, 0.0));
    m.channels
        .insert(1, channel(vec![1.0; 4], Some(16.0), [0.0, 0.0, 2.0], true));
    let mut data = [0.0; 2];
    mix(&mut m, &mut data, 2, 48_000);
    assert!(
        data[0] > 0.5 && (data[0] - data[1]).abs() < 1e-4,
        "{data:?}"
    );
    // Alone, P1 would not hear it: 100 blocks past its 16.
    m.listeners.clear();
    let mut data = [0.0; 2];
    mix(&mut m, &mut data, 2, 48_000);
    assert_eq!(data, [0.0; 2]);
}

#[test]
fn a_shared_listener_registers_even_in_the_hosts_world() {
    let mut host = Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
        (Path::new("/nonexistent/horizon-test-audio")).to_path_buf(),
    ));
    host.set_world("minecraft:overworld");
    let mut guest = host.share(1);
    guest.set_world("minecraft:overworld");
    guest.set_listener([1.0, 2.0, 3.0], 0.0, 0.0);
    let id = guest.backend.create(false).unwrap();
    let mixer = host.backend.mixer.lock().unwrap();
    let overworld = mixer
        .world_names
        .iter()
        .position(|w| w == "minecraft:overworld");
    let overworld = overworld.map(|i| i as u32 + 1);
    assert_eq!(mixer.worlds.get(&1).copied(), overworld);
    assert_eq!(Some(mixer.channels[&id].world), overworld);
}

#[test]
fn cameras_in_other_dimensions_cannot_hear_a_source() {
    let mut m = Mixer {
        listener: [100.0, 0.0, 0.0],
        ..Default::default()
    };
    m.listeners.insert(1, ([0.0, 0.0, 1.0], 0.0, 0.0));
    let nether = m.world_id("nether");
    m.worlds.insert(1, nether);
    m.channels
        .insert(1, channel(vec![1.0; 4], Some(16.0), [0.0, 0.0, 2.0], true));
    let mut data = [0.0; 2];
    mix(&mut m, &mut data, 2, 48_000);
    assert_eq!(
        data, [0.0; 2],
        "nearby Nether camera must not hear Overworld source"
    );
    m.channels.get_mut(&1).unwrap().world = nether;
    mix(&mut m, &mut data, 2, 48_000);
    assert!(data[0] > 0.5, "same-world camera must hear source");
    let end = m.world_id("end");
    m.worlds.insert(1, end);
    data.fill(0.0);
    mix(&mut m, &mut data, 2, 48_000);
    assert_eq!(data, [0.0; 2], "no listener remains in source's world");
}

#[test]
fn sounds_in_a_world_without_listeners_finish_while_inaudible() {
    let mut mixer = Mixer::default();
    let mut source = channel(vec![1.0; 4], None, [0.0; 3], false);
    let nether = mixer.world_id("nether");
    source.world = nether;
    mixer.channels.insert(1, source);
    let mut data = [0.0; 4];
    mix(&mut mixer, &mut data, 2, 48_000);
    assert_eq!(data, [0.0; 4]);
    assert!(mixer.channels[&1].position > 0.0);
    mix(&mut mixer, &mut data, 2, 48_000);
    assert_eq!(mixer.channels[&1].state, State::Stopped);
    mixer.worlds.insert(0, nether);
    mix(&mut mixer, &mut data, 2, 48_000);
    assert_eq!(data, [0.0; 4], "returning must not resume the old one-shot");
}

/// A sound of one frame or none, heard or not, looping or not, stops at its first
/// buffer as the mixing loop stops it: none stays playing (and holds its channel).
#[test]
fn sounds_too_short_to_mix_stop_even_unheard() {
    for (samples, looping) in [(vec![], false), (vec![1.0], false), (vec![1.0], true)] {
        let mut mixer = Mixer::default();
        let mut source = channel(samples.clone(), None, [0.0; 3], looping);
        source.gain = 0.0;
        mixer.channels.insert(1, source);
        let mut data = [0.0; 4];
        mix(&mut mixer, &mut data, 2, 48_000);
        assert_eq!(
            mixer.channels[&1].state,
            State::Stopped,
            "{samples:?} {looping}"
        );
    }
}

#[test]
fn variants_of_one_sound_are_the_same_sound() {
    assert_eq!(
        variantless("minecraft:mob/creeper/say1.ogg"),
        "minecraft:mob/creeper/say"
    );
    assert_eq!(
        variantless("minecraft:step/grass6.ogg"),
        "minecraft:step/grass"
    );
    assert_eq!(
        variantless("minecraft:random/click.ogg"),
        "minecraft:random/click"
    );
}

#[test]
fn another_players_copy_of_a_sound_is_dropped() {
    let mixer = Arc::new(Mutex::new(Mixer::default()));
    // One other player shares the output.
    let shares = Arc::new(std::sync::atomic::AtomicUsize::new(1));
    let backend = |owner| MixerBackend {
        mixer: mixer.clone(),
        root: ResourceSource::indexed(Default::default()),
        cache: Arc::default(),
        decoder: None,
        next: (u64::from(owner)) << 40,
        live: true,
        owner,
        world: 0,
        shares: shares.clone(),
        mod_pcm: HashMap::new(),
        mod_pcm_next: 0,
    };
    let (mut p1, mut p2) = (backend(0), backend(1));
    // P2 is listening (its camera is in the mixer), so two players share the speakers.
    mixer
        .lock()
        .unwrap()
        .listeners
        .insert(1, ([0.0; 3], 0.0, 0.0));
    let started = |b: &mut MixerBackend, at: [f64; 3]| {
        let id = b.create(false).unwrap();
        b.set_position(id, at);
        !b.duplicate(id, ("minecraft", "mob/creeper/say1.ogg"))
    };
    assert!(started(&mut p1, [5.0, 64.0, 5.0]));
    // P2's client got the same sound at the same place: one copy plays.
    assert!(!started(&mut p2, [5.2, 64.0, 5.0]));
    // Elsewhere, or P1's own second one, it is a sound of its own.
    assert!(started(&mut p2, [40.0, 64.0, 5.0]));
    assert!(started(&mut p1, [5.0, 64.0, 5.0]));
    // Equal coordinates in another dimension describe a different sound.
    p2.world = mixer.lock().unwrap().world_id("nether");
    assert!(started(&mut p2, [5.0, 64.0, 5.0]));
    // P2 leaving takes its sounds.
    drop(p2);
    assert!(
        mixer
            .lock()
            .unwrap()
            .channels
            .values()
            .all(|c| c.owner == 0)
    );
}

#[test]
fn reusing_a_seat_does_not_reuse_its_channels_or_leave_its_camera() {
    let host = Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
        (Path::new("/nonexistent/horizon-test-audio")).to_path_buf(),
    ));
    let mut guest = host.share(1);
    guest.set_world("overworld");
    guest.set_listener([1.0, 0.0, 0.0], 0.0, 0.0);
    let old = guest.backend.create(false).unwrap();
    drop(guest);
    {
        let mixer = host.backend.mixer.lock().unwrap();
        assert!(!mixer.channels.contains_key(&old));
        assert!(!mixer.listeners.contains_key(&1));
        assert!(!mixer.worlds.contains_key(&1));
    }
    let mut replacement = host.share(1);
    let new = replacement.backend.create(false).unwrap();
    assert_ne!(old, new);
    assert!(
        replacement
            .backend
            .mixer
            .lock()
            .unwrap()
            .channels
            .contains_key(&new)
    );
}

#[test]
fn a_mono_output_plays_sources_unpanned() {
    let mut m = Mixer::default();
    m.channels
        .insert(1, channel(vec![0.5; 4], None, [0.0, 0.0, -3.0], true));
    let mut data = [0.0; 2];
    mix(&mut m, &mut data, 1, 48_000);
    assert_eq!(data, [0.5; 2]);
}

/// One second of a `hz` sine at 48 kHz, amplitude 0.3 (looped, it repeats seamlessly).
fn sine(hz: f32) -> Vec<f32> {
    (0..48_000)
        .map(|i| 0.3 * (std::f32::consts::TAU * hz * i as f32 / 48_000.0).sin())
        .collect()
}

fn rms(data: &[f32]) -> f32 {
    (data.iter().map(|s| s * s).sum::<f32>() / data.len() as f32).sqrt()
}

/// `seconds` of the mixer's stereo output, in the 10 ms buffers an output stream asks for.
fn listen(m: &mut Mixer, seconds: f32) -> Vec<f32> {
    let mut out = Vec::new();
    let mut buffer = [0.0; 960];
    for _ in 0..(seconds * 100.0).round() as usize {
        mix(m, &mut buffer, 2, 48_000);
        out.extend_from_slice(&buffer);
    }
    out
}

#[test]
fn a_muffle_dulls_and_quietens_every_sound_but_the_exempt_ones() {
    // A looping tone heard through a muffle of `[amount, lowpass]`, once it has settled.
    let heard = |hz: f32, muffle: [f32; 2], unmuffled: bool| {
        let mut m = Mixer::default();
        let mut c = channel(sine(hz), None, [0.0, 0.0, 2.0], true);
        c.unmuffled = unmuffled;
        m.channels.insert(1, c);
        m.muffle.target = muffle;
        listen(&mut m, 0.2);
        listen(&mut m, 0.2)
    };
    let plain = |hz: f32| rms(&heard(hz, [0.0; 2], false));
    // Fully low-passed, a 6 kHz whine all but goes and a 100 Hz hum keeps most of its level.
    assert!(rms(&heard(6000.0, [0.0, 1.0], false)) < plain(6000.0) * 0.05);
    assert!(rms(&heard(100.0, [0.0, 1.0], false)) > plain(100.0) * 0.8);
    // Half the volume taken and no low-pass: half the level, the tone itself untouched.
    let ducked = rms(&heard(6000.0, [0.5, 0.0], false)) / plain(6000.0);
    assert!((ducked - 0.5).abs() < 1e-3, "{ducked}");
    // An exempt sound plays through as if there were no muffle, sample for sample.
    assert_eq!(
        heard(6000.0, [0.5, 1.0], true),
        heard(6000.0, [0.0; 2], false)
    );
}

#[test]
fn a_faded_muffle_leaves_the_sound_as_it_was() {
    let mixer = || {
        let mut m = Mixer::default();
        m.channels
            .insert(1, channel(sine(3000.0), None, [2.0, 0.0, 1.0], true));
        m
    };
    let (mut muffled, mut plain) = (mixer(), mixer());
    muffled.muffle.target = [0.7, 1.0];
    assert_ne!(listen(&mut muffled, 0.1), listen(&mut plain, 0.1));
    muffled.muffle.target = [0.0; 2];
    listen(&mut muffled, 0.5);
    listen(&mut plain, 0.5);
    assert_eq!(listen(&mut muffled, 0.1), listen(&mut plain, 0.1));
}

#[test]
fn a_mods_muffle_fades_with_the_game_ticks_and_spares_its_exempt_sounds() {
    let mut audio = Audio::silent(&ResourceSource::directory(
        "/nonexistent/horizon-test-assets".into(),
    ));
    audio.engine.registry.add_file(
        "minecraft",
        br#"{"ambient.cave":{"sounds":["ambient/cave/cave1"]}}"#,
        &|_, _| true,
    );
    audio.engine.registry.add_file(
        "hztest",
        br#"{"ear.ringing":{"sounds":["ear/ringing"]}}"#,
        &|_, _| true,
    );
    // The engine makes its channels; with no output, none loads a file.
    audio.engine.loaded = true;
    let marks = |audio: &Audio| {
        let m = audio.backend.mixer.lock().unwrap();
        let mut marks: Vec<_> = m
            .channels
            .iter()
            .map(|(&id, c)| (id, c.unmuffled))
            .collect();
        marks.sort();
        marks
    };
    let level = |audio: &Audio| audio.backend.mixer.lock().unwrap().muffle.target;
    let close = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6;
    // Two sounds already playing, and one exempt sound started under the muffle.
    audio.play("ambient.cave", None, 1.0, 1.0);
    audio.play("hztest:ear.ringing", None, 1.0, 1.0);
    assert!(audio.muffle(0.8, 1.0, 4, &["hztest:ear.ringing"]));
    audio.play("hztest:ear.ringing", None, 1.0, 1.0);
    assert_eq!(marks(&audio), [(1, false), (2, true), (3, true)]);
    assert!(close(level(&audio), [0.8, 1.0]));
    let ticks = TickContext::default();
    audio.tick(false, &ticks);
    assert!(close(level(&audio), [0.6, 0.75]), "{:?}", level(&audio));
    // A paused game holds it.
    audio.tick(true, &ticks);
    assert!(close(level(&audio), [0.6, 0.75]), "{:?}", level(&audio));
    for _ in 0..3 {
        audio.tick(false, &ticks);
    }
    assert_eq!(level(&audio), [0.0; 2]);
    // Over: its exemptions go with it.
    assert_eq!(marks(&audio), [(1, false), (2, false), (3, false)]);
    // Out of range changes nothing.
    assert!(!audio.muffle(1.5, 0.0, 10, &[]));
    assert_eq!(level(&audio), [0.0; 2]);
}

#[test]
fn each_mod_audio_source_plays_its_own_chunks_in_order() {
    let mut m = Mixer::default();
    // Two speakers of one mod, each queuing two chunks, interleaved.
    for (source, level) in [(1, 0.1), (2, 0.4), (1, 0.2), (2, 0.3)] {
        assert!(m.push_mod_audio("voice", source, &[level; 4], 48_000, None));
    }
    let mut data = vec![0.0f32; 8 * 2];
    mix(&mut m, &mut data, 2, 48_000);
    // Each speaker's first chunk, then its second: never both of one speaker at once.
    let left: Vec<f32> = data.chunks(2).map(|f| f[0]).collect();
    let expected = [0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5];
    assert!(
        left.iter().zip(expected).all(|(a, b)| (a - b).abs() < 1e-6),
        "{left:?}"
    );
    assert!(m.mod_audio.is_empty(), "both sources played out");
    // Order within a source: its second chunk follows its first.
    assert!(m.push_mod_audio("voice", 1, &[0.25; 2], 48_000, None));
    assert!(m.push_mod_audio("voice", 1, &[0.5; 2], 48_000, None));
    let mut data = vec![0.0f32; 4 * 2];
    mix(&mut m, &mut data, 2, 48_000);
    let left: Vec<f32> = data.chunks(2).map(|f| f[0]).collect();
    assert_eq!(left, [0.25, 0.25, 0.5, 0.5]);
}

#[test]
fn mod_audio_sources_are_kept_apart_by_mod_and_capped() {
    let mut m = Mixer::default();
    assert!(m.push_mod_audio("a", 1, &[0.1; 48_000], 48_000, None));
    assert!(m.push_mod_audio("b", 1, &[0.1; 48_000], 48_000, None));
    assert_eq!(
        m.mod_audio.len(),
        2,
        "the same id in two mods is two sources"
    );
    // Two seconds wait at most per source; a burst past that is refused, not delayed.
    assert!(m.push_mod_audio("a", 1, &[0.1; 48_000], 48_000, None));
    assert!(!m.push_mod_audio("a", 1, &[0.1; 1], 48_000, None));
    assert!(!m.push_mod_audio("a", 2, &[], 48_000, None));
    assert!(!m.push_mod_audio("a", 2, &[0.1], 0, None));
}

#[test]
fn mod_audio_is_panned_and_faded_by_where_it_was_spoken() {
    let mut m = Mixer::default();
    // Facing south (yaw 0), -X is the listener's right.
    assert!(m.push_mod_audio("voice", 1, &[0.5; 16], 48_000, Some([-4.0, 0.0, 0.0])));
    let mut data = vec![0.0f32; 16 * 2];
    mix(&mut m, &mut data, 2, 48_000);
    let left: f32 = data.chunks(2).map(|f| f[0]).sum();
    let right: f32 = data.chunks(2).map(|f| f[1]).sum();
    assert!(right > left, "right {right} should beat left {left}");
    // Past the range it is silent.
    assert!(m.push_mod_audio("voice", 1, &[0.5; 16], 48_000, Some([0.0, 0.0, 60.0])));
    let mut data = vec![0.0f32; 16 * 2];
    mix(&mut m, &mut data, 2, 48_000);
    assert!(data.iter().all(|s| *s == 0.0));
}

#[test]
fn output_devices_are_looked_at_again_only_when_the_cards_change() {
    // No fingerprint (not Linux, or the cards could not be read): the list
    // has to be looked at as before.
    assert!(cards_changed(None, None));
    assert!(cards_changed(Some("0 [PCH ]"), None));
    // The same cards list the same devices; a card added or removed does not.
    assert!(!cards_changed(Some("0 [PCH ]"), Some("0 [PCH ]")));
    assert!(cards_changed(Some("0 [PCH ]"), Some("0 [PCH ]\n 1 [USB ]")));
}

/// The mixer live without an output device, and the button click's event: a sound
/// holds a channel, and a stopped channel is let go at the next update.
fn live_audio() -> Audio {
    let mut audio = Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
        (Path::new("/nonexistent/horizon-test-assets")).to_path_buf(),
    ));
    audio.engine.registry.add_file(
        "minecraft",
        br#"{"ui.button.click":{"sounds":["gui/button"]}}"#,
        &|_, _| true,
    );
    audio.engine.loaded = true;
    audio.backend.live = true;
    audio
}

#[test]
fn a_mod_sound_waits_out_its_delay_plays_and_stops_for_good() {
    use mod_api::SoundState::*;
    let mut audio = live_audio();
    let ctx = TickContext::default();
    let click = mod_api::SoundPlay::ui("minecraft:ui.button.click");
    let sound = audio.play_mod_sound(&click, 2);
    assert_eq!(audio.mod_sound_state(sound), Waiting);
    audio.tick(false, &ctx);
    assert_eq!(audio.mod_sound_state(sound), Waiting, "one tick of two");
    audio.tick(false, &ctx);
    assert_eq!(audio.mod_sound_state(sound), Playing);
    audio.stop_mod_sound(sound);
    audio.tick(false, &ctx);
    audio.tick(false, &ctx);
    assert_eq!(audio.mod_sound_state(sound), Stopped);
    // Stopped while it waits, it never starts; a looping one never repeats.
    let never = audio.play_mod_sound(&click.clone().looping(0), 3);
    audio.stop_mod_sound(never);
    assert_eq!(audio.mod_sound_state(never), Stopped);
    for _ in 0..5 {
        audio.tick(false, &ctx);
        assert_eq!(audio.mod_sound_state(never), Stopped);
    }
}

#[test]
fn a_mod_sound_fades_to_its_volume_and_a_fade_to_zero_stops_it() {
    use mod_api::SoundState::*;
    let mut audio = live_audio();
    let ctx = TickContext::default();
    let sound = audio.play_mod_sound(&mod_api::SoundPlay::ui("minecraft:ui.button.click"), 0);
    assert_eq!(audio.mod_sound_state(sound), Playing);
    let volume = |audio: &Audio| audio.engine.instance(sound).map(|i| i.volume);
    audio.fade_mod_sound(sound, 0.5, 2);
    let mut volumes = Vec::new();
    for _ in 0..3 {
        audio.tick(false, &ctx);
        volumes.push(volume(&audio));
    }
    assert_eq!(
        volumes,
        [Some(0.75), Some(0.5), Some(0.5)],
        "two equal steps, then held"
    );
    // Paused, a fade waits.
    audio.fade_mod_sound(sound, 0.0, 2);
    audio.tick(true, &ctx);
    assert_eq!(volume(&audio), Some(0.5));
    audio.tick(false, &ctx);
    assert_eq!(volume(&audio), Some(0.25));
    assert_eq!(audio.mod_sound_state(sound), Playing);
    for _ in 0..4 {
        audio.tick(false, &ctx);
    }
    assert_eq!(audio.mod_sound_state(sound), Stopped, "faded out");
    // A fade at once: to a volume sets it, to 0 stops it.
    let other = audio.play_mod_sound(&mod_api::SoundPlay::ui("minecraft:ui.button.click"), 0);
    audio.fade_mod_sound(other, 0.3, 0);
    assert_eq!(volume_of(&audio, other), Some(0.3));
    audio.fade_mod_sound(other, 0.0, 0);
    for _ in 0..2 {
        audio.tick(false, &ctx);
    }
    assert_eq!(audio.mod_sound_state(other), Stopped);
}

/// A sound that repeats after a gap (`looping` with a delay) keeps its handle across the
/// gap: once its sample ends it waits for the repeat, plays again, and still stops.
#[test]
fn a_repeating_mod_sound_waits_between_its_repeats_and_plays_again() {
    use mod_api::SoundState::*;
    let mut audio = live_audio();
    let ctx = TickContext::default();
    let click = mod_api::SoundPlay::ui("minecraft:ui.button.click").looping(2);
    let sound = audio.play_mod_sound(&click, 0);
    assert_eq!(audio.mod_sound_state(sound), Playing);
    // The sample ends on its own, as the mixer stops a channel that runs out.
    for channel in audio.backend.mixer.lock().unwrap().channels.values_mut() {
        channel.state = State::Stopped;
    }
    let mut states = Vec::new();
    for _ in 0..30 {
        audio.tick(false, &ctx);
        states.push(audio.mod_sound_state(sound));
    }
    assert_eq!(states[0], Waiting, "{states:?}");
    assert!(!states.contains(&Stopped), "{states:?}");
    assert_eq!(states.last(), Some(&Playing), "repeated: {states:?}");
    audio.stop_mod_sound(sound);
    audio.tick(false, &ctx);
    assert_eq!(audio.mod_sound_state(sound), Stopped);
}

/// A mod's own samples (`play_mod_pcm`) need no file: they start at once from the frame
/// asked, follow `move_mod_sound`, repeat from the first frame after a loop's delay, and are
/// let go once the play is over.
#[test]
fn a_mod_pcm_play_starts_where_asked_follows_and_is_let_go() {
    use mod_api::SoundState::*;
    let mut audio = live_audio();
    let ctx = TickContext::default();
    let samples = Arc::new((0..4800).map(|i| i as f32 / 4800.0).collect::<Vec<_>>());
    let play = mod_api::SoundPlay::at(
        "mymod:song",
        mod_api::SoundSource::Players,
        [1.0, 64.0, 2.0],
    )
    .looping(2);
    let song = audio.play_mod_pcm(&play, samples, 48_000, 1, 1200, 0);
    assert_eq!(audio.mod_sound_state(song), Playing);
    let channel = |audio: &Audio| {
        let m = audio.backend.mixer.lock().unwrap();
        m.channels.values().find(|c| c.samples.is_some()).map(|c| {
            let (samples, rate, channels) = c.samples.as_ref().unwrap();
            (c.state, c.position, c.at, samples.len(), *rate, *channels)
        })
    };
    assert_eq!(
        channel(&audio),
        Some((State::Playing, 1200.0, [1.0, 64.0, 2.0], 4800, 48_000, 1))
    );
    audio.move_mod_sound(song, [5.0, 70.0, -3.0]);
    audio.tick(false, &ctx);
    assert_eq!(channel(&audio).map(|c| c.2), Some([5.0, 70.0, -3.0]));
    // The clip ends; after the loop's delay it plays again from its first frame.
    for channel in audio.backend.mixer.lock().unwrap().channels.values_mut() {
        channel.state = State::Stopped;
    }
    let mut states = Vec::new();
    for _ in 0..30 {
        audio.tick(false, &ctx);
        states.push(audio.mod_sound_state(song));
    }
    assert_eq!(states.last(), Some(&Playing), "repeated: {states:?}");
    assert_eq!(
        channel(&audio).map(|c| (c.0, c.1)),
        Some((State::Playing, 0.0))
    );
    assert_eq!(audio.backend.mod_pcm.len(), 1, "kept for its repeats");
    audio.stop_mod_sound(song);
    audio.tick(false, &ctx);
    assert_eq!(audio.mod_sound_state(song), Stopped);
    audio.tick(false, &ctx);
    assert!(audio.backend.mod_pcm.is_empty(), "let go");
}

/// A mod sound at volume 0 starts silent and keeps its handle, so a fade raises it.
#[test]
fn a_silent_mod_sound_starts_and_a_fade_raises_it() {
    use mod_api::SoundState::*;
    let mut audio = live_audio();
    let ctx = TickContext::default();
    let silent = mod_api::SoundPlay::ui("minecraft:ui.button.click").with_volume(0.0);
    let sound = audio.play_mod_sound(&silent, 0);
    assert_eq!(audio.mod_sound_state(sound), Playing, "started silently");
    audio.fade_mod_sound(sound, 1.0, 2);
    audio.tick(false, &ctx);
    assert_eq!(volume_of(&audio, sound), Some(0.5));
    audio.tick(false, &ctx);
    assert_eq!(volume_of(&audio, sound), Some(1.0));
    assert_eq!(audio.mod_sound_state(sound), Playing);
}

fn volume_of(audio: &Audio, id: InstanceId) -> Option<f32> {
    audio.engine.instance(id).map(|i| i.volume)
}

#[test]
fn idle_mixer_fast_path_zeroes_output_and_decays_limiter() {
    let mut mixer = Mixer {
        limiter_peak: 2.0,
        ..Mixer::default()
    };
    let mut out = vec![0.5f32; 480 * 2];
    mix(&mut mixer, &mut out, 2, 48_000);
    assert!(out.iter().all(|&s| s == 0.0));
    assert!(mixer.limiter_peak < 2.0 && mixer.limiter_peak > 1.0);
    // Once settled at 1.0, stays at 1.0.
    mixer.limiter_peak = 1.0;
    mix(&mut mixer, &mut out, 2, 48_000);
    assert_eq!(mixer.limiter_peak, 1.0);
}

#[test]
fn unpitched_stereo_fast_path_and_inaudible_loop_advance_match_reference() {
    // Six stereo frames with distinct left and right samples.
    // Direct stereo playback at gain 0.5 halves each sample with no panning.
    let stereo_src: Vec<f32> = vec![
        0.25, -0.5, // frame 0 -> [ 0.125, -0.25  ]
        0.5, -0.25, // frame 1 -> [ 0.25,  -0.125 ]
        0.75, 0.125, // frame 2 -> [ 0.375,  0.0625]
        -0.25, 0.75, // frame 3 -> [-0.125,  0.375 ]
        -0.5, 0.25, // frame 4 -> [-0.25,   0.125 ]
        0.125, -0.75, // frame 5 (last frame: stops once i + 1 >= frames)
    ];
    let mut mixer = Mixer::default();
    let mut ch = stereo_channel(stereo_src, None, [0.0; 3], false);
    ch.gain = 0.5;
    ch.relative = true;
    mixer.channels.insert(1, ch);
    // Inaudible looping channel (gain 0) advancing across buffers without wrapping.
    let mut inaudible = channel(vec![0.25; 100], None, [0.0; 3], true);
    inaudible.position = 5.0;
    inaudible.gain = 0.0;
    inaudible.relative = true;
    mixer.channels.insert(2, inaudible);

    // Buffer 1 (2 frames, 0 + 2 < 6): takes the stereo fast path.
    let mut out = [0.0f32; 4];
    mix(&mut mixer, &mut out, 2, 48_000);
    assert_eq!(out, [0.125, -0.25, 0.25, -0.125]);
    assert_eq!(mixer.channels[&1].position, 2.0);
    assert_eq!(mixer.channels[&1].state, State::Playing);
    assert_eq!(mixer.channels[&2].position, 7.0);
    assert_eq!(mixer.channels[&2].state, State::Playing);

    // Buffer 2 (2 frames, 2 + 2 < 6): next buffer continues from frame 2 on the fast path.
    mix(&mut mixer, &mut out, 2, 48_000);
    assert_eq!(out, [0.375, 0.0625, -0.125, 0.375]);
    assert_eq!(mixer.channels[&1].position, 4.0);
    assert_eq!(mixer.channels[&1].state, State::Playing);
    assert_eq!(mixer.channels[&2].position, 9.0);

    // Buffer 3 (2 frames, 4 + 2 == 6): crosses the fast-path boundary, plays frame 4 and stops.
    mix(&mut mixer, &mut out, 2, 48_000);
    assert_eq!(out, [-0.25, 0.125, 0.0, 0.0]);
    assert_eq!(mixer.channels[&1].state, State::Stopped);
    assert_eq!(mixer.channels[&2].position, 11.0);
    assert_eq!(mixer.channels[&2].state, State::Playing);

    // Looping stereo clip crossing the fast-path boundary and continuing on the next buffer.
    let loop_src: Vec<f32> = vec![
        0.25, -0.5, // frame 0 -> [ 0.125, -0.25  ]
        0.5, -0.25, // frame 1 -> [ 0.25,  -0.125 ]
        0.75, 0.125, // frame 2 -> [ 0.375,  0.0625]
        -0.25, 0.75, // frame 3 (wrap boundary: i + 1 >= 4 wraps to frame 0)
    ];
    let mut loop_mixer = Mixer::default();
    let mut looping = stereo_channel(loop_src, None, [0.0; 3], true);
    looping.gain = 0.5;
    looping.relative = true;
    loop_mixer.channels.insert(1, looping);

    // Buffer 1 (0 + 2 < 4): fast path frames 0 and 1.
    mix(&mut loop_mixer, &mut out, 2, 48_000);
    assert_eq!(out, [0.125, -0.25, 0.25, -0.125]);
    assert_eq!(loop_mixer.channels[&1].position, 2.0);

    // Buffer 2 (2 + 2 == 4): crosses fast-path boundary, plays frame 2 then wraps to frame 0.
    mix(&mut loop_mixer, &mut out, 2, 48_000);
    assert_eq!(out, [0.375, 0.0625, 0.125, -0.25]);
    assert_eq!(loop_mixer.channels[&1].position, 1.0);
    assert_eq!(loop_mixer.channels[&1].state, State::Playing);

    // Buffer 3 (1 + 2 < 4): back on fast path, continues cleanly from frames 1 and 2.
    mix(&mut loop_mixer, &mut out, 2, 48_000);
    assert_eq!(out, [0.25, -0.125, 0.375, 0.0625]);
    assert_eq!(loop_mixer.channels[&1].position, 3.0);
}

#[test]
fn late_device_loss_during_stream_recovery_is_caught_after_retiring_stream() {
    use std::time::Instant;
    let mut audio = live_audio();
    let now = Instant::now();
    {
        let mut m = audio.backend.mixer.lock().unwrap();
        for _ in 0..StreamErrors::DEAD_AFTER {
            m.stream_error(false, now);
        }
        assert!(m.take_stopped_stream());
        // Callback reports device loss after the initial status read and before retiring.
        m.stream_error(true, now);
    }
    assert!(audio.retire_stream_and_take_lost_device());
    assert_eq!(audio.backend.mixer.lock().unwrap().lost, None);
    // Trailing stream-only error without device loss preserves active sounds and clears `lost`.
    {
        let mut m = audio.backend.mixer.lock().unwrap();
        m.stream_errors = StreamErrors::default();
        for _ in 0..StreamErrors::DEAD_AFTER {
            m.stream_error(false, now);
        }
        assert_eq!(m.lost, Some(Lost::Stream));
    }
    assert!(!audio.retire_stream_and_take_lost_device());
    assert_eq!(audio.backend.mixer.lock().unwrap().lost, None);
}

#[test]
fn muffle_bus_is_released_on_a_later_tick_once_the_glide_finishes() {
    let mut audio = live_audio();
    let ctx = TickContext::default();
    audio
        .backend
        .mixer
        .lock()
        .unwrap()
        .channels
        .insert(1, channel(sine(440.0), None, [0.0, 0.0, 2.0], true));
    assert!(audio.muffle(0.8, 1.0, 1, &[]));
    // Audio callback allocates the muffle bus and glides `current` above `1e-4`.
    {
        let mut m = audio.backend.mixer.lock().unwrap();
        listen(&mut m, 0.1);
        assert!(m.muffle.active());
        assert!(m.muffle.bus.capacity() > 0);
    }
    // Final fade tick sets target to zero while `current > 1e-4`: bus stays allocated.
    audio.tick(false, &ctx);
    assert!(audio.muffle.is_none());
    assert!(audio.muffle_release_pending);
    {
        let m = audio.backend.mixer.lock().unwrap();
        assert_eq!(m.muffle.target, [0.0; 2]);
        assert!(m.muffle.active());
        assert!(m.muffle.bus.capacity() > 0);
    }
    // A tick before the glide finishes still keeps the bus.
    audio.tick(false, &ctx);
    assert!(audio.muffle_release_pending);
    assert!(audio.backend.mixer.lock().unwrap().muffle.bus.capacity() > 0);
    // Audio callback finishes the glide without deallocating the bus itself.
    {
        let mut m = audio.backend.mixer.lock().unwrap();
        listen(&mut m, 0.5);
        assert!(!m.muffle.active());
        assert!(m.muffle.bus.capacity() > 0);
    }
    // A later game tick reclaims the bus now that the mixer is inactive.
    audio.tick(false, &ctx);
    assert!(!audio.muffle_release_pending);
    {
        let m = audio.backend.mixer.lock().unwrap();
        assert!(m.muffle.bus.is_empty());
        assert_eq!(m.muffle.bus.capacity(), 0);
    }
}
