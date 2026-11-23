//! Decoded block sprites kept on disk between launches (M-11): reading and
//! decoding ~1300 small PNGs is most of `Sprites::load`. One file per resource
//! cache location, valid only while the resource source's textures have the
//! same identities/change stamps (or open archive contents) and format version; anything
//! else (missing, corrupt, stale, unwritable) means the normal decoding path.
use crate::{mipmap::Strategy, resource_source::ResourceRoot, sprites::MAX_SPRITE_FRAMES};
use std::{collections::HashMap, fs, path::PathBuf};

const MAGIC: &[u8; 4] = b"HSPR";
/// Bump when `Decoded` or the decoding in `Sprites::decode` changes. (4: a sprite of more than
/// `MAX_SPRITE_FRAMES` frames is refused, so older files may hold entries decoding no longer takes.)
const VERSION: u32 = 4;
const FILE: &str = "horizon-sprites.cache";
/// The most pixel bytes one entry's frames may hold: a full atlas, the most one decoded image
/// may be (`hud::read_png`).
const MAX_ENTRY_BYTES: u64 = (crate::hud::MAX_ATLAS_SIZE as u64).pow(2) * 4;

/// What decoding one texture gives, before it takes a layer.
#[derive(Clone)]
pub(crate) struct Decoded {
    /// The longer side of a frame: its layer's native size.
    pub side: u32,
    /// Each frame's width and height; `frames` are this size, not squared to `side`, until
    /// `Sprites::finish` knows the layers' size.
    pub frame_size: [u32; 2],
    pub frames: Vec<Vec<u8>>,
    /// The `.mcmeta` `animation` value, as text.
    pub animation: Option<String>,
    pub strategy: Strategy,
    pub alpha_cutoff_bias: f32,
    pub has_transparent: bool,
    pub opaque: bool,
    pub translucent: bool,
}

pub(crate) struct SpriteCache {
    path: PathBuf,
    fingerprint: u64,
    entries: HashMap<String, Decoded>,
    dirty: bool,
}

impl SpriteCache {
    /// The cache of `root`: its stored entries when still valid, else empty.
    /// `None` when the textures cannot be fingerprinted (no caching then).
    pub fn open(root: &ResourceRoot) -> Option<Self> {
        let fingerprint = root.source().fingerprint_under("assets", &["textures"])?;
        fs::create_dir_all(root.cache()).ok()?;
        let path = root.cache().join(FILE);
        let entries = fs::read(&path)
            .ok()
            .and_then(|bytes| parse(&bytes, fingerprint))
            .unwrap_or_default();
        Some(Self {
            path,
            fingerprint,
            entries,
            dirty: false,
        })
    }
    pub fn get(&self, name: &str) -> Option<&Decoded> {
        self.entries.get(name)
    }
    pub fn insert(&mut self, name: &str, decoded: Decoded) {
        self.entries.insert(name.to_owned(), decoded);
        self.dirty = true;
    }
    /// Write new entries; a failure only costs the next launch the decoding.
    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let temporary = self
            .path
            .with_extension(format!("{}.tmp", std::process::id()));
        if fs::write(&temporary, serialize(self.fingerprint, &self.entries)).is_ok()
            && fs::rename(&temporary, &self.path).is_err()
        {
            let _ = fs::remove_file(&temporary);
        }
    }
}

