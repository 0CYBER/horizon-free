//! Fonts (FontManager / FontSet): `font/default.json` and every other font a
//! text component names (`minecraft:alt`, `minecraft:uniform`, a pack's own),
//! each a list of providers tried in order: `space`, `bitmap` (sheets in the
//! shared atlas, or rasterized on demand when a pack's sheet is not in it),
//! `unihex` (GNU Unifont) and `ttf` (TrueTypeGlyphProvider, rasterized with
//! ab_glyph). Glyphs drawn at run time go into a reserved atlas region.
//! Vanilla glyph metrics (ascent, oversampling, advances, bold/shadow
//! offsets), styled drawing and StringSplitter-style line wrapping.
use crate::{
    hud::Rect,
    resource_source::ResourceSource,
    text::{Span, Style, Text, legacy_format},
    zip::Zip,
};
use ab_glyph::{Font as _, ScaleFont as _};
use std::{
    borrow::Cow,
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

/// `FontDescription.DEFAULT`.
pub const DEFAULT: &str = "minecraft:default";

/// A glyph ready to draw: atlas UVs and its box relative to the pen
/// position (GlyphBitmap left/top/right/bottom), in GUI pixels.
#[derive(Clone, Copy, Debug, Default)]
pub struct Glyph {
    pub rect: Option<Rect>,
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub advance: f32,
    /// Shadow and bold offset (1 for bitmap and TrueType glyphs, 0.5 for unihex).
    pub offset: f32,
}
/// Pixels to copy into the shared atlas before the next HUD draw.
#[derive(Clone, Debug)]
pub struct Patch {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[derive(Clone, Copy)]
struct HexGlyph {
    /// Rows left-aligned in 32 bits.
    rows: [u32; 16],
    left: u32,
    right: u32,
}
/// Compact entry in a sorted `HexTable`: 12 bytes instead of a 72-byte
/// `HexGlyph` plus `HashMap` bucket overhead, while `rows` holds only the
/// bytes each glyph's width actually uses (1..=4 bytes/row instead of 64).
#[derive(Clone, Copy)]
struct HexEntry {
    codepoint: u32,
    /// Byte offset into `HexTable::rows` (`16 * bytes_per_row` bytes).
    offset: u32,
    left: u8,
    right: u8,
    /// 1, 2, 3 or 4 bytes per row (corresponding to 8, 16, 24 or 32 px wide).
    bytes_per_row: u8,
    /// Set when a `size_overrides` entry gave `left` or `right` > 255; exact
    /// `u32` bounds then live in `HexTable::wide_bounds`.
    wide_bounds: bool,
}
/// Compact, sorted GNU Unifont glyph table shared across every provider with
/// the same `(hex_file, size_overrides)` definition.
struct HexTable {
    /// Sorted by `codepoint` for O(log N) binary-search lookup.
    entries: Vec<HexEntry>,
    /// Packed row bitmaps, `16 * bytes_per_row` bytes per entry.
    rows: Vec<u8>,
    /// Rare `size_overrides` whose `left` or `right` exceeds `u8::MAX`, sorted
    /// by codepoint: `(codepoint, left, right)`. Empty for standard tables.
    wide_bounds: Vec<(u32, u32, u32)>,
}
impl HexTable {
    fn empty() -> Self {
        Self {
            entries: Vec::new(),
            rows: Vec::new(),
            wide_bounds: Vec::new(),
        }
    }
    #[inline]
    fn bounds(&self, e: HexEntry) -> (u32, u32) {
        if e.wide_bounds
            && let Ok(i) = self
                .wide_bounds
                .binary_search_by_key(&e.codepoint, |&(cp, _, _)| cp)
        {
            let (_, left, right) = self.wide_bounds[i];
            return (left, right);
        }
        (u32::from(e.left), u32::from(e.right))
    }
    fn get(&self, codepoint: u32) -> Option<HexGlyph> {
        let idx = self
            .entries
            .binary_search_by_key(&codepoint, |e| e.codepoint)
            .ok()?;
        let e = self.entries[idx];
        let bpr = usize::from(e.bytes_per_row);
        let base = e.offset as usize;
        let mut rows = [0u32; 16];
        for (r, row) in rows.iter_mut().enumerate() {
            let src = base + r * bpr;
            let mut bytes = [0u8; 4];
            bytes[..bpr].copy_from_slice(&self.rows[src..src + bpr]);
            *row = u32::from_be_bytes(bytes);
        }
        let (left, right) = self.bounds(e);
        Some(HexGlyph { rows, left, right })
    }
    fn advance(&self, codepoint: u32) -> Option<f32> {
        let idx = self
            .entries
            .binary_search_by_key(&codepoint, |e| e.codepoint)
            .ok()?;
        let (left, right) = self.bounds(self.entries[idx]);
        let width = right.saturating_sub(left) + 1;
        Some((width / 2 + 1) as f32)
    }
}
/// Cache key identifying a Unihex provider's loaded table: its normalized
/// archive resource path plus its `size_overrides` tuples `(from, to, left, right)`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct UnihexKey {
    path: Option<String>,
    overrides: Vec<(u32, u32, u32, u32)>,
}
impl UnihexKey {
    fn of(p: &serde_json::Value) -> Self {
        let path = p["hex_file"].as_str().map(unihex_resource_path);
        let overrides = p["size_overrides"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|o| {
                let bound = |k: &str| {
                    o[k].as_str()
                        .and_then(|s| s.chars().next())
                        .map_or(0, u32::from)
                };
                (
                    bound("from"),
                    bound("to"),
                    o["left"].as_u64().unwrap_or(0) as u32,
                    o["right"].as_u64().unwrap_or(15) as u32,
                )
            })
            .collect();
        Self { path, overrides }
    }
}
/// Outcome of loading a Unihex archive for a single provider.
enum HexLoad {
    /// Every `.hex` member in the archive was read (including an omitted
    /// `hex_file` or a valid empty archive): safe to share in `Cache::unihex`.
    Complete(HexTable),
    /// The archive opened, but at least one `.hex` member could not be read:
    /// keep all glyphs parsed from readable members for this provider without
    /// caching the partial table in `Cache::unihex` so a later provider can retry.
    Partial(HexTable),
    /// The archive could not be read or opened as a ZIP: leave `Cache::unihex`
    /// unpopulated so a later provider can retry.
    Failed,
}
/// A provider's `filter` (`FontOption.Filter`): the options it needs.
#[derive(Clone, Copy, Default)]
struct Filter {
    uniform: Option<bool>,
    jp: Option<bool>,
}
impl Filter {
    fn of(provider: &serde_json::Value) -> Self {
        let f = &provider["filter"];
        Self {
            uniform: f["uniform"].as_bool(),
            jp: f["jp"].as_bool(),
        }
    }
    fn passes(self, uniform: bool, japanese: bool) -> bool {
        self.uniform.is_none_or(|v| v == uniform) && self.jp.is_none_or(|v| v == japanese)
    }
}
/// Target raw byte ceiling and max cell count per off-atlas bitmap strip chunk:
/// large cells (e.g. 64×64 = 16 KiB) sit in single-cell strips so cold lookups never
/// decompress a whole row, while smaller cells batch up to 8 cells (<= 8 KiB) for
/// LZ4 ratio and are reused through [`StripCache`].
const SHEET_CHUNK_RAW_BYTES: usize = 8 * 1024;
const SHEET_CHUNK_MAX_CELLS: usize = 8;
/// Strips at or below this size stay uncompressed: LZ4 decode overhead does not pay
/// off for tiny strips.
const SHEET_STRIP_MIN_COMPRESS_BYTES: usize = 512;

fn next_strip_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
/// Shared metadata for all strips cut from one off-atlas bitmap sheet so a decode
/// failure names the sheet and warns at most once.
struct SheetInfo {
    file: Box<str>,
    decode_warned: AtomicBool,
}
impl SheetInfo {
    fn new(file: &str) -> Self {
        Self {
            file: Box::from(file),
            decode_warned: AtomicBool::new(false),
        }
    }
    fn warn_decode_once(&self, detail: impl std::fmt::Display) {
        if !self.decode_warned.swap(true, Ordering::Relaxed) {
            log::warn!(
                "font sheet {} strip could not be decompressed ({detail})",
                self.file
            );
        }
    }
}
enum StripPixels {
    Raw(Vec<u8>),
    Lz4 { len: usize, compressed: Vec<u8> },
}
impl StripPixels {
    fn from_raw(raw: Cow<'_, [u8]>) -> Self {
        let len = raw.len();
        if len > SHEET_STRIP_MIN_COMPRESS_BYTES {
            let mut compressed = lz4_flex::block::compress(raw.as_ref());
            if compressed.len() < len {
                compressed.shrink_to_fit();
                return Self::Lz4 { len, compressed };
            }
        }
        Self::Raw(raw.into_owned())
    }
}
/// One horizontal cell chunk of an off-atlas bitmap font sheet, LZ4-compressed when
/// compression shrinks it so custom HUD/font packs do not keep megabytes of raw RGBA
/// sheets in RAM.
struct SheetStrip {
    id: u64,
    info: Arc<SheetInfo>,
    stride: u32,
    cacheable: bool,
    pixels: StripPixels,
}
impl SheetStrip {
    fn new(info: Arc<SheetInfo>, stride: u32, raw: Cow<'_, [u8]>, cacheable: bool) -> Self {
        Self {
            id: next_strip_id(),
            info,
            stride,
            cacheable,
            pixels: StripPixels::from_raw(raw),
        }
    }
    fn decompress(&self) -> Vec<u8> {
        match &self.pixels {
            StripPixels::Raw(raw) => raw.clone(),
            StripPixels::Lz4 { len, compressed } => {
                let expected = *len;
                #[cfg(test)]
                tests::record_strip_decoded(expected);
                let mut out = vec![0; expected];
                match lz4_flex::block::decompress_into(compressed, &mut out) {
                    Ok(decoded) if decoded == expected => out,
                    Ok(decoded) => {
                        self.info.warn_decode_once(format_args!(
                            "decoded {decoded} bytes, expected {expected}"
                        ));
                        out.fill(0);
                        out
                    }
                    Err(err) => {
                        self.info.warn_decode_once(err);
                        out.fill(0);
                        out
                    }
                }
            }
        }
    }
    #[cfg(test)]
    fn stored_bytes(&self) -> usize {
        match &self.pixels {
            StripPixels::Raw(raw) => raw.len(),
            StripPixels::Lz4 { compressed, .. } => compressed.len(),
        }
    }
}
/// Bounded MRU cache of recently decompressed multi-cell off-atlas bitmap strips so
/// resolving neighboring cells in the same chunk avoids repeated LZ4 decompression.
#[derive(Default)]
struct StripCache {
    entries: Vec<(u64, Vec<u8>)>,
}
impl StripCache {
    const MAX_ENTRIES: usize = 8;
    fn get_or_decode(&mut self, strip: &SheetStrip) -> &[u8] {
        if let Some(pos) = self.entries.iter().position(|(id, _)| *id == strip.id) {
            if pos != 0 {
                self.entries[..=pos].rotate_right(1);
            }
            return &self.entries[0].1;
        }
        let raw = strip.decompress();
        if self.entries.len() == Self::MAX_ENTRIES {
            self.entries.pop();
        }
        self.entries.insert(0, (strip.id, raw));
        &self.entries[0].1
    }
    fn clear(&mut self) {
        self.entries.clear();
    }
}
/// A bitmap glyph whose sheet is not in the atlas: the cell to copy.
struct SheetCell {
    sheet: Arc<SheetStrip>,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    glyph: Glyph,
}
impl SheetCell {
    /// The cell's RGBA rows, or `None` where the cell reaches past the strip,
    /// reusing recently decompressed multi-cell strips via `strips`.
    fn pixels(&self, strips: &mut StripCache) -> Option<Vec<u8>> {
        match &self.sheet.pixels {
            StripPixels::Raw(raw) => self.extract_pixels(raw),
            StripPixels::Lz4 { .. } => {
                if self.sheet.cacheable {
                    let raw = strips.get_or_decode(&self.sheet);
                    self.extract_pixels(raw)
                } else {
                    let raw = self.sheet.decompress();
                    self.extract_pixels(&raw)
                }
            }
        }
    }
    fn extract_pixels(&self, pixels: &[u8]) -> Option<Vec<u8>> {
        let stride = self.sheet.stride as usize;
        let x = self.x as usize;
        let y = self.y as usize;
        let w = self.w as usize;
        let h = self.h as usize;
        if x.checked_add(w)? > stride {
            return None;
        }
        let row_bytes = w.checked_mul(4)?;
        let mut rgba = Vec::with_capacity(row_bytes.checked_mul(h)?);
        for dy in 0..h {
            let at = y
                .checked_add(dy)?
                .checked_mul(stride)?
                .checked_add(x)?
                .checked_mul(4)?;
            rgba.extend_from_slice(pixels.get(at..at.checked_add(row_bytes)?)?);
        }
        Some(rgba)
    }
}
enum BitmapGlyph {
    Atlas(Glyph),
    Sheet(SheetCell),
}
/// `TrueTypeGlyphProvider`.
struct TrueType {
    font: ab_glyph::FontVec,
    /// Canonical codepoint per glyph ID from `ab_glyph::Font::codepoint_ids()`,
    /// preserving the exact first-codepoint-per-glyph set without a per-char `HashMap`.
    canonical: Box<[Option<char>]>,
    /// Sorted `(char, GlyphId)` overrides for the rare multi-subtable case where
    /// `codepoint_ids()` and `font.glyph_id()` map a character to different IDs.
    overrides: Box<[(char, ab_glyph::GlyphId)]>,
    /// FT_Set_Pixel_Sizes: `size * oversample` pixels to the em.
    scale: ab_glyph::PxScale,
    oversample: f32,
    /// FT_Set_Transform's delta, in oversampled pixels (y down).
    shift: (f32, f32),
}
enum Provider {
    Space(HashMap<char, f32>),
    Bitmap(Filter, HashMap<char, BitmapGlyph>),
    /// The filter, cache key while unloaded (cleared once read), and the
    /// shared compact glyph table once read.
    Unihex(Filter, Option<Box<UnihexKey>>, Option<Arc<HexTable>>),
    TrueType(Filter, Box<TrueType>),
}
/// One font (`FontSet`): its providers and the glyphs resolved so far.
struct FontSet {
    providers: Vec<Provider>,
    /// Every char drawn looks up here, so the hasher is Fx, not SipHash.
    glyphs: crate::fast_hash::FxHashMap<char, Glyph>,
    /// `SelectedGlyphs.nonFishy` of the characters whose first glyph has a fishy advance (the
    /// others use `glyphs`): what obfuscated text draws.
    non_fishy: crate::fast_hash::FxHashMap<char, Glyph>,
    /// `glyphsByWidth` for obfuscated text (built on first use), by `ceil(advance)`.
    by_width: Option<HashMap<i32, Vec<char>>>,
}
/// The atlas region glyphs are rasterized into, filled in rows.
struct Region {
    cursor: (u32, u32),
    row: u32,
    pending: Vec<Patch>,
    /// A glyph did not fit since the last `Font::begin_frame`.
    full: bool,
}
struct Cache {
    /// Every font by id, the default one first (the common lookup).
    sets: Vec<(String, FontSet)>,
    region: Region,
    strips: StripCache,
    /// Loaded Unihex tables shared across font sets and filter variants that
    /// reference the same archive and `size_overrides`.
    unihex: HashMap<UnihexKey, Arc<HexTable>>,
    /// `Font.random`: picks obfuscated glyphs.
    random: horizon_core::random::LegacyRandom,
}
pub struct Font {
    /// The atlas rects of every texture, by resource name: a bitmap provider's
    /// `file` may name any texture, not only one under `textures/font`.
    sheets: HashMap<String, Rect>,
    missing: Glyph,
    white: Rect,
    source: ResourceSource,
    /// Atlas size and the reserved dynamic region (y, height).
    atlas: (u32, u32),
    region: (u32, u32),
    metrics_only: bool,
    cache: RefCell<Cache>,
    /// Set anew for each font and each `reset_cache` (see `epoch`).
    generation: std::cell::Cell<u64>,
    /// "Force Unicode Font": the `uniform` filter option.
    pub uniform: bool,
    /// "Japanese Glyph Variants": the `jp` filter option.
    pub japanese: bool,
}
#[inline]
fn parse_hex_digit(b: u8) -> Option<u32> {
    match b {
        b'0'..=b'9' => Some(u32::from(b - b'0')),
        b'a'..=b'f' => Some(u32::from(b - b'a' + 10)),
        b'A'..=b'F' => Some(u32::from(b - b'A' + 10)),
        _ => None,
    }
}
#[inline]
fn parse_hex_byte(hi: u8, lo: u8) -> Option<u8> {
    Some(((parse_hex_digit(hi)? << 4) | parse_hex_digit(lo)?) as u8)
}
/// UnihexProvider.readFromStream over one `.hex` file into compact row storage.
fn parse_hex(
    text: &[u8],
    entries: &mut Vec<HexEntry>,
    rows_buf: &mut Vec<u8>,
    by_cp: &mut crate::fast_hash::FxHashMap<u32, usize>,
    has_dead_rows: &mut bool,
) {
    for line in text.split(|b| *b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some(colon) = line.iter().position(|b| *b == b':') else {
            continue;
        };
        let (code, data) = (&line[..colon], &line[colon + 1..]);
        if !(4..=6).contains(&code.len()) {
            continue;
        }
        let Some(codepoint) = code
            .iter()
            .try_fold(0u32, |a, b| parse_hex_digit(*b).map(|d| a << 4 | d))
        else {
            continue;
        };
        let bpr = match data.len() {
            32 => 1usize,
            64 => 2,
            96 => 3,
            128 => 4,
            _ => continue,
        };
        let bit_width = (bpr as u32) * 8;
        let mut row_bytes = [0u8; 64];
        let mut mask = 0u32;
        let mut ok = true;
        for r in 0..16 {
            let mut be = [0u8; 4];
            for j in 0..bpr {
                let pos = (r * bpr + j) * 2;
                match parse_hex_byte(data[pos], data[pos + 1]) {
                    Some(byte) => {
                        row_bytes[r * bpr + j] = byte;
                        be[j] = byte;
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                break;
            }
            mask |= u32::from_be_bytes(be);
        }
        if !ok {
            continue;
        }
        // LineData.calculateWidth.
        let (left, right) = if mask == 0 {
            (0u8, bit_width as u8)
        } else {
            (
                mask.leading_zeros() as u8,
                (31 - mask.trailing_zeros()) as u8,
            )
        };
        let slice = &row_bytes[..16 * bpr];
        match by_cp.entry(codepoint) {
            std::collections::hash_map::Entry::Occupied(occ) => {
                let existing = &mut entries[*occ.get()];
                if usize::from(existing.bytes_per_row) == bpr {
                    let start = existing.offset as usize;
                    rows_buf[start..start + slice.len()].copy_from_slice(slice);
                } else {
                    let Ok(offset) = u32::try_from(rows_buf.len()) else {
                        continue;
                    };
                    existing.offset = offset;
                    existing.bytes_per_row = bpr as u8;
                    rows_buf.extend_from_slice(slice);
                    *has_dead_rows = true;
                }
                existing.left = left;
                existing.right = right;
            }
            std::collections::hash_map::Entry::Vacant(vac) => {
                let Ok(offset) = u32::try_from(rows_buf.len()) else {
                    continue;
                };
                rows_buf.extend_from_slice(slice);
                vac.insert(entries.len());
                entries.push(HexEntry {
                    codepoint,
                    offset,
                    left,
                    right,
                    bytes_per_row: bpr as u8,
                    wide_bounds: false,
                });
            }
        }
    }
}
fn read_json(source: &ResourceSource, path: &str) -> Option<serde_json::Value> {
    serde_json::from_slice(&source.read(path).ok()?).ok()
}
fn resource_path(id: &str, folder: &str, ext: &str) -> String {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    format!("assets/{ns}/{folder}/{}{ext}", path.trim_end_matches(ext))
}
fn unihex_resource_path(file: &str) -> String {
    resource_path(file, "", "").replace("//", "/")
}
/// A bitmap provider's `height` (8 when missing or null) and `ascent`, as
/// vanilla decodes them: any JSON number taken as an integer, a fraction
/// truncated toward zero, saturated to the 32-bit range (vanilla wraps there).
/// `None` when either isn't a number or `ascent` is missing.
fn bitmap_height_ascent(p: &serde_json::Value) -> Option<(i32, i32)> {
    let int = |v: &serde_json::Value| {
        v.as_i64()
            .or_else(|| v.as_f64().map(|f| f as i64))
            .map(|v| v.clamp(i32::MIN.into(), i32::MAX.into()) as i32)
    };
    let height = if p["height"].is_null() {
        Some(8)
    } else {
        int(&p["height"])
    };
    Some((height?, int(&p["ascent"])?))
}
/// Why vanilla refuses a bitmap provider while its font file decodes
/// (`BitmapProvider.Definition`): `height` or `ascent` not a number (or no
/// `ascent`), `ascent` above `height`, or a `chars` grid that is empty or whose
/// rows differ in length, counted in code points as `String.codePoints` does
/// (a Rust `char` is one). Other providers pass.
fn bitmap_error(p: &serde_json::Value) -> Option<String> {
    if p["type"] != "bitmap" {
        return None;
    }
    let file = p["file"].as_str().unwrap_or("");
    let Some((height, ascent)) = bitmap_height_ascent(p) else {
        return Some(format!(
            "bitmap {file}: height and ascent have to be numbers"
        ));
    };
    if ascent > height {
        return Some(format!(
            "bitmap {file}: ascent {ascent} higher than height {height}"
        ));
    }
    let rows = p["chars"].as_array().map(|rows| {
        rows.iter()
            .map(|r| r.as_str().map(|s| s.chars().count()))
            .collect::<Option<Vec<_>>>()
    });
    let Some(Some(rows)) = rows else {
        return Some(format!("bitmap {file}: chars has to be a list of strings"));
    };
    match rows.first() {
        None | Some(0) => Some(format!("bitmap {file}: chars has no characters")),
        Some(&width) => rows.iter().find(|&&r| r != width).map(|r| {
            format!(
                "bitmap {file}: rows of chars have to be the same length \
                 (found: {r} code points, expected: {width})"
            )
        }),
    }
}
/// The first of a font file's `providers` that vanilla refuses, and why: it
/// then drops the whole file of that pack (`FontManager.loadResourceStack`).
pub(crate) fn font_file_error(providers: &serde_json::Value) -> Option<String> {
    providers
        .as_array()
        .into_iter()
        .flatten()
        .find_map(bitmap_error)
}
/// A bound on what one font takes in through `reference` providers: each reference followed and
/// each provider it brings in count one. Vanilla's `default` takes in a few dozen
/// (`include/default`, `include/space`, `include/unifont`); a server pack that keeps one bitmap
/// provider per glyph in an included font takes in thousands to tens of thousands (the first
/// plane's private use area alone has 6400 code points), which fits; an included font with more
/// than about 65000 providers is cut. A font's own providers never count, so a pack that lists
/// every glyph in `font/default.json` keeps them all. Without the bound, a font that names
/// another several times at each level of nesting would copy its providers exponentially. It
/// is not higher because `load_bitmap` reads and decodes an off-atlas sheet again for every
/// copy of its provider.
const MAX_REFERENCED_PROVIDERS: usize = 1 << 16;

/// Providers of a font definition with references expanded, in order, with
/// their `filter` objects. Each font file is read once per load, however often
/// and under whichever spelling (`default`, `minecraft:default`) it is named:
/// `FontManager` builds each font once. A reference back to a font that is
/// still being expanded is a cycle and adds nothing. Vanilla drops the whole
/// font instead, as it does for a reference to a missing font; Horizon keeps
/// the font's other providers in both cases.
fn providers(source: &ResourceSource, id: &str) -> Vec<serde_json::Value> {
    let mut walk = ReferenceWalk {
        source,
        files: HashMap::new(),
        open: Vec::new(),
        budget: MAX_REFERENCED_PROVIDERS,
        logged: HashSet::new(),
    };
    let mut out = Vec::new();
    walk.expand(&resource_path(id, "font", ".json"), 0, &mut out);
    out
}
/// One font's expansion: the files read so far and the references followed.
struct ReferenceWalk<'a> {
    source: &'a ResourceSource,
    /// Each font file's providers by path, `None` when it is missing or refused.
    files: HashMap<String, Option<Rc<Vec<serde_json::Value>>>>,
    /// Paths of the fonts being expanded, outermost first.
    open: Vec<String>,
    /// References and referenced providers still allowed (`MAX_REFERENCED_PROVIDERS`).
    budget: usize,
    /// Fonts whose cycle, and the font whose cut, were logged already.
    logged: HashSet<String>,
}
impl ReferenceWalk<'_> {
    fn file(&mut self, path: &str) -> Option<Rc<Vec<serde_json::Value>>> {
        if let Some(file) = self.files.get(path) {
            return file.clone();
        }
        #[cfg(test)]
        tests::record_font_read();
        let file = read_json(self.source, path).and_then(|mut def| {
            if let Some(error) = font_file_error(&def["providers"]) {
                log::error!("font {path} skipped: {error}");
                return None;
            }
            let list = match def.get_mut("providers").map(serde_json::Value::take) {
                Some(serde_json::Value::Array(list)) => list,
                _ => Vec::new(),
            };
            Some(Rc::new(list))
        });
        self.files.insert(path.to_owned(), file.clone());
        file
    }
    fn cut(&mut self) {
        let top = self.open.first().cloned().unwrap_or_default();
        if self.logged.insert(format!("cut {top}")) {
            log::error!(
                "font {top}: references past {MAX_REFERENCED_PROVIDERS} providers left out"
            );
        }
    }
    fn expand(&mut self, path: &str, depth: u32, out: &mut Vec<serde_json::Value>) {
        if depth > 8 {
            return;
        }
        let Some(list) = self.file(path) else {
            return;
        };
        self.open.push(path.to_owned());
        for p in list.iter() {
            if p["type"] == "reference" {
                if self.budget == 0 {
                    self.cut();
                    continue;
                }
                self.budget -= 1;
                let target = resource_path(p["id"].as_str().unwrap_or(""), "font", ".json");
                if self.open.contains(&target) {
                    if self.logged.insert(format!("cycle {target}")) {
                        log::error!(
                            "font {target} skipped where {path} references it: \
                             it references itself through {}",
                            self.open.join(" -> ")
                        );
                    }
                    continue;
                }
                let start = out.len();
                self.expand(&target, depth + 1, out);
                // A reference's filter applies to everything it includes.
                if let Some(f) = p["filter"].as_object() {
                    for n in out[start..].iter_mut().filter(|n| n.is_object()) {
                        let merged = n["filter"]
                            .as_object()
                            .cloned()
                            .unwrap_or_default()
                            .into_iter()
                            .chain(f.clone())
                            .collect();
                        n["filter"] = serde_json::Value::Object(merged);
                    }
                }
            } else if depth == 0 {
                out.push(p.clone());
            } else if self.budget > 0 {
                self.budget -= 1;
                out.push(p.clone());
            } else {
                self.cut();
            }
        }
        self.open.pop();
    }
}
// FontTexture.add nudges UVs inside the glyph to avoid neighboring texels.
fn glyph_rect(x: u32, y: u32, w: u32, h: u32, atlas: (u32, u32)) -> Rect {
    Rect {
        u0: (x as f32 + 0.01) / atlas.0 as f32,
        v0: (y as f32 + 0.01) / atlas.1 as f32,
        u1: ((x + w) as f32 - 0.01) / atlas.0 as f32,
        v1: ((y + h) as f32 - 0.01) / atlas.1 as f32,
        w,
        h,
    }
}
/// The unscaled pixel width of a bitmap cell (`BitmapProvider.getActualGlyphWidth`).
fn used_width(pixels: &[u8], stride: u32, (cx, cy, cw, ch): (u32, u32, u32, u32)) -> u32 {
    (0..cw)
        .rev()
        .find(|&x| {
            (0..ch).any(|y| {
                pixels
                    .get((((cy + y) * stride + cx + x) * 4 + 3) as usize)
                    .is_some_and(|a| *a != 0)
            })
        })
        .map_or(0, |x| x + 1)
}
/// `FontSet.hasFishyAdvance`: an advance under 0 or over 32. Such a glyph is still the one drawn
/// (`SelectedGlyphs.any` is the first provider's); only `SelectedGlyphs.nonFishy`, what obfuscated
/// text draws, moves on to the next provider that has the character. Packs rely on it: a `space`
/// provider's kerning runs from -512 to 512, and a tall bitmap's glyphs pass 32. Horizon keeps one
/// advance per glyph and tests it here.
fn fishy_advance(advance: f32) -> bool {
    !(0.0..=32.0).contains(&advance)
}
/// A Unihex glyph's advance (`UnihexProvider.Glyph.info().getAdvance`): half its width
/// plus one, the width being the bits from its left to its right column.
fn hex_advance(h: &HexGlyph) -> f32 {
    let width = h.right.saturating_sub(h.left) + 1;
    (width / 2 + 1) as f32
}
/// `BitmapProvider.Definition.load`: each character's glyph, from the atlas
/// when the sheet is in it (`pixels` its RGBA), else from the sheet's own
/// pixels, copied into the atlas when first drawn.
fn load_bitmap(
    source: &ResourceSource,
    p: &serde_json::Value,
    sheets: &HashMap<String, Rect>,
    pixels: &[u8],
    atlas: (u32, u32),
    metrics_only: bool,
) -> HashMap<char, BitmapGlyph> {
    let mut out = HashMap::new();
    let file = p["file"].as_str().unwrap_or("");
    let (ns, name) = file.split_once(':').unwrap_or(("minecraft", file));
    let name = name.trim_end_matches(".png");
    // The atlas names the default namespace's textures by their path and every
    // other namespace's as `namespace:path` (HudAtlas::build).
    let atlas_name = if ns == "minecraft" {
        name.to_owned()
    } else {
        format!("{ns}:{name}")
    };
    // Checked with the whole file (`font_file_error`): rows of one length, none empty.
    let rows: Vec<Vec<char>> = p["chars"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r.as_str().map(|s| s.chars().collect()))
        .collect();
    let Some(width) = rows.first().map(Vec::len).filter(|w| *w > 0) else {
        return out;
    };
    let in_atlas = sheets.get(&atlas_name).copied();
    // The sheet: its size, and its pixels when they are not in the atlas.
    let (sw, sh, sheet) = match in_atlas {
        Some(rect) => {
            let (w, h) = rect.texels(atlas);
            (w, h, None)
        }
        None => match source
            .read(&resource_path(file, "textures", ""))
            .map_err(Into::into)
            .and_then(|bytes| crate::hud::decode_bytes(&bytes))
        {
            Ok((w, h, rgba)) => (w, h, Some((w, rgba))),
            Err(_) => {
                log::warn!("font sheet {file} could not be read");
                return out;
            }
        },
    };
    let cell_w = sw / width as u32;
    let cell_h = sh / rows.len() as u32;
    // Checked with the whole file (`font_file_error`).
    let (height, ascent) = bitmap_height_ascent(p).unwrap_or((8, 7));
    let (height, ascent) = (height as f32, ascent as f32);
    let top = 7.0 - ascent;
    // A grid with more rows or columns than the sheet has pixels: each
    // character is an empty glyph of advance 1, as vanilla's arithmetic ends
    // there, and still kept from later providers.
    if cell_w == 0 || cell_h == 0 {
        for ch in rows.iter().flatten().filter(|c| **c != '\0') {
            out.entry(*ch).or_insert(BitmapGlyph::Atlas(Glyph {
                rect: None,
                left: 0.0,
                top,
                right: 0.0,
                bottom: top,
                advance: 1.0,
                offset: 1.0,
            }));
        }
        return out;
    }
    let scale = height / cell_h as f32;
    let origin = in_atlas.map_or((0, 0), |rect| {
        (
            (rect.u0 * atlas.0 as f32).round() as u32,
            (rect.v0 * atlas.1 as f32).round() as u32,
        )
    });
    let cell_bytes = (cell_w as usize)
        .saturating_mul(cell_h as usize)
        .saturating_mul(4);
    let chunk_cols = (SHEET_CHUNK_RAW_BYTES / cell_bytes.max(1)).clamp(1, SHEET_CHUNK_MAX_CELLS);
    let mut sheet_info: Option<Arc<SheetInfo>> = None;
    for (r, row) in rows.iter().enumerate() {
        // Horizontal cell chunks per row: large cells stay single-cell strips so
        // cold lookups never decompress a whole row, while smaller cells batch up
        // to `SHEET_CHUNK_MAX_CELLS` (<= `SHEET_CHUNK_RAW_BYTES`) for LZ4 ratio.
        let mut row_strip: Option<(usize, Arc<SheetStrip>)> = None;
        for (c, ch) in row.iter().enumerate() {
            // Character 0 pads the grid. A character listed twice keeps its
            // last cell, as vanilla's `Map.put` does.
            if *ch == '\0' {
                continue;
            }
            let (x, y) = (c as u32 * cell_w, r as u32 * cell_h);
            let used = match &sheet {
                Some((stride, rgba)) => used_width(rgba, *stride, (x, y, cell_w, cell_h)),
                None => used_width(
                    pixels,
                    atlas.0,
                    (origin.0 + x, origin.1 + y, cell_w, cell_h),
                ),
            };
            let glyph = Glyph {
                rect: None,
                left: 0.0,
                top,
                right: cell_w as f32 * scale,
                bottom: top + cell_h as f32 * scale,
                // `(int) (0.5 + width * scale)`: truncated toward zero, so a negative
                // `height` gives a cell an advance one more than flooring would.
                advance: (0.5 + used as f32 * scale).trunc() + 1.0,
                offset: 1.0,
            };
            if metrics_only {
                out.insert(*ch, BitmapGlyph::Atlas(glyph));
                continue;
            }
            let glyph = match &sheet {
                None => BitmapGlyph::Atlas(Glyph {
                    rect: Some(glyph_rect(
                        origin.0 + x,
                        origin.1 + y,
                        cell_w,
                        cell_h,
                        atlas,
                    )),
                    ..glyph
                }),
                Some((stride, rgba)) => {
                    let chunk_start_col = (c / chunk_cols) * chunk_cols;
                    let cached = row_strip
                        .as_ref()
                        .filter(|(start, _)| *start == chunk_start_col)
                        .map(|(_, strip)| Arc::clone(strip));
                    let strip = cached.unwrap_or_else(|| {
                        let chunk_end_col = (chunk_start_col + chunk_cols).min(row.len());
                        let chunk_x = chunk_start_col as u32 * cell_w;
                        let chunk_stride = (chunk_end_col - chunk_start_col) as u32 * cell_w;
                        let raw = if chunk_start_col == 0 && chunk_stride == *stride {
                            let start = (y as usize) * (*stride as usize) * 4;
                            let end = ((y + cell_h) as usize) * (*stride as usize) * 4;
                            Cow::Borrowed(rgba.get(start..end).unwrap_or_default())
                        } else {
                            let row_bytes = chunk_stride as usize * 4;
                            let mut buf = Vec::with_capacity(row_bytes * cell_h as usize);
                            for dy in 0..cell_h {
                                let src = (((y + dy) as usize) * (*stride as usize)
                                    + chunk_x as usize)
                                    * 4;
                                if let Some(line) = src
                                    .checked_add(row_bytes)
                                    .and_then(|end| rgba.get(src..end))
                                {
                                    buf.extend_from_slice(line);
                                }
                            }
                            Cow::Owned(buf)
                        };
                        let active_cells = row[chunk_start_col..chunk_end_col]
                            .iter()
                            .filter(|&&cell_ch| cell_ch != '\0')
                            .count();
                        let info = Arc::clone(
                            sheet_info.get_or_insert_with(|| Arc::new(SheetInfo::new(file))),
                        );
                        let strip =
                            Arc::new(SheetStrip::new(info, chunk_stride, raw, active_cells > 1));
                        row_strip = Some((chunk_start_col, Arc::clone(&strip)));
                        strip
                    });
                    BitmapGlyph::Sheet(SheetCell {
                        sheet: strip,
                        x: (c - chunk_start_col) as u32 * cell_w,
                        y: 0,
                        w: cell_w,
                        h: cell_h,
                        glyph,
                    })
                }
            };
            out.insert(*ch, glyph);
        }
    }
    out
}
/// `TrueTypeGlyphProviderDefinition.load`: a TrueType file (not CFF) under
/// `font/`, its Unicode characters but the `skip` ones.
fn load_true_type(source: &ResourceSource, p: &serde_json::Value) -> Option<TrueType> {
    let file = p["file"].as_str()?;
    let data = source
        .read(&resource_path(file, "font", ""))
        .map_err(|e| log::warn!("font file {file}: {e}"))
        .ok()?;
    // FT_Get_Font_Format must say TrueType: CFF outlines ("OTTO") are refused.
    if data.starts_with(b"OTTO") {
        log::warn!("font file {file} is not in TTF format");
        return None;
    }
    let font = ab_glyph::FontVec::try_from_vec(data)
        .map_err(|e| log::warn!("font file {file}: {e}"))
        .ok()?;
    let size = p["size"].as_f64().unwrap_or(11.0) as f32;
    let oversample = p["oversample"].as_f64().unwrap_or(1.0) as f32;
    let shift = p["shift"].as_array().map_or((0.0, 0.0), |s| {
        let f = |i: usize| s.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        (f(0), f(1))
    });
    let mut skip: Vec<char> = match &p["skip"] {
        serde_json::Value::String(s) => s.chars().collect(),
        serde_json::Value::Array(a) => a
            .iter()
            .filter_map(|v| v.as_str())
            .flat_map(str::chars)
            .collect(),
        _ => Vec::new(),
    };
    skip.sort_unstable();
    skip.dedup();
    let mut canonical = vec![None; font.glyph_count()];
    let mut overrides: Vec<(char, ab_glyph::GlyphId)> = Vec::new();
    for (id, c) in font.codepoint_ids() {
        if id.0 == 0 || skip.binary_search(&c).is_ok() {
            continue;
        }
        if font.glyph_id(c) == id {
            let idx = usize::from(id.0);
            if idx >= canonical.len() {
                canonical.resize(idx + 1, None);
            }
            if canonical[idx].is_none_or(|prev| prev == c) {
                canonical[idx] = Some(c);
                if !overrides.is_empty() {
                    overrides.retain(|(ch, _)| *ch != c);
                }
                continue;
            }
        }
        if let Some((_, existing)) = overrides.iter_mut().find(|(ch, _)| *ch == c) {
            *existing = id;
        } else {
            overrides.push((c, id));
        }
    }
    let used_len = canonical
        .iter()
        .rposition(Option::is_some)
        .map_or(0, |i| i + 1);
    canonical.truncate(used_len);
    overrides.sort_unstable_by_key(|(c, _)| *c);
    let pixels_per_em = (size * oversample).round();
    // ab_glyph scales by the font's height; FreeType by its em.
    let units = font.units_per_em().unwrap_or(1000.0);
    let scale = ab_glyph::PxScale::from(pixels_per_em * font.height_unscaled() / units);
    Some(TrueType {
        font,
        canonical: canonical.into_boxed_slice(),
        overrides: overrides.into_boxed_slice(),
        scale,
        oversample,
        shift: (shift.0 * oversample, shift.1 * oversample),
    })
}
/// Result of checking a TrueType glyph's scaled outline against the atlas bounds.
enum TrueTypeOutline {
    /// No drawable outline (e.g. spaces): still fits as an empty glyph.
    Empty,
    /// Non-empty outline whose pixel bounds fit within the atlas limit.
    Drawable(ab_glyph::OutlinedGlyph, u32, u32),
    /// Outline exceeds the atlas limit and must be refused so later providers can resolve it.
    Oversized,
}
impl TrueType {
    #[inline]
    fn glyph_id(&self, c: char) -> Option<ab_glyph::GlyphId> {
        if !self.overrides.is_empty()
            && let Ok(idx) = self.overrides.binary_search_by_key(&c, |(ch, _)| *ch)
        {
            return Some(self.overrides[idx].1);
        }
        let id = self.font.glyph_id(c);
        (id.0 != 0 && self.canonical.get(usize::from(id.0)).copied().flatten() == Some(c))
            .then_some(id)
    }
    /// The advance in GUI pixels: hinted FreeType advances are whole
    /// oversampled pixels.
    fn advance(&self, id: ab_glyph::GlyphId) -> f32 {
        self.font.as_scaled(self.scale).h_advance(id).round() / self.oversample
    }
    /// Checks whether `id`'s scaled outline fits within `max` atlas pixels without
    /// allocating raster pixels. Shared by `glyph` (via `Font::resolve`) and
    /// `Font::provider_advance` so oversized glyphs fall through identically in both.
    fn outline_for_atlas(&self, id: ab_glyph::GlyphId, max: (u32, u32)) -> TrueTypeOutline {
        let glyph =
            id.with_scale_and_position(self.scale, ab_glyph::point(self.shift.0, self.shift.1));
        let Some(outline) = self.font.outline_glyph(glyph) else {
            return TrueTypeOutline::Empty;
        };
        let bounds = outline.px_bounds();
        let (w, h) = (bounds.width() as u32, bounds.height() as u32);
        if w == 0 || h == 0 {
            return TrueTypeOutline::Empty;
        }
        if w > max.0 || h > max.1 {
            return TrueTypeOutline::Oversized;
        }
        TrueTypeOutline::Drawable(outline, w, h)
    }
    /// Whether `id` fits within `max` atlas pixels (empty-outline glyphs such as spaces fit).
    fn fits_atlas(&self, id: ab_glyph::GlyphId, max: (u32, u32)) -> bool {
        !matches!(self.outline_for_atlas(id, max), TrueTypeOutline::Oversized)
    }
    /// `loadGlyph` and its bitmap: the grey coverage as `rrrr` texels
    /// (`text_intensity`), below 0.1 cut (its discard). A glyph wider or taller than `max`
    /// (what the atlas can hold) is refused before its bitmap is allocated: a pack's `size`
    /// and `oversample` are otherwise unbounded.
    fn glyph(
        &self,
        id: ab_glyph::GlyphId,
        max: (u32, u32),
    ) -> Option<(Glyph, Option<(u32, u32, Vec<u8>)>)> {
        let advance = self.advance(id);
        let empty = Glyph {
            rect: None,
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            advance,
            offset: 1.0,
        };
        let (outline, w, h) = match self.outline_for_atlas(id, max) {
            TrueTypeOutline::Empty => return Some((empty, None)),
            TrueTypeOutline::Oversized => return None,
            TrueTypeOutline::Drawable(outline, w, h) => (outline, w, h),
        };
        let bounds = outline.px_bounds();
        let mut rgba = vec![0u8; w as usize * h as usize * 4];
        outline.draw(|x, y, coverage| {
            let v = if coverage < 0.1 {
                0
            } else {
                (coverage.min(1.0) * 255.0).round() as u8
            };
            let i = ((y * w + x) * 4) as usize;
            if let Some(px) = rgba.get_mut(i..i + 4) {
                px.copy_from_slice(&[v; 4]);
            }
        });
        let os = self.oversample;
        // GlyphBitmap: bitmap_left / bitmap_top over the oversampling, and
        // top = 7 - bearingTop.
        let (left, top) = (bounds.min.x / os, 7.0 + bounds.min.y / os);
        Some((
            Glyph {
                rect: None,
                left,
                top,
                right: left + w as f32 / os,
                bottom: top + h as f32 / os,
                advance,
                offset: 1.0,
            },
            Some((w, h, rgba)),
        ))
    }
}
/// Load a Unihex archive and apply `size_overrides`.
///
/// Returns [`HexLoad::Failed`] when the archive itself cannot be read or opened
/// as a ZIP, [`HexLoad::Partial`] when the archive opens but at least one `.hex`
/// member fails to read (keeping all glyphs parsed from readable members while
/// leaving the shared cache unpopulated for a later retry), and
/// [`HexLoad::Complete`] when `hex_file` is omitted or every `.hex` member is
/// read cleanly.
fn load_hex(source: &ResourceSource, key: &UnihexKey) -> HexLoad {
    let Some(path) = key.path.as_deref() else {
        return HexLoad::Complete(HexTable::empty());
    };
    let Ok(zip) = source.read(path).and_then(Zip::from_bytes) else {
        return HexLoad::Failed;
    };
    let names: Vec<String> = zip
        .names()
        .filter(|n| n.ends_with(".hex"))
        .map(str::to_owned)
        .collect();
    let mut entries = Vec::new();
    let mut rows = Vec::new();
    let mut by_cp = crate::fast_hash::FxHashMap::default();
    let mut has_dead_rows = false;
    let mut complete = true;
    for name in names {
        let Ok(bytes) = zip.read(&name) else {
            complete = false;
            continue;
        };
        parse_hex(
            &bytes,
            &mut entries,
            &mut rows,
            &mut by_cp,
            &mut has_dead_rows,
        );
    }
    let mut wide_bounds_map: crate::fast_hash::FxHashMap<u32, (u32, u32)> =
        crate::fast_hash::FxHashMap::default();
    for &(from, to, left, right) in &key.overrides {
        for c in from..=to {
            if let Some(&idx) = by_cp.get(&c) {
                let entry = &mut entries[idx];
                if let (Ok(l), Ok(r)) = (u8::try_from(left), u8::try_from(right)) {
                    entry.left = l;
                    entry.right = r;
                    if entry.wide_bounds {
                        entry.wide_bounds = false;
                        wide_bounds_map.remove(&c);
                    }
                } else {
                    entry.left = 0;
                    entry.right = 0;
                    entry.wide_bounds = true;
                    wide_bounds_map.insert(c, (left, right));
                }
            }
        }
    }
    drop(by_cp);
    entries.sort_unstable_by_key(|e| e.codepoint);
    if has_dead_rows {
        let total: usize = entries
            .iter()
            .map(|e| 16 * usize::from(e.bytes_per_row))
            .sum();
        let mut compacted = Vec::with_capacity(total);
        for e in &mut entries {
            let len = 16 * usize::from(e.bytes_per_row);
            let start = e.offset as usize;
            e.offset = compacted.len() as u32;
            compacted.extend_from_slice(&rows[start..start + len]);
        }
        rows = compacted;
    }
    let mut wide_bounds: Vec<(u32, u32, u32)> = wide_bounds_map
        .into_iter()
        .map(|(cp, (left, right))| (cp, left, right))
        .collect();
    wide_bounds.sort_unstable_by_key(|&(cp, _, _)| cp);
    entries.shrink_to_fit();
    rows.shrink_to_fit();
    wide_bounds.shrink_to_fit();
    let table = HexTable {
        entries,
        rows,
        wide_bounds,
    };
    if complete {
        HexLoad::Complete(table)
    } else {
        HexLoad::Partial(table)
    }
}
fn ensure_unihex<'a>(
    source: &ResourceSource,
    lazy: &mut Option<Box<UnihexKey>>,
    loaded: &'a mut Option<Arc<HexTable>>,
    unihex_cache: &mut HashMap<UnihexKey, Arc<HexTable>>,
) -> &'a HexTable {
    loaded.get_or_insert_with(|| {
        let Some(key) = lazy.take() else {
            return Arc::new(HexTable::empty());
        };
        let key = *key;
        if let Some(table) = unihex_cache.get(&key) {
            return table.clone();
        }
        match load_hex(source, &key) {
            HexLoad::Complete(table) => {
                let table = Arc::new(table);
                unihex_cache.insert(key, table.clone());
                table
            }
            HexLoad::Partial(table) => Arc::new(table),
            HexLoad::Failed => Arc::new(HexTable::empty()),
        }
    })
}
/// The ids of every font definition (`font/**/*.json` of each namespace).
fn font_ids(source: &ResourceSource) -> Vec<String> {
    source
        .list_under("assets", &["font"])
        .into_iter()
        .filter_map(|file| {
            let (namespace, path) = file.strip_prefix("assets/")?.split_once("/font/")?;
            Some(format!("{namespace}:{}", path.strip_suffix(".json")?))
        })
        .collect()
}

