//! Sound playback: `sounds.json` events with weighted variants, Ogg/Vorbis
//! decoding, and a small mixer with vanilla's linear distance attenuation
//! (`SoundInstance.Attenuation.LINEAR`, default range 16 blocks) and stereo panning.
use crate::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug)]
struct Variant {
    name: String,
    volume: f32,
    pitch: f32,
    weight: u32,
    attenuation: f32,
    is_event: bool,
}
struct Voice {
    samples: Arc<Vec<f32>>,
    position: f64,
    step: f64,
    left: f32,
    right: f32,
}
#[derive(Default)]
struct Mixer {
    voices: Vec<Voice>,
    /// The streaming music track (`SoundSource.MUSIC`), kept apart from the
    /// voice cap and scaled by `music_gain` (MusicManager's fade).
    music: Option<Voice>,
    music_gain: f32,
    /// Tickable sound instances the game keeps a handle to (ambient loops).
    tracked: Vec<Tracked>,
}
struct Tracked {
    id: u64,
    voice: Voice,
    looping: bool,
    volume: f32,
}
/// The current music track: decoding on a worker until the voice exists.
struct MusicTrack {
    event: String,
    /// Set by the decoder thread once the voice is in the mixer (or failed).
    ready: Arc<std::sync::atomic::AtomicBool>,
}
pub struct Audio {
    _stream: Option<cpal::Stream>,
    mixer: Arc<Mutex<Mixer>>,
    rate: u32,
    root: PathBuf,
    events: HashMap<String, Vec<Variant>>,
    cache: HashMap<String, Option<(Arc<Vec<f32>>, u32)>>,
    seed: u64,
    pub listener: [f64; 3],
    pub listener_yaw: f32,
    pub master: f32,
    music: Option<MusicTrack>,
    next_tracked: u64,
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
        .format(&hint, stream, &FormatOptions::default(), &MetadataOptions::default())
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
        let Ok(buffer) = decoder.decode(&packet) else { continue };
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
impl Audio {
    pub fn new(root: &Path) -> Self {
        let mut events = HashMap::new();
        if let Ok(text) = fs::read(root.join("assets/minecraft/sounds.json"))
            && let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(&text)
        {
            for (name, def) in map {
                let variants = def["sounds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|s| match s {
                        Value::String(n) => Variant {
                            name: n.clone(),
                            volume: 1.0,
                            pitch: 1.0,
                            weight: 1,
                            attenuation: 16.0,
                            is_event: false,
                        },
                        _ => Variant {
                            name: s["name"].as_str().unwrap_or("").to_owned(),
                            volume: s["volume"].as_f64().unwrap_or(1.0) as f32,
                            pitch: s["pitch"].as_f64().unwrap_or(1.0) as f32,
                            weight: s["weight"].as_u64().unwrap_or(1) as u32,
                            attenuation: s["attenuation_distance"].as_f64().unwrap_or(16.0) as f32,
                            is_event: s["type"].as_str() == Some("event"),
                        },
                    })
                    .collect::<Vec<_>>();
                events.insert(format!("minecraft:{name}"), variants);
            }
        }
        let mixer = Arc::new(Mutex::new(Mixer { music_gain: 1.0, ..Mixer::default() }));
        let (stream, rate) = match Self::open(mixer.clone()) {
            Ok(v) => (Some(v.0), v.1),
            Err(e) => {
                eprintln!("Horizon: audio unavailable: {e}");
                (None, 48_000)
            }
        };
        Self {
            _stream: stream,
            mixer,
            rate,
            root: root.to_owned(),
            events,
            cache: HashMap::new(),
            seed: 0x5DEE_CE66,
            listener: [0.0; 3],
            listener_yaw: 0.0,
            master: 1.0,
            music: None,
            next_tracked: 0,
        }
    }
    fn open(mixer: Arc<Mutex<Mixer>>) -> Result<(cpal::Stream, u32)> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or("no audio output device")?;
        let config = device.default_output_config()?;
        let rate = config.sample_rate().0;
        let channels = config.channels() as usize;
        let stream = device.build_output_stream(
            &config.into(),
            move |data: &mut [f32], _| {
                data.fill(0.0);
                let Ok(mut mixer) = mixer.lock() else { return };
                for frame in data.chunks_mut(channels) {
                    let (mut l, mut r) = (0.0f32, 0.0f32);
                    for v in mixer.voices.iter_mut() {
                        let i = v.position as usize;
                        if i + 1 >= v.samples.len() {
                            continue;
                        }
                        let t = (v.position - i as f64) as f32;
                        let s = v.samples[i] * (1.0 - t) + v.samples[i + 1] * t;
                        l += s * v.left;
                        r += s * v.right;
                        v.position += v.step;
                    }
                    for t in mixer.tracked.iter_mut() {
                        let v = &mut t.voice;
                        let mut i = v.position as usize;
                        if t.looping && i + 1 >= v.samples.len() {
                            v.position = 0.0;
                            i = 0;
                        }
                        if i + 1 < v.samples.len() {
                            let f = (v.position - i as f64) as f32;
                            let s = (v.samples[i] * (1.0 - f) + v.samples[i + 1] * f) * t.volume;
                            l += s * v.left;
                            r += s * v.right;
                            v.position += v.step;
                        }
                    }
                    let gain = mixer.music_gain;
                    if let Some(v) = mixer.music.as_mut() {
                        let i = v.position as usize;
                        if i + 1 < v.samples.len() {
                            let t = (v.position - i as f64) as f32;
                            let s = (v.samples[i] * (1.0 - t) + v.samples[i + 1] * t) * gain;
                            l += s * v.left;
                            r += s * v.right;
                            v.position += v.step;
                        }
                    }
                    if channels >= 2 {
                        frame[0] = l.clamp(-1.0, 1.0);
                        frame[1] = r.clamp(-1.0, 1.0);
                    } else {
                        frame[0] = ((l + r) / 2.0).clamp(-1.0, 1.0);
                    }
                }
                mixer.voices.retain(|v| (v.position as usize) + 1 < v.samples.len());
                mixer.tracked.retain(|t| t.looping || (t.voice.position as usize) + 1 < t.voice.samples.len());
                if mixer.music.as_ref().is_some_and(|v| (v.position as usize) + 1 >= v.samples.len()) {
                    mixer.music = None;
                }
            },
            |e| eprintln!("Horizon: audio stream error: {e}"),
            None,
        )?;
        stream.play()?;
        Ok((stream, rate))
    }
    fn random(&mut self) -> u64 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        self.seed
    }
    fn resolve(&mut self, event: &str, depth: u32) -> Option<Variant> {
        let variants = self.events.get(event)?.clone();
        let total: u32 = variants.iter().map(|v| v.weight).sum();
        if total == 0 {
            return None;
        }
        let mut pick = (self.random() % u64::from(total)) as u32;
        let chosen = variants.into_iter().find(|v| {
            if pick < v.weight {
                true
            } else {
                pick -= v.weight;
                false
            }
        })?;
        if chosen.is_event && depth < 4 {
            let name = if chosen.name.contains(':') { chosen.name.clone() } else { format!("minecraft:{}", chosen.name) };
            let mut inner = self.resolve(&name, depth + 1)?;
            inner.volume *= chosen.volume;
            inner.pitch *= chosen.pitch;
            return Some(inner);
        }
        Some(chosen)
    }
    /// Play a sound event at a world position (None = non-positional UI sound).
    pub fn play(&mut self, event: &str, at: Option<[f64; 3]>, volume: f32, pitch: f32) {
        if self._stream.is_none() {
            return;
        }
        let Some(variant) = self.resolve(event, 0) else { return };
        let file = variant.name.trim_start_matches("minecraft:").to_owned();
        let entry = self.cache.entry(file.clone()).or_insert_with(|| {
            decode(&self.root.join(format!("assets/minecraft/sounds/{file}.ogg"))).map(|(s, r)| (Arc::new(s), r))
        });
        let Some((samples, rate)) = entry.clone() else { return };
        let volume = (volume * variant.volume).clamp(0.0, 1.0) * self.master * crate::settings::event_volume(event);
        let pitch = (pitch * variant.pitch).clamp(0.5, 2.0);
        let (mut left, mut right) = (volume, volume);
        if let Some(p) = at {
            let d = [p[0] - self.listener[0], p[1] - self.listener[1], p[2] - self.listener[2]];
            let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() as f32;
            let range = variant.attenuation * volume.max(1.0);
            let gain = (1.0 - distance / range).clamp(0.0, 1.0);
            if gain <= 0.0 {
                return;
            }
            let yaw = self.listener_yaw.to_radians();
            // Right vector of a Minecraft yaw.
            let side = (-(yaw.cos()) as f64 * d[0] - (yaw.sin()) as f64 * d[2]) as f32 / distance.max(0.001);
            let pan = side.clamp(-1.0, 1.0);
            left = volume * gain * (1.0 - pan.max(0.0) * 0.6);
            right = volume * gain * (1.0 + pan.min(0.0) * 0.6);
        }
        let step = f64::from(rate) / f64::from(self.rate) * f64::from(pitch);
        if let Ok(mut m) = self.mixer.lock() {
            if m.voices.len() > 64 {
                m.voices.remove(0);
            }
            m.voices.push(Voice {
                samples,
                position: 0.0,
                step,
                left,
                right,
            });
        }
    }
    /// `SoundManager.play` of a music instance (`SimpleSoundInstance.forMusic`:
    /// non-positional, streamed). Decoding runs on a worker thread; the track
    /// counts as active from now on. Returns false when nothing can play.
    pub fn play_music(&mut self, event: &str) -> bool {
        self.stop_music();
        if self._stream.is_none() {
            return false;
        }
        let Some(variant) = self.resolve(event, 0) else { return false };
        let path = self.root.join(format!("assets/minecraft/sounds/{}.ogg", variant.name.trim_start_matches("minecraft:")));
        let volume = variant.volume.clamp(0.0, 1.0) * self.master * crate::settings::event_volume(event);
        let step_scale = f64::from(variant.pitch.clamp(0.5, 2.0)) / f64::from(self.rate);
        let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (mixer, done) = (self.mixer.clone(), ready.clone());
        std::thread::spawn(move || {
            if let Some((samples, rate)) = decode(&path)
                && let Ok(mut m) = mixer.lock()
                && !done.load(std::sync::atomic::Ordering::Acquire)
            {
                m.music = Some(Voice {
                    samples: Arc::new(samples),
                    position: 0.0,
                    step: f64::from(rate) * step_scale,
                    left: volume,
                    right: volume,
                });
            }
            done.store(true, std::sync::atomic::Ordering::Release);
        });
        self.music = Some(MusicTrack { event: event.to_owned(), ready });
        true
    }
    /// `SoundManager.stop` of the music instance.
    pub fn stop_music(&mut self) {
        if let Some(track) = self.music.take() {
            // A decoder still running sees the flag and drops its result.
            track.ready.store(true, std::sync::atomic::Ordering::Release);
        }
        if let Ok(mut m) = self.mixer.lock() {
            m.music = None;
        }
    }
    /// `SoundManager.isActive`: decoding or still sounding.
    pub fn music_active(&mut self) -> bool {
        let Some(track) = &self.music else { return false };
        if !track.ready.load(std::sync::atomic::Ordering::Acquire) {
            return true;
        }
        let playing = self.mixer.lock().is_ok_and(|m| m.music.is_some());
        if !playing {
            self.music = None;
        }
        playing
    }
    /// The event of the current music track.
    pub fn music_event(&self) -> Option<&str> {
        self.music.as_ref().map(|t| t.event.as_str())
    }
    /// `SoundManager.updateCategoryVolume(MUSIC, gain)`.
    pub fn set_music_gain(&mut self, gain: f32) {
        if let Ok(mut m) = self.mixer.lock() {
            m.music_gain = gain;
        }
    }
    /// Play a relative (non-positional) tickable sound instance the caller
    /// keeps a handle to: ambient loops (`looping`) and sub-sounds.
    pub fn play_tracked(&mut self, event: &str, volume: f32, looping: bool) -> Option<u64> {
        self._stream.as_ref()?;
        let variant = self.resolve(event, 0)?;
        let file = variant.name.trim_start_matches("minecraft:").to_owned();
        let root = &self.root;
        let (samples, rate) = self
            .cache
            .entry(file.clone())
            .or_insert_with(|| decode(&root.join(format!("assets/minecraft/sounds/{file}.ogg"))).map(|(s, r)| (Arc::new(s), r)))
            .clone()?;
        let gain = variant.volume.clamp(0.0, 1.0) * self.master * crate::settings::event_volume(event);
        let step = f64::from(rate) / f64::from(self.rate) * f64::from(variant.pitch.clamp(0.5, 2.0));
        self.next_tracked += 1;
        let id = self.next_tracked;
        let mut m = self.mixer.lock().ok()?;
        m.tracked.push(Tracked {
            id,
            voice: Voice { samples, position: 0.0, step, left: gain, right: gain },
            looping,
            volume: volume.clamp(0.0, 1.0),
        });
        Some(id)
    }
    /// A tracked instance's volume (`AbstractTickableSoundInstance.volume`).
    pub fn set_tracked_volume(&mut self, id: u64, volume: f32) {
        if let Ok(mut m) = self.mixer.lock()
            && let Some(t) = m.tracked.iter_mut().find(|t| t.id == id)
        {
            t.volume = volume.clamp(0.0, 1.0);
        }
    }
    pub fn stop_tracked(&mut self, id: u64) {
        if let Ok(mut m) = self.mixer.lock() {
            m.tracked.retain(|t| t.id != id);
        }
    }
    /// Still sounding (a finished one-shot is gone).
    pub fn tracked_active(&self, id: u64) -> bool {
        self.mixer.lock().is_ok_and(|m| m.tracked.iter().any(|t| t.id == id))
    }
}