fn serialize(fingerprint: u64, entries: &HashMap<String, Decoded>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&fingerprint.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (name, d) in entries {
        put_bytes(&mut out, name.as_bytes());
        out.extend_from_slice(&d.side.to_le_bytes());
        out.extend_from_slice(&d.frame_size[0].to_le_bytes());
        out.extend_from_slice(&d.frame_size[1].to_le_bytes());
        out.extend_from_slice(&(d.frames.len() as u32).to_le_bytes());
        for frame in &d.frames {
            put_bytes(&mut out, frame);
        }
        put_bytes(&mut out, d.animation.as_deref().unwrap_or("").as_bytes());
        out.push(u8::from(d.animation.is_some()));
        out.push(d.strategy as u8);
        out.extend_from_slice(&d.alpha_cutoff_bias.to_bits().to_le_bytes());
        out.push(
            u8::from(d.has_transparent) | u8::from(d.opaque) << 1 | u8::from(d.translucent) << 2,
        );
    }
    out
}
fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, tail) = self.0.split_at_checked(n)?;
        self.0 = tail;
        Some(head)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }
}
/// Whether `count` frames of `frame_len` bytes fit both what is left of the file (each record
/// is a 4-byte length and its bytes) and a full atlas, frame table included.
fn frames_fit(frame_len: u128, count: u32, remaining: usize) -> bool {
    let count = u128::from(count);
    let table_entry = std::mem::size_of::<Vec<u8>>() as u128;
    count * (frame_len + 4) <= remaining as u128
        && count * (frame_len + table_entry) <= u128::from(MAX_ENTRY_BYTES)
}
fn parse(bytes: &[u8], fingerprint: u64) -> Option<HashMap<String, Decoded>> {
    let mut r = Reader(bytes);
    if r.take(4)? != MAGIC || r.u32()? != VERSION {
        return None;
    }
    if u64::from_le_bytes(r.take(8)?.try_into().ok()?) != fingerprint {
        return None;
    }
    let count = r.u32()?;
    let mut entries = HashMap::new();
    for _ in 0..count {
        let name = String::from_utf8(r.bytes()?.to_vec()).ok()?;
        let side = r.u32()?;
        let frame_size = [r.u32()?, r.u32()?];
        let frame_count = r.u32()?;
        // Checked before anything is copied: the shape, then the frames' total against what the
        // file holds and against a full atlas (what one decoded image may be), counting the
        // frame table too, which for tiny frames outweighs their pixels. The table grows only
        // as each record's length checks out.
        if frame_count == 0
            || frame_count > MAX_SPRITE_FRAMES
            || frame_size.contains(&0)
            || frame_size[0].max(frame_size[1]) != side
        {
            return None;
        }
        let frame_len = u128::from(frame_size[0]) * u128::from(frame_size[1]) * 4;
        if !frames_fit(frame_len, frame_count, r.0.len()) {
            return None;
        }
        let mut frames = Vec::new();
        for _ in 0..frame_count {
            let frame = r.bytes()?;
            if frame.len() as u128 != frame_len {
                return None;
            }
            frames.push(frame.to_vec());
        }
        let animation = String::from_utf8(r.bytes()?.to_vec()).ok()?;
        let has_animation = r.take(1)?[0] != 0;
        let strategy = match r.take(1)?[0] {
            0 => Strategy::Auto,
            1 => Strategy::Mean,
            2 => Strategy::Cutout,
            3 => Strategy::StrictCutout,
            4 => Strategy::DarkCutout,
            _ => return None,
        };
        let alpha_cutoff_bias = f32::from_bits(r.u32()?);
        let flags = r.take(1)?[0];
        entries.insert(
            name,
            Decoded {
                side,
                frame_size,
                frames,
                animation: has_animation.then_some(animation),
                strategy,
                alpha_cutoff_bias,
                has_transparent: flags & 1 != 0,
                opaque: flags & 2 != 0,
                translucent: flags & 4 != 0,
            },
        );
    }
    r.0.is_empty().then_some(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn sample(blue: u8, animation: Option<&str>) -> Decoded {
        Decoded {
            side: 2,
            frame_size: [2, 2],
            frames: vec![vec![blue; 16], vec![blue + 1; 16]],
            animation: animation.map(str::to_owned),
            strategy: Strategy::Cutout,
            alpha_cutoff_bias: 0.25,
            has_transparent: true,
            opaque: false,
            translucent: true,
        }
    }
    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("horizon-sprite-cache-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("assets/minecraft/textures/block")).unwrap();
        dir
    }
    #[test]
    fn round_trips() {
        let mut entries = HashMap::new();
        entries.insert("a:b".to_owned(), sample(1, Some("{\"frametime\":2}")));
        entries.insert("a:c".to_owned(), sample(9, None));
        let back = parse(&serialize(7, &entries), 7).unwrap();
        for (name, d) in &entries {
            let e = &back[name];
            assert_eq!(
                (d.side, d.frame_size, &d.frames, &d.animation),
                (e.side, e.frame_size, &e.frames, &e.animation)
            );
            assert_eq!(d.alpha_cutoff_bias, e.alpha_cutoff_bias);
            assert_eq!(
                (d.has_transparent, d.opaque, d.translucent, d.strategy as u8),
                (e.has_transparent, e.opaque, e.translucent, e.strategy as u8)
            );
        }
    }
    #[test]
    fn wrong_fingerprint_truncation_and_garbage_are_misses() {
        let mut entries = HashMap::new();
        entries.insert("a:b".to_owned(), sample(1, None));
        let bytes = serialize(7, &entries);
        assert!(parse(&bytes, 8).is_none());
        for cut in 0..bytes.len() {
            assert!(parse(&bytes[..cut], 7).is_none(), "cut {cut}");
        }
        assert!(parse(b"garbage", 7).is_none());
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(parse(&extra, 7).is_none());
    }
    /// The frame table is charged with the pixels: for tiny frames it is the larger part.
    #[test]
    fn a_frame_table_counts_toward_the_entry_budget() {
        let table = std::mem::size_of::<Vec<u8>>() as u128;
        let tiny = 100_000_000;
        // 1x1 frames: 400 MB of pixels fit a full atlas, their table does not.
        assert!(u128::from(tiny) * 4 <= u128::from(MAX_ENTRY_BYTES));
        assert!(u128::from(tiny) * (4 + table) > u128::from(MAX_ENTRY_BYTES));
        assert!(!frames_fit(4, tiny, usize::MAX));
        assert!(frames_fit(4, 1000, 8000));
        assert!(!frames_fit(4, 1000, 7999));
        assert!(!frames_fit(
            u128::from(u32::MAX) * u128::from(u32::MAX) * 4,
            u32::MAX,
            usize::MAX
        ));
    }
    /// Many 1x1 frames claimed over a short payload: a miss, with nothing reserved for them.
    #[test]
    fn a_huge_count_of_tiny_frames_is_a_miss() {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&VERSION.to_le_bytes());
        bytes.extend_from_slice(&7u64.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        put_bytes(&mut bytes, b"a:b");
        for field in [1u32, 1, 1, 30_000_000] {
            bytes.extend_from_slice(&field.to_le_bytes());
        }
        put_bytes(&mut bytes, &[0; 4]);
        assert!(parse(&bytes, 7).is_none());
    }
    /// An entry declaring far more frames than the file holds is a miss before anything is
    /// reserved for them.
    #[test]
    fn a_huge_frame_count_with_a_short_payload_is_a_miss() {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&VERSION.to_le_bytes());
        bytes.extend_from_slice(&7u64.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        put_bytes(&mut bytes, b"a:b");
        for field in [16, 16, 16, u32::MAX] {
            bytes.extend_from_slice(&field.to_le_bytes());
        }
        put_bytes(&mut bytes, &[0; 16 * 16 * 4]);
        assert!(parse(&bytes, 7).is_none());
    }
    /// An entry with more frames than decoding allows is a miss, as `decode_file_data` would
    /// refuse that sprite: the limit holds for what a cache restores too.
    #[test]
    fn an_entry_above_the_frame_limit_is_a_miss() {
        let entry = |frames: u32| {
            let mut d = sample(1, None);
            (d.side, d.frame_size) = (1, [1, 1]);
            d.frames = vec![vec![9; 4]; frames as usize];
            serialize(7, &HashMap::from([("a:b".to_owned(), d)]))
        };
        let at_limit = parse(&entry(MAX_SPRITE_FRAMES), 7).unwrap();
        assert_eq!(at_limit["a:b"].frames.len(), MAX_SPRITE_FRAMES as usize);
        assert!(parse(&entry(MAX_SPRITE_FRAMES + 1), 7).is_none());
    }
    /// A frame size with a zero side is a miss: its texture is decoded again, never read
    /// from empty frames.
    #[test]
    fn a_zero_frame_side_is_a_miss() {
        let entry = |frame_size: [u32; 2], frames: Vec<Vec<u8>>| {
            let mut d = sample(1, None);
            (d.side, d.frame_size, d.frames) = (16, frame_size, frames);
            serialize(7, &HashMap::from([("a:b".to_owned(), d)]))
        };
        assert!(parse(&entry([0, 16], vec![vec![]]), 7).is_none());
        let wide = parse(&entry([16, 2], vec![vec![5; 16 * 2 * 4]]), 7).unwrap();
        assert_eq!(wide["a:b"].frame_size, [16, 2]);
    }
    #[test]
    fn changed_texture_invalidates_and_corrupt_file_is_ignored() {
        let root = scratch("invalidate");
        let png = root.join("assets/minecraft/textures/block/x.png");
        fs::write(&png, b"one").unwrap();
        let mut cache = SpriteCache::open(&ResourceRoot::directory(root.clone())).unwrap();
        cache.insert("minecraft:block/x", sample(1, None));
        cache.save();
        assert!(
            SpriteCache::open(&ResourceRoot::directory(root.clone()))
                .unwrap()
                .get("minecraft:block/x")
                .is_some()
        );
        // Same length, new content and time: still stale.
        std::thread::sleep(std::time::Duration::from_millis(30));
        fs::write(&png, b"two").unwrap();
        assert!(
            SpriteCache::open(&ResourceRoot::directory(root.clone()))
                .unwrap()
                .get("minecraft:block/x")
                .is_none()
        );
        // A new file also invalidates.
        let mut cache = SpriteCache::open(&ResourceRoot::directory(root.clone())).unwrap();
        cache.insert("minecraft:block/x", sample(1, None));
        cache.save();
        fs::write(root.join("assets/minecraft/textures/block/y.png"), b"y").unwrap();
        assert!(
            SpriteCache::open(&ResourceRoot::directory(root.clone()))
                .unwrap()
                .get("minecraft:block/x")
                .is_none()
        );
        // Corrupt cache file: empty, no error.
        fs::write(root.join(FILE), b"HSPR junk").unwrap();
        assert!(
            SpriteCache::open(&ResourceRoot::directory(root.clone()))
                .unwrap()
                .get("minecraft:block/x")
                .is_none()
        );
        let _ = fs::remove_dir_all(&root);
    }
    #[test]
    fn indexed_archive_cache_uses_the_open_snapshot_and_separate_cache_location() {
        use crate::{
            packs::{self, Pack, Source},
            resource_source::ResourceSource,
        };
        use std::io::Write;
        let root = scratch("archive-snapshot");
        let base = root.join("base");
        fs::create_dir_all(base.join("assets/minecraft/textures/block")).unwrap();
        let name = "assets/minecraft/textures/block/x.png";
        fs::write(base.join(name), b"base").unwrap();
        let jar = root.join("pack.jar");
        let archive = |path: &Path, value: &[u8]| {
            let mut writer = zip::ZipWriter::new(fs::File::create(path).unwrap());
            for name in [name, "assets/minecraft/textures/block/extra.png"] {
                writer
                    .start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(value).unwrap();
            }
            writer.finish().unwrap();
        };
        archive(&jar, b"before");
        let packs = [Pack::mod_layer("jar".into(), Source::Zip(jar.clone()))];
        let source = packs::resources(ResourceSource::directory(base.clone()), &packs).unwrap();
        let before = ResourceRoot::new(base.clone(), root.join("cache"), source);
        let mut cache = SpriteCache::open(&before).unwrap();
        cache.insert("minecraft:block/x", sample(1, None));
        cache.save();
        assert!(
            SpriteCache::open(&before)
                .unwrap()
                .get("minecraft:block/x")
                .is_some()
        );
        assert!(root.join("cache").join(FILE).is_file());
        assert!(!base.join(FILE).exists());
        assert!(
            !base
                .join("assets/minecraft/textures/block/extra.png")
                .exists()
        );
        assert_eq!(fs::read(base.join(name)).unwrap(), b"base");
        let times = fs::FileTimes::new()
            .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(60));
        fs::OpenOptions::new()
            .write(true)
            .open(&jar)
            .unwrap()
            .set_times(times)
            .unwrap();
        let touched = ResourceRoot::new(
            base.clone(),
            root.join("cache"),
            packs::resources(ResourceSource::directory(base.clone()), &packs).unwrap(),
        );
        assert!(
            SpriteCache::open(&touched)
                .unwrap()
                .get("minecraft:block/x")
                .is_some(),
            "marking a downloaded archive used must not invalidate identical bytes"
        );

        let replacement = root.join("new.jar");
        archive(&replacement, b"after");
        fs::rename(&jar, root.join("old.jar")).unwrap();
        fs::rename(replacement, &jar).unwrap();
        let source = packs::resources(ResourceSource::directory(base.clone()), &packs).unwrap();
        let after = ResourceRoot::new(base, root.join("cache"), source);
        // Reopening a pathname for the fingerprint would now misidentify the
        // old source as the new one. Both hashes must follow their open readers.
        assert_eq!(before.source().read(name).unwrap(), b"before");
        assert_eq!(after.source().read(name).unwrap(), b"after");
        assert!(
            SpriteCache::open(&before)
                .unwrap()
                .get("minecraft:block/x")
                .is_some()
        );
        assert!(
            SpriteCache::open(&after)
                .unwrap()
                .get("minecraft:block/x")
                .is_none()
        );
        let mut cache = SpriteCache::open(&after).unwrap();
        cache.insert("minecraft:block/x", sample(2, None));
        cache.save();
        assert!(
            SpriteCache::open(&before)
                .unwrap()
                .get("minecraft:block/x")
                .is_none()
        );
        assert_eq!(
            SpriteCache::open(&after)
                .unwrap()
                .get("minecraft:block/x")
                .unwrap()
                .frames,
            sample(2, None).frames
        );
        let _ = fs::remove_dir_all(root);
    }
    fn write_png(path: &Path, rgba: [u8; 4]) {
        let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), 16, 16);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels: Vec<u8> = rgba.iter().copied().cycle().take(16 * 16 * 4).collect();
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
    }
    fn sprite_pixels(root: &Path) -> Vec<u8> {
        let mut sprites = crate::sprites::Sprites::with_disk_cache(
            &crate::resource_source::ResourceRoot::directory(root.to_owned()),
        );
        sprites.insert("minecraft:block/x").unwrap();
        sprites.save_disk_cache();
        sprites.finish();
        sprites.pixels
    }
    #[test]
    fn altered_png_is_not_served_stale_and_cache_matches_decoding() {
        let root = scratch("stale");
        let png = root.join("assets/minecraft/textures/block/x.png");
        write_png(&png, [255, 0, 0, 255]);
        let decoded = sprite_pixels(&root);
        assert!(root.join(FILE).exists());
        assert_eq!(
            sprite_pixels(&root),
            decoded,
            "cache hit differs from decoding"
        );
        std::thread::sleep(std::time::Duration::from_millis(30));
        write_png(&png, [0, 255, 0, 255]);
        let fresh = sprite_pixels(&root);
        assert_eq!(&fresh[..4], &[0, 255, 0, 255]);
        // Corrupt and deleted caches fall back to decoding.
        fs::write(root.join(FILE), b"HSPR\x01\0\0\0 junk").unwrap();
        assert_eq!(sprite_pixels(&root), fresh);
        fs::remove_file(root.join(FILE)).unwrap();
        assert_eq!(sprite_pixels(&root), fresh);
        let _ = fs::remove_dir_all(&root);
    }
}