impl Region {
    /// Place `w x h` pixels in the region (rows left to right); their atlas
    /// position, or none when it is full.
    fn place(
        &mut self,
        w: u32,
        h: u32,
        rgba: Vec<u8>,
        atlas: (u32, u32),
        region: (u32, u32),
    ) -> Option<(u32, u32)> {
        let (mut x, mut y) = self.cursor;
        if x + w + 1 > atlas.0 {
            x = 0;
            y += self.row + 1;
            self.row = 0;
        }
        if y + h > region.1 {
            self.full = true;
            return None;
        }
        let (ax, ay) = (x, region.0 + y);
        self.pending.push(Patch {
            x: ax,
            y: ay,
            width: w,
            height: h,
            rgba,
        });
        self.cursor = (x + w + 1, y);
        self.row = self.row.max(h);
        Some((ax, ay))
    }
}
/// A value no font has had yet (`Font::epoch`).
fn next_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
impl Font {
    /// Build from the atlas holding the bitmap font sheets (`sprites`, of
    /// `atlas` size) with `region` the reserved rows.
    #[horizon_mod_api_macros::replaceable]
    pub fn load(
        source: &ResourceSource,
        sprites: &HashMap<String, Rect>,
        pixels: &[u8],
        atlas: (u32, u32),
        region: (u32, u32),
        white: Rect,
    ) -> Self {
        Self::load_with(source, sprites, pixels, atlas, region, white, false)
    }
    /// Resolve provider files against the game's resources when an overlay lacks them.
    /// Definitions remain the overlay's so pack filters still hide lower definitions.
    #[allow(clippy::too_many_arguments)]
    pub fn load_over(
        source: &ResourceSource,
        defaults: &ResourceSource,
        sprites: &HashMap<String, Rect>,
        pixels: &[u8],
        atlas: (u32, u32),
        region: (u32, u32),
        white: Rect,
    ) -> Self {
        if source == defaults {
            return Self::load(source, sprites, pixels, atlas, region, white);
        }
        use horizon_assets::resource_source::Resource;
        let mut files: std::collections::BTreeMap<_, _> = defaults
            .list("assets/")
            .into_iter()
            .filter(|p| !(p.contains("/font/") && p.ends_with(".json")))
            .map(|p| (p.clone(), Resource::file(defaults.clone(), p)))
            .collect();
        files.extend(
            source
                .list("assets/")
                .into_iter()
                .map(|p| (p.clone(), Resource::file(source.clone(), p))),
        );
        Self::load(
            &ResourceSource::indexed(files),
            sprites,
            pixels,
            atlas,
            region,
            white,
        )
    }
    /// A server measures text but never draws it. Decode bitmap sheets only
    /// to obtain advances, then release their pixels; never rasterize glyphs.
    pub fn metrics(source: &ResourceSource) -> Self {
        let white = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 1,
            h: 1,
        };
        Self::load_with(source, &HashMap::new(), &[], (1, 1), (0, 0), white, true)
    }
    #[allow(clippy::too_many_arguments)]
    fn load_with(
        source: &ResourceSource,
        sprites: &HashMap<String, Rect>,
        pixels: &[u8],
        atlas: (u32, u32),
        region: (u32, u32),
        white: Rect,
        metrics_only: bool,
    ) -> Self {
        let sheets = sprites.clone();
        let font = Self {
            sheets,
            missing: Glyph {
                rect: None,
                left: 0.0,
                top: 0.0,
                right: 5.0,
                bottom: 8.0,
                advance: 6.0,
                offset: 1.0,
            },
            white,
            source: source.clone(),
            atlas,
            region,
            metrics_only,
            cache: RefCell::new(Cache {
                sets: Vec::new(),
                region: Region {
                    cursor: (0, 0),
                    row: 0,
                    pending: Vec::new(),
                    full: false,
                },
                strips: StripCache::default(),
                unihex: HashMap::new(),
                // `Font.random`: `RandomSource.create()`.
                random: crate::random_source::create(),
            }),
            generation: std::cell::Cell::new(next_generation()),
            uniform: false,
            japanese: false,
        };
        // FontManager.prepare: every font definition of the resources.
        let mut ids = font_ids(source);
        ids.sort_by_key(|id| id != DEFAULT);
        let sets = ids
            .into_iter()
            .map(|id| {
                let set = font.load_set(&id, pixels);
                (id, set)
            })
            .collect();
        font.cache.borrow_mut().sets = sets;
        font
    }
    /// `FontManager`: a font's providers with references expanded.
    fn load_set(&self, id: &str, pixels: &[u8]) -> FontSet {
        let all = providers(&self.source, id);
        let providers = all
            .iter()
            .filter_map(|p| match p["type"].as_str()? {
                "space" => Some(Provider::Space(
                    p["advances"]
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter_map(|(k, v)| {
                            let advance = v.as_f64()? as f32;
                            // A huge number casts to infinity, which would break every
                            // line that holds the glyph.
                            advance.is_finite().then_some((k.chars().next()?, advance))
                        })
                        .collect(),
                )),
                "bitmap" => Some(Provider::Bitmap(
                    Filter::of(p),
                    load_bitmap(
                        &self.source,
                        p,
                        &self.sheets,
                        pixels,
                        self.atlas,
                        self.metrics_only,
                    ),
                )),
                "unihex" => Some(Provider::Unihex(
                    Filter::of(p),
                    Some(Box::new(UnihexKey::of(p))),
                    None,
                )),
                "ttf" => Some(Provider::TrueType(
                    Filter::of(p),
                    Box::new(load_true_type(&self.source, p)?),
                )),
                _ => None,
            })
            .collect();
        FontSet {
            providers,
            glyphs: Default::default(),
            non_fishy: Default::default(),
            by_width: None,
        }
    }
    /// Forget rasterized glyphs (after changing font options).
    pub fn reset_cache(&self) {
        let mut cache = self.cache.borrow_mut();
        for (_, set) in &mut cache.sets {
            set.glyphs.clear();
            set.non_fishy.clear();
            set.by_width = None;
        }
        cache.region.cursor = (0, 0);
        cache.region.row = 0;
        cache.region.full = false;
        cache.strips.clear();
        self.generation.set(next_generation());
    }
    /// Call once per frame before any text is laid out. When the dynamic
    /// region ran out of room (vanilla's glyph textures grow instead), the
    /// glyphs are forgotten so they are rasterized again from the start; it
    /// waits for the frame boundary because quads already queued hold the old
    /// rects.
    pub fn begin_frame(&self) {
        if self.cache.borrow().region.full {
            self.reset_cache();
        }
    }
    /// Changes whenever a glyph may come out different: another font, a
    /// cache reset (glyphs move in the atlas) or other font options. Text laid
    /// out under one epoch can be kept until it changes.
    pub fn epoch(&self) -> u64 {
        self.generation.get() << 2 | u64::from(self.uniform) << 1 | u64::from(self.japanese)
    }
    /// The glyph for a character in the default font (FontSet.getGlyph).
    pub fn glyph(&self, c: char) -> Glyph {
        self.glyph_of(DEFAULT, c)
    }
    /// The glyph for a character drawn in `style`'s font (its id is
    /// already complete, see `text::font_id`).
    pub fn glyph_styled(&self, c: char, style: &Style) -> Glyph {
        self.glyph_of(style.font.as_deref().unwrap_or(DEFAULT), c)
    }
    /// The glyph for a character in the font `id` (`alt` is `minecraft:alt`).
    pub fn glyph_in(&self, id: &str, c: char) -> Glyph {
        if id.contains(':') {
            self.glyph_of(id, c)
        } else {
            self.glyph_of(&format!("minecraft:{id}"), c)
        }
    }
    /// The first active provider's glyph, else the missing glyph.
    fn glyph_of(&self, id: &str, c: char) -> Glyph {
        let mut cache = self.cache.borrow_mut();
        let Cache {
            sets,
            region,
            strips,
            unihex,
            ..
        } = &mut *cache;
        // FontManager.getFontSetRaw: a font that doesn't exist draws every
        // character as the missing glyph.
        let Some((_, set)) = sets.iter_mut().find(|(k, _)| k == id) else {
            return self.missing;
        };
        if let Some(g) = set.glyphs.get(&c) {
            return *g;
        }
        let was_full = std::mem::replace(&mut region.full, false);
        let found = self.resolve(set, region, strips, unihex, c, false);
        // No room: the missing glyph for now, not remembered, so the
        // character is tried again once `begin_frame` cleared the region.
        let overflowed = std::mem::replace(&mut region.full, was_full);
        region.full |= overflowed;
        if overflowed {
            return self.missing;
        }
        let glyph = found.unwrap_or(self.missing);
        set.glyphs.insert(c, glyph);
        glyph
    }
    /// `FontSet.getRandomGlyph`'s `SelectedGlyphs.nonFishy`: the glyph of the first provider whose
    /// advance is not fishy, else the missing glyph. A character whose first glyph is fine has
    /// the same one for both.
    fn glyph_non_fishy(&self, id: &str, c: char) -> Glyph {
        let any = self.glyph_of(id, c);
        if !fishy_advance(any.advance) {
            return any;
        }
        let mut cache = self.cache.borrow_mut();
        let Cache {
            sets,
            region,
            strips,
            unihex,
            ..
        } = &mut *cache;
        let Some((_, set)) = sets.iter_mut().find(|(k, _)| k == id) else {
            return self.missing;
        };
        if let Some(g) = set.non_fishy.get(&c) {
            return *g;
        }
        let was_full = std::mem::replace(&mut region.full, false);
        let found = self.resolve(set, region, strips, unihex, c, true);
        let overflowed = std::mem::replace(&mut region.full, was_full);
        region.full |= overflowed;
        if overflowed {
            return self.missing;
        }
        let glyph = found.unwrap_or(self.missing);
        set.non_fishy.insert(c, glyph);
        glyph
    }
    fn resolve(
        &self,
        set: &mut FontSet,
        region: &mut Region,
        strips: &mut StripCache,
        unihex_cache: &mut HashMap<UnihexKey, Arc<HexTable>>,
        c: char,
        non_fishy: bool,
    ) -> Option<Glyph> {
        let (uniform, japanese) = (self.uniform, self.japanese);
        for provider in &mut set.providers {
            match provider {
                Provider::Space(advances) => {
                    if let Some(a) = advances.get(&c)
                        && !(non_fishy && fishy_advance(*a))
                    {
                        return Some(Glyph {
                            rect: None,
                            left: 0.0,
                            top: 0.0,
                            right: 0.0,
                            bottom: 0.0,
                            advance: *a,
                            offset: 1.0,
                        });
                    }
                }
                Provider::Bitmap(filter, glyphs) if filter.passes(uniform, japanese) => {
                    match glyphs.get(&c) {
                        Some(BitmapGlyph::Atlas(g)) if !(non_fishy && fishy_advance(g.advance)) => {
                            return Some(*g);
                        }
                        Some(BitmapGlyph::Sheet(cell))
                            if !(non_fishy && fishy_advance(cell.glyph.advance)) =>
                        {
                            let Some(rgba) = cell.pixels(strips) else {
                                continue;
                            };
                            let (x, y) =
                                region.place(cell.w, cell.h, rgba, self.atlas, self.region)?;
                            return Some(Glyph {
                                rect: Some(glyph_rect(x, y, cell.w, cell.h, self.atlas)),
                                ..cell.glyph
                            });
                        }
                        _ => {}
                    }
                }
                Provider::Unihex(filter, lazy, loaded) if filter.passes(uniform, japanese) => {
                    let table = ensure_unihex(&self.source, lazy, loaded, unihex_cache);
                    if let Some(h) = table.get(u32::from(c))
                        && !(non_fishy && fishy_advance(hex_advance(&h)))
                    {
                        return self.rasterize_hex(region, h);
                    }
                }
                Provider::TrueType(filter, ttf) if filter.passes(uniform, japanese) => {
                    if let Some(id) = ttf.glyph_id(c)
                        && !(non_fishy && fishy_advance(ttf.advance(id)))
                    {
                        if self.metrics_only {
                            return Some(Glyph {
                                rect: None,
                                advance: ttf.advance(id),
                                offset: 1.0,
                                ..self.missing
                            });
                        }
                        let Some((glyph, bitmap)) = ttf.glyph(id, (self.atlas.0, self.region.1))
                        else {
                            continue;
                        };
                        return match bitmap {
                            Some((w, h, rgba)) => {
                                let (x, y) = region.place(w, h, rgba, self.atlas, self.region)?;
                                Some(Glyph {
                                    rect: Some(glyph_rect(x, y, w, h, self.atlas)),
                                    ..glyph
                                })
                            }
                            None => Some(glyph),
                        };
                    }
                }
                _ => {}
            }
        }
        None
    }
    fn rasterize_hex(&self, region: &mut Region, h: HexGlyph) -> Option<Glyph> {
        let width = h.right.saturating_sub(h.left) + 1;
        if self.metrics_only {
            return Some(Glyph {
                rect: None,
                left: 0.0,
                top: 0.0,
                right: width as f32 / 2.0,
                bottom: 8.0,
                advance: hex_advance(&h),
                offset: 0.5,
            });
        }
        let mut rgba = Vec::with_capacity((width * 16 * 4) as usize);
        for row in h.rows {
            for bit in h.left..=h.right {
                let set = bit < 32 && (row >> (31 - bit)) & 1 != 0;
                rgba.extend(if set { [255; 4] } else { [0; 4] });
            }
        }
        let (x, y) = region.place(width, 16, rgba, self.atlas, self.region)?;
        Some(Glyph {
            rect: Some(glyph_rect(x, y, width, 16, self.atlas)),
            left: 0.0,
            top: 0.0,
            right: width as f32 / 2.0,
            bottom: 8.0,
            advance: hex_advance(&h),
            offset: 0.5,
        })
    }
    /// Starts the random `random_glyph` draws on over from `seed`.
    pub fn reseed(&self, seed: i64) {
        self.cache.borrow_mut().random = horizon_core::random::LegacyRandom::new(seed);
    }
    /// `FontSet.getRandomGlyph`: what an obfuscated character draws, a random
    /// glyph of the font as wide as `advance` (rounded up), else the missing
    /// glyph.
    ///
    /// Vanilla picks among every glyph of the font set, Unifont's included.
    /// Those are rasterized into a fixed atlas region on demand, which a
    /// line of obfuscated text would fill in seconds, so the candidates are
    /// the glyphs already in the atlas (the bitmap sheets there and spaces),
    /// or for a font without any (Force Unicode Font, a TrueType font) its
    /// glyphs below U+0250.
    pub fn random_glyph(&self, style: &Style, advance: f32) -> Glyph {
        use horizon_core::random::RandomSource;
        let id = style.font.as_deref().unwrap_or(DEFAULT);
        let width = advance.ceil() as i32;
        let pick = {
            let mut cache = self.cache.borrow_mut();
            let Cache {
                sets,
                unihex,
                random,
                ..
            } = &mut *cache;
            let Some((_, set)) = sets.iter_mut().find(|(k, _)| k == id) else {
                return self.missing;
            };
            if set.by_width.is_none() {
                set.by_width = Some(self.glyphs_by_width(set, unihex));
            }
            set.by_width
                .as_ref()
                .and_then(|m| m.get(&width))
                .and_then(|chars| {
                    let i = random.next_int_bound(chars.len() as i32).ok()?;
                    chars.get(i as usize).copied()
                })
        };
        pick.map_or(self.missing, |c| self.glyph_non_fishy(id, c))
    }
    fn glyphs_by_width(
        &self,
        set: &mut FontSet,
        unihex_cache: &mut HashMap<UnihexKey, Arc<HexTable>>,
    ) -> HashMap<i32, Vec<char>> {
        let (uniform, japanese) = (self.uniform, self.japanese);
        let mut chars = std::collections::BTreeSet::new();
        let mut in_atlas = false;
        for provider in &set.providers {
            match provider {
                Provider::Space(advances) => chars.extend(advances.keys().copied()),
                Provider::Bitmap(filter, glyphs) if filter.passes(uniform, japanese) => {
                    for (c, g) in glyphs {
                        if matches!(g, BitmapGlyph::Atlas(_)) {
                            in_atlas = true;
                            chars.insert(*c);
                        }
                    }
                }
                _ => {}
            }
        }
        if !in_atlas {
            // Nothing in the atlas: the glyphs below U+0250, rasterized as used.
            chars.extend((0..0x250u32).filter_map(char::from_u32));
        }
        let mut out: HashMap<i32, Vec<char>> = HashMap::new();
        for c in chars {
            if let Some(advance) = self.provider_advance(set, unihex_cache, c) {
                out.entry(advance.ceil() as i32).or_default().push(c);
            }
        }
        out
    }
    /// A character's advance in `set` (its first provider that has it, fishy or not, as the
    /// by-width table of `FontSet.reload` takes it), without rasterizing it.
    fn provider_advance(
        &self,
        set: &mut FontSet,
        unihex_cache: &mut HashMap<UnihexKey, Arc<HexTable>>,
        c: char,
    ) -> Option<f32> {
        let (uniform, japanese) = (self.uniform, self.japanese);
        for provider in &mut set.providers {
            match provider {
                Provider::Space(advances) => {
                    if let Some(a) = advances.get(&c) {
                        return Some(*a);
                    }
                }
                Provider::Bitmap(filter, glyphs) if filter.passes(uniform, japanese) => {
                    match glyphs.get(&c) {
                        Some(BitmapGlyph::Atlas(g)) => {
                            return Some(g.advance);
                        }
                        Some(BitmapGlyph::Sheet(cell)) => {
                            return Some(cell.glyph.advance);
                        }
                        _ => {}
                    }
                }
                Provider::Unihex(filter, lazy, loaded) if filter.passes(uniform, japanese) => {
                    let table = ensure_unihex(&self.source, lazy, loaded, unihex_cache);
                    if let Some(advance) = table.advance(u32::from(c)) {
                        return Some(advance);
                    }
                }
                Provider::TrueType(filter, ttf) if filter.passes(uniform, japanese) => {
                    if let Some(id) = ttf.glyph_id(c)
                        && (self.metrics_only || ttf.fits_atlas(id, (self.atlas.0, self.region.1)))
                    {
                        return Some(ttf.advance(id));
                    }
                }
                _ => {}
            }
        }
        None
    }
    /// Atlas pixels rasterized since the last call.
    #[horizon_mod_api_macros::replaceable]
    pub fn take_patches(&self) -> Vec<Patch> {
        std::mem::take(&mut self.cache.borrow_mut().region.pending)
    }
    pub fn advance(&self, c: char, style: &Style) -> f32 {
        if c == '\u{fffc}' && style.inline_object.is_some() {
            return 8.0;
        }
        let g = self.glyph_styled(c, style);
        g.advance + if style.is_bold() { g.offset } else { 0.0 }
    }
    /// Font.width of plain text (legacy codes are formatting, not glyphs).
    pub fn width(&self, s: &str) -> f32 {
        if s.contains('§') {
            return self.width_text(&Text::legacy(s));
        }
        crate::bidi::visual_str(s)
            .chars()
            .map(|c| self.glyph(c).advance)
            .sum::<f32>()
            .ceil()
    }
    /// Font.width of a component: `§` codes in the spans are formatting,
    /// not glyphs. Right-to-left text is measured as it is drawn, shaped.
    pub fn width_text(&self, t: &Text) -> f32 {
        let t = t.resolved();
        crate::bidi::visual(&t)
            .chars()
            .map(|(c, s)| self.advance(c, s))
            .sum::<f32>()
            .ceil()
    }
    /// `Font.width(FormattedCharSequence)` of a sequence a component built
    /// with `FormattedCharSequence.forward`: every character is a glyph, a
    /// `§` included (`StringDecomposer.iterate`, not `iterateFormatted`).
    pub fn width_drawn(&self, t: &Text) -> f32 {
        crate::bidi::visual(t)
            .chars()
            .map(|(c, s)| self.advance(c, s))
            .sum::<f32>()
            .ceil()
    }
    /// `StringSplitter.splitLines`: each line and whether it continues a
    /// wrapped one. A line breaks at a `\n`, or once a character overflows
    /// `max_width` (never the first visible one) at the last space so far, or
    /// before that character; the `\n` or space at the break is dropped.
    pub fn split_lines(&self, t: &Text, max_width: f32) -> Vec<(Text, bool)> {
        let max_width = max_width.max(1.0);
        let chars: Vec<(char, &Style)> = t.chars().collect();
        let n = chars.len();
        let mut out = Vec::new();
        let (mut start, mut wrapped, mut force_new_line) = (0, false, false);
        loop {
            // LineBreakFinder over what is left.
            let (mut width, mut had_visible, mut last_space) = (0.0f32, false, None);
            let mut split = None;
            for (i, &(c, style)) in chars.iter().enumerate().skip(start) {
                if c == '\n' {
                    split = Some(i);
                    break;
                }
                if c == ' ' {
                    last_space = Some(i);
                }
                let advance = self.advance(c, style);
                width += advance;
                if !had_visible || width <= max_width {
                    had_visible |= advance != 0.0;
                } else {
                    split = Some(last_space.unwrap_or(i));
                    break;
                }
            }
            let Some(at) = split else { break };
            let new_line = chars[at].0 == '\n';
            let skip = usize::from(new_line || chars[at].0 == ' ');
            force_new_line = new_line;
            out.push((t.slice(start, at), wrapped));
            wrapped = !new_line;
            start = at + skip;
        }
        if start < n {
            out.push((t.slice(start, n), wrapped));
        } else if force_new_line {
            out.push((Text::empty(), false));
        }
        out
    }
    /// `Font.split`: `StringSplitter.splitLines(text, width, Style.EMPTY)` over
    /// the text's parts with their `§` codes in. A code styles what follows
    /// (`§r` back to no style while measuring), and the part a line breaks in
    /// goes on in the style at the break. Lines keep their codes for drawing.
    pub fn split_formatted(&self, t: &Text, max_width: f32) -> Vec<Text> {
        let max_width = max_width.max(1.0);
        let mut parts: Vec<(Vec<char>, Span)> = t
            .spans
            .iter()
            .filter(|s| !s.text.is_empty())
            .map(|s| (s.text.chars().collect(), s.with_text(String::new())))
            .collect();
        let mut flat: Vec<char> = parts.iter().flat_map(|p| p.0.iter().copied()).collect();
        let mut out = Vec::new();
        let mut force_new_line = false;
        // One LineBreakFinder per line, over the parts left.
        while let Some((at, split_style)) = self.line_break(&parts, max_width) {
            let tail = flat[at];
            let new_line = tail == '\n';
            let skip = usize::from(new_line || tail == ' ');
            force_new_line = new_line;
            out.push(split_at(&mut parts, at, skip, &split_style));
            flat.drain(..at + skip);
        }
        if !parts.is_empty() {
            out.push(Text::from_spans(
                parts
                    .into_iter()
                    .map(|(text, span)| span.with_text(text.into_iter().collect::<String>()))
                    .collect(),
            ));
        } else if force_new_line {
            out.push(Text::empty());
        }
        out
    }
    /// `StringSplitter.LineBreakFinder` fed by `StringDecomposer.iterateFormatted`:
    /// where the parts break (a character index) and the style there.
    fn line_break(&self, parts: &[(Vec<char>, Span)], max_width: f32) -> Option<(usize, Style)> {
        let (mut width, mut had_visible, mut last_space) = (0.0f32, false, None);
        let mut offset = 0;
        for (contents, part_style) in parts {
            let mut style = part_style.style.clone();
            let mut i = 0;
            while i < contents.len() {
                let c = contents[i];
                if c == '§' {
                    // A `§` that ends its part ends only that part.
                    let Some(&code) = contents.get(i + 1) else {
                        break;
                    };
                    style = legacy_format(&style, code, &Style::default());
                    i += 2;
                    continue;
                }
                let at = offset + i;
                if c == '\n' {
                    return Some((at, style));
                }
                if c == ' ' {
                    last_space = Some((at, style.clone()));
                }
                let advance = self.advance(c, &style);
                width += advance;
                if had_visible && width > max_width {
                    return Some(last_space.unwrap_or((at, style)));
                }
                had_visible |= advance != 0.0;
                i += 1;
            }
            offset += contents.len();
        }
        None
    }
    /// `ComponentRenderUtils.wrapComponents`: the split lines, wrapped ones
    /// indented by an unstyled space; one empty line for empty text.
    pub fn wrap_components(&self, t: &Text, max_width: i32) -> Vec<Text> {
        let lines: Vec<Text> = self
            .split_lines(t, max_width as f32)
            .into_iter()
            .map(|(line, wrapped)| {
                if wrapped {
                    let mut spans = Text::literal(" ").into_spans();
                    spans.extend(line.into_spans());
                    Text::from_spans(spans)
                } else {
                    line
                }
            })
            .collect();
        if lines.is_empty() {
            vec![Text::empty()]
        } else {
            lines
        }
    }
    /// StringSplitter.splitLines: wrap at the last space that fits (or
    /// mid-word when none does); `\n` always breaks. Styles are kept.
    pub fn split(&self, t: &Text, max_width: f32) -> Vec<Text> {
        let chars: Vec<(char, Style)> = t.chars().map(|(c, s)| (c, s.clone())).collect();
        let mut lines = Vec::new();
        let mut start = 0;
        while start < chars.len() {
            let mut width = 0.0;
            let mut end = chars.len();
            let mut next = chars.len();
            let mut last_space = None;
            for (i, (c, style)) in chars.iter().enumerate().skip(start) {
                if *c == '\n' {
                    end = i;
                    next = i + 1;
                    break;
                }
                if *c == ' ' {
                    last_space = Some(i);
                }
                width += self.advance(*c, style);
                if width > max_width {
                    match last_space {
                        Some(s) if s > start => {
                            end = s;
                            next = s + 1;
                        }
                        _ => {
                            end = i.max(start + 1);
                            next = end;
                        }
                    }
                    break;
                }
            }
            lines.push(t.slice(start, end));
            start = next;
            if start == chars.len() && chars.last().is_some_and(|c| c.0 == '\n') {
                lines.push(Text::empty());
            }
        }
        if lines.is_empty() {
            lines.push(Text::empty());
        }
        lines
    }
    /// Characters of `s` that fit in `width` (Font.plainSubstrByWidth).
    pub fn fit(&self, s: &str, width: f32) -> usize {
        let mut w = 0.0;
        for (i, c) in s.chars().enumerate() {
            w += self.glyph(c).advance;
            if w > width {
                return i;
            }
        }
        s.chars().count()
    }
    /// Characters of `s` that fit in `width`, measured from the end.
    pub fn fit_reverse(&self, s: &str, width: f32) -> usize {
        let mut w = 0.0;
        for (i, c) in s.chars().rev().enumerate() {
            w += self.glyph(c).advance;
            if w > width {
                return i;
            }
        }
        s.chars().count()
    }
    pub fn white(&self) -> Rect {
        self.white
    }
}

/// `StringSplitter.FlatComponents.splitAt`: the parts before `at` as a line,
/// `skip` characters dropped, and the part it ends in restyled as `style`.
fn split_at(parts: &mut Vec<(Vec<char>, Span)>, at: usize, skip: usize, style: &Style) -> Text {
    let mut line = Vec::new();
    let mut push = |text: &[char], span: &Span| {
        line.push(span.with_text(text.iter().collect::<String>()));
    };
    // Each part done is removed, so the one at hand is always the first.
    let (mut position, mut in_skip) = (at, false);
    while let Some((text, part_style)) = parts.first() {
        let size = text.len();
        if !in_skip {
            if position > size {
                push(text, part_style);
                parts.remove(0);
                position -= size;
                continue;
            }
            if position > 0 {
                push(&text[..position], part_style);
            }
            position += skip;
            in_skip = true;
        }
        if position <= size {
            if position == size {
                parts.remove(0);
            } else {
                let mut span = part_style.clone();
                span.style = style.clone();
                parts[0] = (text[position..].to_vec(), span);
            }
            break;
        }
        parts.remove(0);
        position -= size;
    }
    Text::from_spans(line)
}

#[cfg(test)]
mod tests;
