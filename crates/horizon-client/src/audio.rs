//! Sound playback: vanilla's `SoundEngine` bookkeeping (`sound_engine`) over a
//! small mixer standing in for OpenAL: one channel per source with its gain,
//! pitch, linear distance attenuation (`AL_LINEAR_DISTANCE`, reference 0,
//! rolloff 1), looping, relative flag and source state, Ogg/Vorbis decoding
//! (streams on a worker thread) and stereo panning.
use crate::Result;
use crate::sound_engine::{
    Attenuation, Backend, Bound, ChannelId, InstanceId, PlayResult, SoundEngine, SoundInstance,
    Sounds, TickContext,
};
use crate::sound_events::{Sound, SoundRegistry};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use horizon_core::random::{LegacyRandom, RandomSource};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// An OpenAL source's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Initial,
    Playing,
    Paused,
    Stopped,
}
/// An OpenAL source.
struct Channel {
    samples: Option<(Arc<Vec<f32>>, u32)>,
    position: f64,
    state: State,
    gain: f32,
    pitch: f32,
    /// `AL_LINEAR_DISTANCE` max distance, or no attenuation.
    linear: Option<f32>,
    looping: bool,
    at: [f32; 3],
    relative: bool,
}
#[derive(Default)]
struct Mixer {
    channels: HashMap<ChannelId, Channel>,
    listener: [f64; 3],
    listener_yaw: f32,
}
/// A sound the player could have heard (`SoundEventListener.onPlaySound`):
/// what the subtitle overlay needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Heard {
    /// The `subtitle` translation key of the sound event.
    pub subtitle: String,
    /// The instance's position (relative sounds report their raw coordinates).
    pub at: Option<[f64; 3]>,
    /// `SoundEngine.getRange`: infinite for relative and non-attenuated sounds.
    pub range: f32,
    pub looping: bool,
}
/// The mixer side of the engine (`Backend`).
pub struct MixerBackend {
    mixer: Arc<Mutex<Mixer>>,
    root: PathBuf,
    cache: Arc<Mutex<SoundCache>>,
    /// The decoder thread for static sounds, started with the first one.
    decoder: Option<std::sync::mpsc::Sender<Decode>>,
    next: ChannelId,
    live: bool,
}
/// Decoded samples and their rate (`None`: the file did not load).
type Samples = Option<(Arc<Vec<f32>>, u32)>;
/// Decoded static sounds (`SoundBufferLibrary`'s buffers), dropped least
/// recently used first past `CACHE_BYTES`: kept forever, every sound heard in
/// a session stayed (up to 1.4 GiB of samples, villages playing hundreds).
#[derive(Default)]
struct SoundCache {
    entries: HashMap<String, (Samples, u64)>,
    bytes: usize,
    clock: u64,
}
const CACHE_BYTES: usize = 64 << 20;
impl SoundCache {
    fn get(&mut self, key: &str) -> Option<Samples> {
        self.clock += 1;
        let entry = self.entries.get_mut(key)?;
        entry.1 = self.clock;
        Some(entry.0.clone())
    }
    fn insert(&mut self, key: String, samples: Samples) {
        self.clock += 1;
        self.bytes += Self::size(&samples);
        if let Some((old, _)) = self.entries.insert(key, (samples, self.clock)) {
            self.bytes -= Self::size(&old);
        }
        while self.bytes > CACHE_BYTES && self.entries.len() > 1 {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some((gone, _)) = self.entries.remove(&oldest) {
                self.bytes -= Self::size(&gone);
            }
        }
    }
    fn size(samples: &Samples) -> usize {
        samples
            .as_ref()
            .map_or(0, |(s, _)| s.len() * std::mem::size_of::<f32>())
    }
}
/// A static sound to decode for a channel waiting on it.
struct Decode {
    key: String,
    file: PathBuf,
    channel: ChannelId,
}
/// Give a channel waiting on its sound the decoded samples and start it, as
/// the buffer future attaching and playing the source. A source stopped or
/// released meanwhile stays so.
fn attach(mixer: &Mutex<Mixer>, channel: ChannelId, samples: Samples, looping: bool) {
    if let Some(samples) = samples
        && let Ok(mut m) = mixer.lock()
        && let Some(c) = m.channels.get_mut(&channel)
        && c.state == State::Initial
    {
        c.samples = Some(samples);
        c.position = 0.0;
        c.state = State::Playing;
        c.looping |= looping;
    }
}
pub struct Audio {
    _stream: Option<cpal::Stream>,
    pub engine: SoundEngine,
    backend: MixerBackend,
    /// Record `Heard` sounds (the subtitle overlay is listening).
    pub record_heard: bool,
    pub listener: [f64; 3],
    pub listener_yaw: f32,
    pub master: f32,
    /// Seeds of positional plays (the client level random's `nextLong`).
    random: LegacyRandom,
    /// A `SoundSource` index the next sound plays on (`/playsound`).
    source_override: Option<usize>,
    /// Positional sounds played since the last drain, kept while the world
    /// is open to the network so guests hear them too.
    pub capture: Option<Vec<(String, [f64; 3], f32, f32)>>,
}
fn decode(path: &Path) -> Option<(Vec<f32>, u32)> {
    use symphonia::core::{
        audio::SampleBuffer, codecs::DecoderOptions, formats::FormatOptions, io::MediaSourceStream,
        meta::MetadataOptions, probe::Hint,
    };
    let file = fs::File::open(path).ok()?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("ogg");
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?;
    let mut format = probed.format;
    let track = format.default_track()?;
    let id = track.id;
    let rate = track.codec_params.sample_rate?;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .ok()?;
    let mut mono = Vec::new();
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != id {
            continue;
        }
        let Ok(buffer) = decoder.decode(&packet) else {
            continue;
        };
        let spec = *buffer.spec();
        let channels = spec.channels.count().max(1);
        let mut samples = SampleBuffer::<f32>::new(buffer.capacity() as u64, spec);
        samples.copy_interleaved_ref(buffer);
        for frame in samples.samples().chunks(channels) {
            mono.push(frame.iter().sum::<f32>() / channels as f32);
        }
    }
    Some((mono, rate))
}
impl MixerBackend {
    fn with(&self, channel: ChannelId, f: impl FnOnce(&mut Channel)) {
        if let Ok(mut m) = self.mixer.lock()
            && let Some(c) = m.channels.get_mut(&channel)
        {
            f(c);
        }
    }
}
impl Backend for MixerBackend {
    fn create(&mut self, _streaming: bool) -> Option<ChannelId> {
        self.next += 1;
        let id = self.next;
        self.mixer.lock().ok()?.channels.insert(
            id,
            Channel {
                samples: None,
                position: 0.0,
                state: State::Initial,
                gain: 1.0,
                pitch: 1.0,
                linear: None,
                looping: false,
                at: [0.0; 3],
                relative: false,
            },
        );
        Some(id)
    }
    fn destroy(&mut self, channel: ChannelId) {
        if let Ok(mut m) = self.mixer.lock() {
            m.channels.remove(&channel);
        }
    }
    fn set_pitch(&mut self, channel: ChannelId, pitch: f32) {
        self.with(channel, |c| c.pitch = pitch);
    }
    fn set_volume(&mut self, channel: ChannelId, volume: f32) {
        self.with(channel, |c| c.gain = volume);
    }
    fn set_attenuation(&mut self, channel: ChannelId, linear: Option<f32>) {
        self.with(channel, |c| c.linear = linear);
    }
    fn set_looping(&mut self, channel: ChannelId, looping: bool) {
        self.with(channel, |c| c.looping = looping);
    }
    fn set_position(&mut self, channel: ChannelId, p: [f64; 3]) {
        self.with(channel, |c| c.at = [p[0] as f32, p[1] as f32, p[2] as f32]);
    }
    fn set_relative(&mut self, channel: ChannelId, relative: bool) {
        self.with(channel, |c| c.relative = relative);
    }
    /// `getCompleteBuffer` (cached, else decoded on the decoder thread) or
    /// `getStream` (decoded on a thread of its own), then attach and
    /// `alSourcePlay`. A sound that cannot load leaves its source initial,
    /// as a failed buffer future does in vanilla.
    fn start(&mut self, channel: ChannelId, sound: &Sound, looping: bool) {
        if !self.live {
            return;
        }
        let (namespace, path) = sound.path();
        let file = self.root.join("assets").join(namespace).join(&path);
        if sound.stream {
            // A streamed sound loops by restarting its stream.
            let mixer = self.mixer.clone();
            std::thread::spawn(move || {
                let samples = decode(&file).map(|(s, r)| (Arc::new(s), r));
                attach(&mixer, channel, samples, looping);
            });
            return;
        }
        let key = format!("{namespace}:{path}");
        let cached = self.cache.lock().ok().and_then(|mut c| c.get(&key));
        if let Some(samples) = cached {
            attach(&self.mixer, channel, samples, false);
            return;
        }
        let decoder = self.decoder.get_or_insert_with(|| {
            let (send, receive) = std::sync::mpsc::channel::<Decode>();
            let (mixer, cache) = (self.mixer.clone(), self.cache.clone());
            std::thread::Builder::new()
                .name("sound decoder".into())
                .spawn(move || {
                    for job in receive {
                        // Queued twice before the first decode finished.
                        let cached = cache.lock().ok().and_then(|mut c| c.get(&job.key));
                        let samples = cached.unwrap_or_else(|| {
                            let samples = decode(&job.file).map(|(s, r)| (Arc::new(s), r));
                            if let Ok(mut c) = cache.lock() {
                                c.insert(job.key, samples.clone());
                            }
                            samples
                        });
                        attach(&mixer, job.channel, samples, false);
                    }
                })
                .expect("sound decoder thread");
            send
        });
        let _ = decoder.send(Decode { key, file, channel });
    }
    /// `alSourceStop`: any state becomes stopped, an initial source too (a
    /// sound still decoding then never starts).
    fn stop(&mut self, channel: ChannelId) {
        self.with(channel, |c| c.state = State::Stopped);
    }
    fn pause(&mut self, channel: ChannelId) {
        self.with(channel, |c| {
            if c.state == State::Playing {
                c.state = State::Paused;
            }
        });
    }
    fn unpause(&mut self, channel: ChannelId) {
        self.with(channel, |c| {
            if c.state == State::Paused {
                c.state = State::Playing;
            }
        });
    }
    fn stopped(&mut self, channel: ChannelId) -> bool {
        self.mixer
            .lock()
            .map(|m| {
                m.channels
                    .get(&channel)
                    .is_none_or(|c| c.state == State::Stopped)
            })
            .unwrap_or(true)
    }
}
/// The gain OpenAL applies for distance (`AL_LINEAR_DISTANCE` with reference
/// distance 0 and rolloff 1: the distance clamps to the max distance).
fn distance_gain(linear: Option<f32>, distance: f32) -> f32 {
    match linear {
        None => 1.0,
        Some(max) if max > 0.0 => (1.0 - distance.min(max) / max).clamp(0.0, 1.0),
        Some(_) => 0.0,
    }
}
fn mix(mixer: &mut Mixer, data: &mut [f32], channels: usize, rate: u32) {
    data.fill(0.0);
    let listener = mixer.listener;
    let yaw = mixer.listener_yaw.to_radians();
    // Gains and pans per source for this buffer.
    let mut voices: Vec<(ChannelId, f32, f32, f64)> = Vec::new();
    for (id, c) in &mixer.channels {
        let Some((_, source_rate)) = &c.samples else {
            continue;
        };
        if c.state != State::Playing {
            continue;
        }
        let d = if c.relative {
            [f64::from(c.at[0]), f64::from(c.at[1]), f64::from(c.at[2])]
        } else {
            [
                f64::from(c.at[0]) - listener[0],
                f64::from(c.at[1]) - listener[1],
                f64::from(c.at[2]) - listener[2],
            ]
        };
        let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() as f32;
        let gain = c.gain.clamp(0.0, 1.0) * distance_gain(c.linear, distance);
        // The listener's right vector for a Minecraft yaw.
        let side = if c.relative {
            (d[0] as f32) / distance.max(0.001)
        } else {
            (-(yaw.cos()) as f64 * d[0] - (yaw.sin()) as f64 * d[2]) as f32 / distance.max(0.001)
        };
        let pan = side.clamp(-1.0, 1.0);
        let left = gain * (1.0 - pan.max(0.0) * 0.6);
        let right = gain * (1.0 + pan.min(0.0) * 0.6);
        let step = f64::from(*source_rate) / f64::from(rate) * f64::from(c.pitch.max(0.0));
        voices.push((*id, left, right, step));
    }
    for (id, left, right, step) in voices {
        let Some(c) = mixer.channels.get_mut(&id) else {
            continue;
        };
        let Some((samples, _)) = c.samples.clone() else {
            continue;
        };
        for frame in data.chunks_mut(channels) {
            let mut i = c.position as usize;
            if i + 1 >= samples.len() {
                if c.looping && samples.len() > 1 {
                    c.position = 0.0;
                    i = 0;
                } else {
                    c.state = State::Stopped;
                    break;
                }
            }
            let t = (c.position - i as f64) as f32;
            let s = samples[i] * (1.0 - t) + samples[i + 1] * t;
            if channels >= 2 {
                frame[0] += s * left;
                frame[1] += s * right;
            } else {
                frame[0] += s * (left + right) / 2.0;
            }
            c.position += step;
        }
    }
    for s in data.iter_mut() {
        *s = s.clamp(-1.0, 1.0);
    }
}
/// Why the game should not open an output, if it should not:
/// `HORIZON_NO_AUDIO`, or a virtual screen (`vscreen`), where agents and
/// harnesses run the game and must stay off the user's speakers.
/// `HORIZON_AUDIO=1` opens the output anyway (testing into a null sink).
fn output_disabled(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<String> {
    if var("HORIZON_NO_AUDIO").is_some() {
        return Some("audio off: HORIZON_NO_AUDIO is set".into());
    }
    if var("HORIZON_AUDIO").is_some_and(|v| v == "1") {
        return None;
    }
    let display = var("WAYLAND_DISPLAY")?;
    Path::new(&display)
        .components()
        .any(|c| c.as_os_str() == "vscreen")
        .then(|| "audio off on a virtual screen (vscreen); HORIZON_AUDIO=1 opens it".into())
}
/// An output stream on `device` in its default configuration, whatever its
/// sample format: the mixer works in f32 and converts.
fn open_device(
    device: &cpal::Device,
    mixer: Arc<Mutex<Mixer>>,
) -> Result<(cpal::Stream, cpal::SupportedStreamConfig)> {
    use cpal::SampleFormat as F;
    let config = device.default_output_config()?;
    let stream = match config.sample_format() {
        F::F32 => build_stream::<f32>(device, &config, mixer),
        F::F64 => build_stream::<f64>(device, &config, mixer),
        F::I16 => build_stream::<i16>(device, &config, mixer),
        F::I32 => build_stream::<i32>(device, &config, mixer),
        F::I8 => build_stream::<i8>(device, &config, mixer),
        F::U16 => build_stream::<u16>(device, &config, mixer),
        F::U32 => build_stream::<u32>(device, &config, mixer),
        F::U8 => build_stream::<u8>(device, &config, mixer),
        other => Err(format!("unsupported sample format {other}").into()),
    }?;
    stream.play()?;
    Ok((stream, config))
}
fn build_stream<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    mixer: Arc<Mutex<Mixer>>,
) -> Result<cpal::Stream> {
    let rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    let mut scratch = Vec::new();
    let stream = device.build_output_stream(
        &config.config(),
        move |data: &mut [T], _| {
            scratch.resize(data.len(), 0.0f32);
            if let Ok(mut m) = mixer.lock() {
                mix(&mut m, &mut scratch, channels, rate);
            } else {
                scratch.fill(0.0);
            }
            for (out, s) in data.iter_mut().zip(&scratch) {
                *out = T::from_sample(*s);
            }
        },
        |e| log::warn!("Horizon: audio stream error: {e}"),
        None,
    )?;
    Ok(stream)
}
impl Audio {
    pub fn new(root: &Path) -> Self {
        Self::with_output(root, true)
    }
    /// No output device: the dedicated server.
    pub fn silent(root: &Path) -> Self {
        Self::with_output(root, false)
    }
    fn with_output(root: &Path, output: bool) -> Self {
        let registry = SoundRegistry::load(root);
        let mixer = Arc::new(Mutex::new(Mixer::default()));
        let opened = if output {
            Self::open(mixer.clone())
        } else {
            Err("no output on a dedicated server".into())
        };
        let (stream, live) = match opened {
            Ok(v) => (Some(v), true),
            Err(e) => {
                if output {
                    log::warn!("Horizon: no sound: {e}");
                }
                (None, false)
            }
        };
        let mut engine = SoundEngine::new(registry);
        engine.loaded = live;
        Self {
            _stream: stream,
            engine,
            backend: MixerBackend {
                mixer,
                root: root.to_owned(),
                cache: Arc::default(),
                decoder: None,
                next: 0,
                live,
            },
            record_heard: false,
            listener: [0.0; 3],
            listener_yaw: 0.0,
            master: 1.0,
            random: LegacyRandom::new(crate::sound_engine::unique_seed()),
            source_override: None,
            capture: None,
        }
    }
    fn open(mixer: Arc<Mutex<Mixer>>) -> Result<cpal::Stream> {
        if let Some(reason) = output_disabled(|name| std::env::var_os(name)) {
            return Err(reason.into());
        }
        let host = cpal::default_host();
        let mut errors = Vec::new();
        // The default device first, then any other output a host lists:
        // a default that cannot open (busy, or a stale ALSA default) should
        // not leave the game silent when another device works.
        let default = host.default_output_device();
        let others = host.output_devices().into_iter().flatten();
        let mut tried = Vec::new();
        for device in default.into_iter().chain(others) {
            let name = device.name().unwrap_or_else(|_| "unnamed device".into());
            if tried.contains(&name) {
                continue;
            }
            tried.push(name.clone());
            match open_device(&device, mixer.clone()) {
                Ok((stream, config)) => {
                    log::info!(
                        "Horizon: audio output {name} ({} Hz, {} channels, {})",
                        config.sample_rate().0,
                        config.channels(),
                        config.sample_format()
                    );
                    return Ok(stream);
                }
                Err(e) => errors.push(format!("{name}: {e}")),
            }
        }
        if errors.is_empty() {
            return Err(format!("no audio output device on the {} host", host.id().name()).into());
        }
        Err(format!("no audio output device opened ({})", errors.join("; ")).into())
    }
    fn bound(&mut self) -> Bound<'_, MixerBackend> {
        Bound {
            engine: &mut self.engine,
            backend: &mut self.backend,
        }
    }
    /// `SoundManager.updateSource`: the camera position and yaw.
    pub fn set_listener(&mut self, position: [f64; 3], yaw: f32) {
        self.listener = position;
        self.listener_yaw = yaw;
        if let Ok(mut m) = self.backend.mixer.lock() {
            m.listener = position;
            m.listener_yaw = yaw;
        }
    }
    /// Sync the volume options, refreshing the categories that changed
    /// (the option sliders' `refreshCategoryVolume`).
    fn sync_volumes(&mut self) {
        let mut volumes = [self.master; 11];
        for (i, v) in volumes.iter_mut().enumerate().skip(1) {
            *v = crate::settings::source_volume(i);
        }
        for source in 0..11 {
            if self.engine.source_volumes[source] != volumes[source] {
                self.engine.source_volumes[source] = volumes[source];
                self.engine
                    .refresh_category_volume(source, &mut self.backend);
            }
        }
        self.engine.listening = self.record_heard;
    }
    /// `SoundManager.tick(paused)`, once per client tick.
    pub fn tick(&mut self, paused: bool, ctx: &TickContext) {
        self.sync_volumes();
        self.engine.tick(paused, ctx, &mut self.backend);
    }
    /// `SoundManager.pauseAllExcept(MUSIC, UI)`.
    pub fn pause(&mut self) {
        self.engine.pause_all_except(
            &[crate::sound_engine::MUSIC, crate::sound_engine::UI],
            &mut self.backend,
        );
    }
    /// `SoundManager.resume`.
    pub fn resume(&mut self) {
        self.engine.resume(&mut self.backend);
    }
    /// `SoundManager.stop()`: every sound.
    pub fn stop_all(&mut self) {
        self.engine.stop_all(&mut self.backend);
    }
    /// `SoundManager.reload` (resource packs changed): stop every sound, then
    /// read sound events and sound files from the new resource root.
    pub fn reload(&mut self, root: &Path) {
        self.stop_all();
        self.engine.registry = SoundRegistry::load(root);
        self.backend.root = root.to_owned();
        let mut cache = self.backend.cache.lock().unwrap();
        cache.entries.clear();
        cache.bytes = 0;
    }
    /// Play a sound event at a world position, as `ClientLevel.playLocalSound`
    /// (a `SimpleSoundInstance` seeded from the level random), or relative to
    /// the listener without attenuation (`None`).
    pub fn play(&mut self, event: &str, at: Option<[f64; 3]>, volume: f32, pitch: f32) {
        let seed = self.random.next_long();
        self.play_seeded(event, at, volume, pitch, seed);
    }
    /// `play` with the seed a sound packet carries.
    pub fn play_seeded(
        &mut self,
        event: &str,
        at: Option<[f64; 3]>,
        volume: f32,
        pitch: f32,
        seed: i64,
    ) {
        if let (Some(capture), Some(at)) = (self.capture.as_mut(), at) {
            capture.push((event.to_owned(), at, volume, pitch));
        }
        self.sync_volumes();
        let event = crate::sound_events::normalize(event);
        let source = self
            .source_override
            .unwrap_or_else(|| crate::settings::sound_source(&event));
        let random = LegacyRandom::new(seed);
        let instance = match at {
            Some(at) => SoundInstance::at(&event, source, volume, pitch, random, at),
            None => SoundInstance::new(
                &event,
                source,
                volume,
                pitch,
                random,
                false,
                0,
                Attenuation::None,
                [0.0; 3],
                true,
            ),
        };
        self.play_instance(instance, false);
    }
    /// Play a sound on a given `SoundSource` index (`/playsound`).
    pub fn play_on(
        &mut self,
        source: usize,
        event: &str,
        at: Option<[f64; 3]>,
        volume: f32,
        pitch: f32,
    ) {
        self.source_override = Some(source);
        self.play(event, at, volume, pitch);
        self.source_override = None;
    }
    /// `SimpleSoundInstance.forJukeboxSong` at a jukebox; the handle stays
    /// until `release`.
    pub fn play_jukebox_song(&mut self, event: &str, pos: [i32; 3]) -> InstanceId {
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        let event = crate::sound_events::normalize(event);
        let instance = SoundInstance::for_jukebox_song(&event, at, SoundEngine::unseeded());
        self.play_instance(instance, true).0
    }
    /// Stop and drop a held instance.
    pub fn stop_and_release(&mut self, id: InstanceId) {
        self.stop_instance(id);
        self.release(id);
    }
    /// Every sound event id (for command completion).
    /// The engine's table sizes and the mixer's channels (`status`).
    pub fn census(&self) -> serde_json::Value {
        let mut census = self.engine.census();
        census["channels"] = self
            .backend
            .mixer
            .lock()
            .map_or(0, |m| m.channels.len())
            .into();
        if let Ok(cache) = self.backend.cache.lock() {
            census["decoded"] = cache.entries.len().into();
            census["decoded_mib"] = (cache.bytes >> 20).into();
        }
        census
    }
    pub fn event_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.engine.registry.names().cloned().collect();
        v.sort();
        v
    }
    /// `ClientboundStopSoundPacket`: stop the playing sounds of an event
    /// and/or a source.
    pub fn stop_sounds(&mut self, event: Option<&str>, source: Option<usize>) {
        let event = event.map(crate::sound_events::normalize);
        self.engine
            .stop_matching(event.as_deref(), source, &mut self.backend);
    }
    /// The sounds played since the last call, for the subtitle overlay.
    pub fn take_heard(&mut self) -> Vec<Heard> {
        std::mem::take(&mut self.engine.notifications)
            .into_iter()
            .filter_map(|n| {
                Some(Heard {
                    subtitle: n.subtitle?,
                    at: Some(n.position),
                    range: n.range,
                    looping: n.looping,
                })
            })
            .collect()
    }
}
impl Sounds for Audio {
    fn engine(&mut self) -> &mut SoundEngine {
        &mut self.engine
    }
    fn play_instance(&mut self, instance: SoundInstance, retain: bool) -> (InstanceId, PlayResult) {
        self.sync_volumes();
        self.bound().play_instance(instance, retain)
    }
    fn play_id(&mut self, id: InstanceId) -> PlayResult {
        self.sync_volumes();
        self.bound().play_id(id)
    }
    fn stop_instance(&mut self, id: InstanceId) {
        self.bound().stop_instance(id);
    }
    fn update_category_volume(&mut self, source: usize, gain: f32) {
        self.bound().update_category_volume(source, gain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(samples: Vec<f32>, linear: Option<f32>, at: [f32; 3], looping: bool) -> Channel {
        Channel {
            samples: Some((Arc::new(samples), 48_000)),
            position: 0.0,
            state: State::Playing,
            gain: 1.0,
            pitch: 1.0,
            linear,
            looping,
            at,
            relative: false,
        }
    }

    #[test]
    fn decoded_sounds_stay_within_the_budget_least_recently_used_out() {
        let mut cache = SoundCache::default();
        let quarter = CACHE_BYTES / 4 / std::mem::size_of::<f32>();
        let sound = || Some((Arc::new(vec![0.0f32; quarter]), 44100));
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
        assert!(
            output_disabled(env(&[("HORIZON_NO_AUDIO", "1"), ("HORIZON_AUDIO", "1")])).is_some()
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
}
