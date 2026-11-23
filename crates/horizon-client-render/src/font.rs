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
    collections::HashMap,
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
    ids: HashMap<char, ab_glyph::GlyphId>,
    /// FT_Set_Pixel_Sizes: `size * oversample` pixels to the em.
    scale: ab_glyph::PxScale,
    oversample: f32,
    /// FT_Set_Transform's delta, in oversampled pixels (y down).
    shift: (f32, f32),
}
enum Provider {
    Space(HashMap<char, f32>),
    Bitmap(Filter, HashMap<char, BitmapGlyph>),
    /// The definition, and its glyphs once read.
    Unihex(Filter, serde_json::Value, Option<HashMap<u32, HexGlyph>>),
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
fn parse_hex_digit(b: u8) -> Option<u32> {
    (b as char).to_digit(16)
}
/// UnihexProvider.readFromStream over one `.hex` file.
fn parse_hex(text: &[u8], out: &mut HashMap<u32, HexGlyph>) {
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
        let digits_per_row = match data.len() {
            32 => 2,
            64 => 4,
            96 => 6,
            128 => 8,
            _ => continue,
        };
        let bit_width = digits_per_row as u32 * 4;
        let mut rows = [0u32; 16];
        let mut mask = 0u32;
        let mut ok = true;
        for (r, row) in rows.iter_mut().enumerate() {
            let chunk = &data[r * digits_per_row..(r + 1) * digits_per_row];
            match chunk
                .iter()
                .try_fold(0u32, |a, b| parse_hex_digit(*b).map(|d| a << 4 | d))
            {
                Some(v) => {
                    *row = v << (32 - bit_width);
                    mask |= *row;
                }
                None => ok = false,
            }
        }
        if !ok {
            continue;
        }
        // LineData.calculateWidth.
        let (left, right) = if mask == 0 {
            (0, bit_width)
        } else {
            (mask.leading_zeros(), 31 - mask.trailing_zeros())
        };
        out.insert(codepoint, HexGlyph { rows, left, right });
    }
}
fn read_json(source: &ResourceSource, path: &str) -> Option<serde_json::Value> {
    serde_json::from_slice(&source.read(path).ok()?).ok()
}
fn resource_path(id: &str, folder: &str, ext: &str) -> String {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    format!("assets/{ns}/{folder}/{}{ext}", path.trim_end_matches(ext))
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
/// Providers of a font definition with references expanded, in order, with
/// their `filter` objects.
fn providers(source: &ResourceSource, id: &str, depth: u32, out: &mut Vec<serde_json::Value>) {
    if depth > 8 {
        return;
    }
    let path = resource_path(id, "font", ".json");
    let Some(def) = read_json(source, &path) else {
        return;
    };
    if let Some(error) = font_file_error(&def["providers"]) {
        log::error!("font {path} skipped: {error}");
        return;
    }
    for p in def["providers"].as_array().into_iter().flatten() {
        if p["type"] == "reference" {
            let mut nested = Vec::new();
            providers(
                source,
                p["id"].as_str().unwrap_or(""),
                depth + 1,
                &mut nested,
            );
            for mut n in nested {
                // A reference's filter applies to everything it includes.
                if let Some(f) = p["filter"].as_object() {
                    let merged = n["filter"]
                        .as_object()
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .chain(f.clone())
                        .collect();
                    n["filter"] = serde_json::Value::Object(merged);
                }
                out.push(n);
            }
        } else {
            out.push(p.clone());
        }
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
    let skip: Vec<char> = match &p["skip"] {
        serde_json::Value::String(s) => s.chars().collect(),
        serde_json::Value::Array(a) => a
            .iter()
            .filter_map(|v| v.as_str())
            .flat_map(str::chars)
            .collect(),
        _ => Vec::new(),
    };
    let ids = font
        .codepoint_ids()
        .filter(|(id, c)| id.0 != 0 && !skip.contains(c))
        .map(|(id, c)| (c, id))
        .collect();
    let pixels_per_em = (size * oversample).round();
    // ab_glyph scales by the font's height; FreeType by its em.
    let units = font.units_per_em().unwrap_or(1000.0);
    let scale = ab_glyph::PxScale::from(pixels_per_em * font.height_unscaled() / units);
    Some(TrueType {
        font,
        ids,
        scale,
        oversample,
        shift: (shift.0 * oversample, shift.1 * oversample),
    })
}
impl TrueType {
    /// The advance in GUI pixels: hinted FreeType advances are whole
    /// oversampled pixels.
    fn advance(&self, id: ab_glyph::GlyphId) -> f32 {
        self.font.as_scaled(self.scale).h_advance(id).round() / self.oversample
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
        let glyph =
            id.with_scale_and_position(self.scale, ab_glyph::point(self.shift.0, self.shift.1));
        let empty = Glyph {
            rect: None,
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            advance,
            offset: 1.0,
        };
        let Some(outline) = self.font.outline_glyph(glyph) else {
            return Some((empty, None));
        };
        let bounds = outline.px_bounds();
        let (w, h) = (bounds.width() as u32, bounds.height() as u32);
        if w == 0 || h == 0 {
            return Some((empty, None));
        }
        if w > max.0 || h > max.1 {
            return None;
        }
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
fn load_hex(source: &ResourceSource, p: &serde_json::Value) -> HashMap<u32, HexGlyph> {
    let mut glyphs = HashMap::new();
    let Some(file) = p["hex_file"].as_str() else {
        return glyphs;
    };
    let path = resource_path(file, "", "").replace("//", "/");
    let Ok(zip) = source.read(&path).and_then(Zip::from_bytes) else {
        return glyphs;
    };
    let names: Vec<String> = zip
        .names()
        .filter(|n| n.ends_with(".hex"))
        .map(str::to_owned)
        .collect();
    for name in names {
        if let Ok(bytes) = zip.read(&name) {
            parse_hex(&bytes, &mut glyphs);
        }
    }
    for o in p["size_overrides"].as_array().into_iter().flatten() {
        let bound = |k: &str| {
            o[k].as_str()
                .and_then(|s| s.chars().next())
                .map_or(0, u32::from)
        };
        let (left, right) = (
            o["left"].as_u64().unwrap_or(0) as u32,
            o["right"].as_u64().unwrap_or(15) as u32,
        );
        for c in bound("from")..=bound("to") {
            if let Some(g) = glyphs.get_mut(&c) {
                g.left = left;
                g.right = right;
            }
        }
    }
    glyphs
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
        let mut all = Vec::new();
        providers(&self.source, id, 0, &mut all);
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
                "unihex" => Some(Provider::Unihex(Filter::of(p), p.clone(), None)),
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
        let found = self.resolve(set, region, strips, c, false);
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
            ..
        } = &mut *cache;
        let Some((_, set)) = sets.iter_mut().find(|(k, _)| k == id) else {
            return self.missing;
        };
        if let Some(g) = set.non_fishy.get(&c) {
            return *g;
        }
        let was_full = std::mem::replace(&mut region.full, false);
        let found = self.resolve(set, region, strips, c, true);
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
                Provider::Unihex(filter, def, glyphs) if filter.passes(uniform, japanese) => {
                    let glyphs = glyphs.get_or_insert_with(|| load_hex(&self.source, def));
                    if let Some(h) = glyphs.get(&u32::from(c)).copied()
                        && !(non_fishy && fishy_advance(hex_advance(&h)))
                    {
                        return self.rasterize_hex(region, h);
                    }
                }
                Provider::TrueType(filter, ttf) if filter.passes(uniform, japanese) => {
                    if let Some(id) = ttf.ids.get(&c)
                        && !(non_fishy && fishy_advance(ttf.advance(*id)))
                    {
                        if self.metrics_only {
                            return Some(Glyph {
                                rect: None,
                                advance: ttf.advance(*id),
                                offset: 1.0,
                                ..self.missing
                            });
                        }
                        let (glyph, bitmap) = ttf.glyph(*id, (self.atlas.0, self.region.1))?;
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
            let Cache { sets, random, .. } = &mut *cache;
            let Some((_, set)) = sets.iter_mut().find(|(k, _)| k == id) else {
                return self.missing;
            };
            if set.by_width.is_none() {
                set.by_width = Some(self.glyphs_by_width(set));
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
    fn glyphs_by_width(&self, set: &mut FontSet) -> HashMap<i32, Vec<char>> {
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
            if let Some(advance) = self.provider_advance(set, c) {
                out.entry(advance.ceil() as i32).or_default().push(c);
            }
        }
        out
    }
    /// A character's advance in `set` (its first provider that has it, fishy or not, as the
    /// by-width table of `FontSet.reload` takes it), without rasterizing it.
    fn provider_advance(&self, set: &mut FontSet, c: char) -> Option<f32> {
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
                Provider::Unihex(filter, def, glyphs) if filter.passes(uniform, japanese) => {
                    let glyphs = glyphs.get_or_insert_with(|| load_hex(&self.source, def));
                    if let Some(h) = glyphs.get(&u32::from(c)) {
                        return Some(hex_advance(h));
                    }
                }
                Provider::TrueType(filter, ttf) if filter.passes(uniform, japanese) => {
                    if let Some(id) = ttf.ids.get(&c) {
                        return Some(ttf.advance(*id));
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
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    thread_local! {
        static STRIP_DECODED_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
    pub(super) fn record_strip_decoded(bytes: usize) {
        STRIP_DECODED_BYTES.with(|total| total.set(total.get().saturating_add(bytes)));
    }
    fn measure_decoded_bytes<R>(f: impl FnOnce() -> R) -> (R, usize) {
        STRIP_DECODED_BYTES.with(|total| total.set(0));
        let out = f();
        let decoded = STRIP_DECODED_BYTES.with(std::cell::Cell::get);
        (out, decoded)
    }
    #[test]
    fn unihex_rows_and_widths() {
        let mut out = HashMap::new();
        // 'A' from unifont (8 wide) and a 16-wide glyph with an empty left column.
        let wide = format!("4E00:{}7FFE{}\n", "0000".repeat(4), "0000".repeat(11));
        parse_hex(b"0041:0000000018242442427E424242420000\n", &mut out);
        parse_hex(wide.as_bytes(), &mut out);
        let a = out[&0x41];
        assert_eq!((a.left, a.right), (1, 6));
        assert_eq!(a.rows[4], 0x18 << 24);
        let yi = out[&0x4E00];
        assert_eq!((yi.left, yi.right), (1, 14));
        // Unparseable lines are skipped.
        parse_hex(b"zz:00\n", &mut out);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn glyph_uvs_have_the_native_texel_inset() {
        for (x, y, w, h) in [(120, 48, 8, 8), (0, 0, 1, 16), (4000, 2000, 32, 16)] {
            let r = glyph_rect(x, y, w, h, (4096, 4096));
            assert_eq!((r.w, r.h), (w, h));
            assert!(r.u0 > x as f32 / 4096.0 && r.v0 > y as f32 / 4096.0);
            assert!(r.u1 < (x + w) as f32 / 4096.0 && r.v1 < (y + h) as f32 / 4096.0);
            assert_eq!(r.u0, (x as f32 + 0.01) / 4096.0);
            assert_eq!(r.v1, ((y + h) as f32 - 0.01) / 4096.0);
        }
    }

    /// A resource root with these font definitions and files.
    fn root(name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("horizon-font-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (path, bytes) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        root
    }
    fn white() -> Rect {
        Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 0.0,
            v1: 0.0,
            w: 1,
            h: 1,
        }
    }
    fn load(root: &Path) -> Font {
        Font::load(
            &ResourceSource::directory(root.to_owned()),
            &HashMap::new(),
            &[],
            (4096, 1024),
            (512, 512),
            white(),
        )
    }
    fn png(width: u32, height: u32, opaque: impl Fn(u32, u32) -> bool) -> Vec<u8> {
        let mut pixels = Vec::new();
        for y in 0..height {
            for x in 0..width {
                pixels.extend(if opaque(x, y) { [255; 4] } else { [0; 4] });
            }
        }
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        out
    }

    #[test]
    fn archive_fonts_keep_lazy_unihex_on_the_open_reload_snapshot() {
        use std::io::{Cursor, Write};
        let root =
            std::env::temp_dir().join(format!("horizon-font-archive-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let write = |path: &Path, bits: &str| {
            let mut unifont = zip::ZipWriter::new(Cursor::new(Vec::new()));
            unifont
                .start_file("glyphs.hex", zip::write::SimpleFileOptions::default())
                .unwrap();
            unifont
                .write_all(format!("0041:{}\n", bits.repeat(16)).as_bytes())
                .unwrap();
            let unifont = unifont.finish().unwrap().into_inner();
            let image = png(4, 8, |_, _| true);
            let mut pack = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
            for (name, bytes) in [
                ("assets/minecraft/font/default.json", br#"{"providers":[{"type":"reference","id":"demo:nested/bitmap"}]}"#.as_slice()),
                ("assets/demo/font/nested/bitmap.json", br#"{"providers":[{"type":"bitmap","file":"demo:font/sheet.png","ascent":7,"chars":["a"]}]}"#),
                ("assets/demo/font/hex.json", br#"{"providers":[{"type":"unihex","hex_file":"demo:font/unifont.zip"}]}"#),
                ("assets/demo/font/type.json", br#"{"providers":[{"type":"ttf","file":"demo:mono.ttf","size":11}]}"#),
                ("assets/demo/font/mono.ttf", horizon_theme::fonts::GEIST_MONO),
                ("assets/demo/font/unifont.zip", unifont.as_slice()),
                ("assets/demo/textures/font/sheet.png", image.as_slice()),
            ] {
                pack.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
                pack.write_all(bytes).unwrap();
            }
            pack.finish().unwrap();
        };
        let jar = root.join("fonts.jar");
        write(&jar, "FF");
        let source = ResourceSource::archive(&jar).unwrap();
        let font = Font::load(
            &source,
            &HashMap::new(),
            &[],
            (4096, 1024),
            (512, 512),
            white(),
        );
        let metrics = Font::metrics(&source);
        let replacement = root.join("replacement.jar");
        write(&replacement, "F0");
        std::fs::rename(&jar, root.join("old.jar")).unwrap();
        std::fs::rename(replacement, &jar).unwrap();
        assert_eq!(
            font.glyph('a').advance,
            5.0,
            "bitmap font reference in another namespace"
        );
        assert!(
            font.glyph_in("demo:type", 'A').rect.is_some(),
            "archive TrueType data"
        );
        assert_eq!(
            font.glyph_in("demo:hex", 'A').advance,
            5.0,
            "first lazy read uses the old nested archive"
        );
        assert_eq!(metrics.glyph_in("demo:hex", 'A').advance, 5.0);
        assert!(metrics.glyph_in("demo:hex", 'A').rect.is_none());
        let reloaded_source = ResourceSource::archive(&jar).unwrap();
        let reloaded = Font::metrics(&reloaded_source);
        assert_eq!(reloaded.glyph_in("demo:hex", 'A').advance, 3.0);
        assert!(!root.join("assets").exists());
        drop((font, metrics, reloaded, source, reloaded_source));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn server_metrics_preserve_bitmap_true_type_and_unihex_advances() {
        let sheet = png(16, 8, |x, _| x < 2 || (8..14).contains(&x));
        let def = br#"{"providers":[{"type":"space","advances":{" ":4}},{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ab"]}]}"#;
        let ttf = br#"{"providers":[{"type":"ttf","file":"minecraft:mono.ttf","size":11,"oversample":2}]}"#;
        let root = root(
            "metrics",
            &[
                ("assets/minecraft/font/default.json", def),
                ("assets/minecraft/font/alt.json", ttf),
                (
                    "assets/minecraft/font/mono.ttf",
                    horizon_theme::fonts::GEIST_MONO,
                ),
                ("assets/minecraft/textures/font/a.png", &sheet),
            ],
        );
        let rendered = load(&root);
        let metrics = Font::metrics(&ResourceSource::directory(root.clone()));
        assert_eq!(metrics.width("a b"), 14.0);
        assert_eq!(metrics.width("§lab"), 12.0);
        let text = Text::literal("ab ab");
        assert_eq!(
            metrics.split_formatted(&text, 10.0),
            rendered.split_formatted(&text, 10.0)
        );
        let alt = Style {
            font: Some("minecraft:alt".into()),
            bold: Some(true),
            ..Style::default()
        };
        for c in ['A', 'i', ' ', '\u{fffd}'] {
            assert_eq!(metrics.advance(c, &alt), rendered.advance(c, &alt));
            assert!(metrics.glyph_styled(c, &alt).rect.is_none());
        }
        let mut hex = HashMap::new();
        parse_hex(b"0041:0000000018242442427E424242420000\n", &mut hex);
        let h = hex[&0x41];
        let glyph = metrics
            .rasterize_hex(&mut metrics.cache.borrow_mut().region, h)
            .unwrap();
        assert_eq!(glyph.advance, 4.0);
        assert_eq!(glyph.offset, 0.5);
        assert!(glyph.rect.is_none());
        assert!(metrics.take_patches().is_empty());
        assert!(!rendered.take_patches().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn true_type_glyphs_are_rasterized_at_the_provider_size() {
        let def = br#"{"providers":[{"type":"ttf","file":"minecraft:mono.ttf","size":11,"oversample":2,"shift":[0.5,1],"skip":["B"]}]}"#;
        let root = root(
            "ttf",
            &[
                ("assets/minecraft/font/default.json", def),
                (
                    "assets/minecraft/font/mono.ttf",
                    horizon_theme::fonts::GEIST_MONO,
                ),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        let a = font.glyph('A');
        let rect = a.rect.expect("A is drawn");
        // Oversampled 2x: the bitmap is twice the glyph's GUI size.
        assert_eq!(rect.w as f32 / 2.0, a.right - a.left);
        assert_eq!(rect.h as f32 / 2.0, a.bottom - a.top);
        // A monospaced font: every advance is the same, in half pixels.
        assert_eq!(a.advance, font.glyph('i').advance);
        assert_eq!((a.advance * 2.0).fract(), 0.0);
        assert!(a.advance > 4.0 && a.advance < 9.0, "{}", a.advance);
        // Capitals sit on the baseline at y = 7, shifted down a pixel.
        assert!((a.bottom - 8.0).abs() <= 0.5, "{}", a.bottom);
        assert_eq!(a.offset, 1.0);
        // The space has an advance and nothing to draw.
        let space = font.glyph(' ');
        assert!(space.rect.is_none() && space.advance > 0.0);
        // Skipped characters fall through to the missing glyph.
        assert_eq!(font.glyph('B').advance, 6.0);
        // Coverage as `rrrr` texels, faint coverage cut.
        let patches = font.take_patches();
        assert_eq!(patches.len(), 2);
        let texels: Vec<&[u8]> = patches.iter().flat_map(|p| p.rgba.chunks(4)).collect();
        assert!(texels.iter().all(|t| t.iter().all(|v| *v == t[0])));
        assert!(texels.iter().all(|t| t[0] == 0 || t[0] >= 25));
        assert!(texels.iter().any(|t| t[0] == 255));
    }

    /// A pack's `size` and `oversample` are unbounded: a glyph larger than the atlas can hold is
    /// refused before its bitmap is allocated, and falls to the missing glyph.
    #[test]
    fn a_true_type_glyph_too_big_for_the_atlas_is_refused_before_it_is_drawn() {
        let def = br#"{"providers":[{"type":"ttf","file":"minecraft:mono.ttf","size":100000}]}"#;
        let root = root(
            "ttf-huge",
            &[
                ("assets/minecraft/font/default.json", def),
                (
                    "assets/minecraft/font/mono.ttf",
                    horizon_theme::fonts::GEIST_MONO,
                ),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        let a = font.glyph('A');
        assert!(a.rect.is_none());
        assert_eq!(a.advance, 6.0, "the missing glyph");
        assert!(font.take_patches().is_empty());
    }

    /// A `space` advance too big for an `f32` would be infinite: the glyph is left out.
    #[test]
    fn a_space_advance_too_big_for_a_float_is_ignored() {
        let def = br#"{"providers":[{"type":"space","advances":{"a":1e300,"b":3}}]}"#;
        let root = root("space-huge", &[("assets/minecraft/font/default.json", def)]);
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(font.glyph('a').advance, 6.0, "the missing glyph");
        assert_eq!(font.glyph('b').advance, 3.0);
    }

    /// A bitmap provider's `file` can name any texture. When the atlas has that
    /// texture (HUD packs keep their glyph sheets outside `textures/font`), the
    /// glyphs are read from it instead of being copied into the small run-time
    /// region one by one; the region then cannot fill up and turn later glyphs
    /// into the missing one, whose advance shifts the rest of a HUD line.
    #[test]
    fn a_bitmap_sheet_outside_font_is_read_from_the_atlas() {
        let sheet = png(16, 8, |x, _| x < 2 || (8..14).contains(&x));
        let (_, _, pixels) = crate::hud::decode_bytes(&sheet).unwrap();
        let rect = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 16,
            h: 8,
        };
        for (ns, file, key) in [
            ("minecraft", "minecraft:hud/sheet.png", "hud/sheet"),
            (
                "hudengine",
                "hudengine:font/sheet.png",
                "hudengine:font/sheet",
            ),
        ] {
            let def = format!(
                r#"{{"providers":[{{"type":"bitmap","file":"{file}","ascent":7,"chars":["ab"]}}]}}"#
            );
            // The image is only in the atlas: the pack's copy is not on disk.
            let root = root(
                ns,
                &[("assets/minecraft/font/default.json", def.as_bytes())],
            );
            let sprites = HashMap::from([(key.to_string(), rect)]);
            let font = Font::load(
                &ResourceSource::directory(root.clone()),
                &sprites,
                &pixels,
                (16, 8),
                (0, 8),
                white(),
            );
            let _ = std::fs::remove_dir_all(&root);
            assert_eq!(font.glyph('a').advance, 3.0, "{ns}");
            assert_eq!(font.glyph('b').advance, 7.0, "{ns}");
            assert!(font.glyph('a').rect.is_some(), "{ns}");
            assert!(
                font.take_patches().is_empty(),
                "{ns}: the atlas sheet is not copied into the run-time region"
            );
        }
    }

    #[test]
    fn styles_pick_their_font_and_unknown_fonts_draw_missing_glyphs() {
        let sheet = png(16, 8, |x, _| x < 2 || (8..14).contains(&x));
        let default = br#"{"providers":[{"type":"space","advances":{" ":4}},{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ab"]}]}"#;
        let alt = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ba"]}]}"#;
        let root = root(
            "styles",
            &[
                ("assets/minecraft/font/default.json", default),
                ("assets/minecraft/font/alt.json", alt),
                ("assets/minecraft/textures/font/a.png", &sheet),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        // The sheet is not in this atlas: glyphs are copied in when first drawn.
        assert_eq!(font.glyph('a').advance, 3.0);
        assert_eq!(font.glyph('b').advance, 7.0);
        let alt = Style {
            font: Some("minecraft:alt".into()),
            ..Style::default()
        };
        assert_eq!(font.glyph_styled('a', &alt).advance, 7.0);
        assert_eq!(font.glyph_in("alt", 'b').advance, 3.0);
        assert!(font.glyph_styled('a', &alt).rect.is_some());
        // Not in alt (no space provider there), or no such font at all.
        assert_eq!(font.glyph_styled(' ', &alt).advance, 6.0);
        let missing = Style {
            font: Some("minecraft:nope".into()),
            ..Style::default()
        };
        assert_eq!(font.glyph_styled(' ', &missing).advance, 6.0);
        assert_eq!(font.advance('a', &alt), 7.0);
        let text = Text::literal("ab").with_style(&alt);
        assert_eq!(font.width_text(&text), 10.0);
        assert_eq!(font.random_glyph(&alt, 7.0).advance, 7.0);
        assert!(!font.take_patches().is_empty());
    }

    /// `FontSet.computeGlyphInfo`: `SelectedGlyphs.any` is the first provider's glyph, fishy
    /// advance or not, and is what is drawn and measured; `nonFishy`, what obfuscated text draws,
    /// is the first provider whose advance is within 0..32, else the missing glyph.
    #[test]
    fn a_fishy_glyph_is_drawn_and_only_obfuscated_text_falls_through() {
        // 64 opaque columns: the first provider's 'a' advances 65, past the 32 vanilla
        // calls fishy, and the narrow provider's 'a' advances 3.
        let wide = png(64, 8, |_, _| true);
        let narrow = png(16, 8, |x, _| x < 2);
        let def = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/wide.png","ascent":7,"chars":["a"]},{"type":"bitmap","file":"minecraft:font/narrow.png","ascent":7,"chars":["a"]}]}"#;
        let root = root(
            "fishy",
            &[
                ("assets/minecraft/font/default.json", def),
                ("assets/minecraft/textures/font/wide.png", &wide),
                ("assets/minecraft/textures/font/narrow.png", &narrow),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        let drawn = font.glyph('a');
        assert_eq!(drawn.advance, 65.0, "the first provider's glyph is drawn");
        assert!(drawn.rect.is_some());
        assert_eq!(font.advance('a', &Style::default()), 65.0);
        // The by-width table files 'a' under its first glyph's 65; the glyph it then draws is
        // the narrow provider's (`nonFishy`).
        assert_eq!(font.random_glyph(&Style::default(), 65.0).advance, 3.0);
        // Nothing else is 3 wide in the table, and the missing glyph is 6.
        assert_eq!(font.random_glyph(&Style::default(), 3.0).advance, 6.0);
    }

    /// A `space` provider's shift glyphs (kerning in server packs: -512 to 512) are drawn and
    /// measured as they are, and `nonFishy` has nothing else to draw for them.
    #[test]
    fn space_glyphs_with_any_advance_are_kept() {
        let shifts: Vec<i32> = (0..10)
            .map(|i| -(1 << i))
            .chain((0..10).map(|i| 1 << i))
            .collect();
        let advances: Vec<String> = shifts
            .iter()
            .enumerate()
            .map(|(i, a)| format!("\"\\ue{:03x}\": {a}", i))
            .collect();
        let def = format!(
            r#"{{"providers":[{{"type":"space","advances":{{{}}}}}]}}"#,
            advances.join(",")
        );
        let root = root(
            "space-shifts",
            &[("assets/minecraft/font/default.json", def.as_bytes())],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        for (i, a) in shifts.iter().enumerate() {
            let c = char::from_u32(0xe000 + i as u32).unwrap();
            assert_eq!(font.glyph(c).advance, *a as f32, "{c:?}");
            assert_eq!(font.advance(c, &Style::default()), *a as f32);
        }
        // Obfuscated text asks for a glyph of a given width: a fishy space has none to give.
        let c = char::from_u32(0xe000 + 19).unwrap();
        assert_eq!(font.glyph(c).advance, 512.0);
        assert_eq!(
            font.glyph_non_fishy(DEFAULT, c).advance,
            6.0,
            "the missing glyph"
        );
        assert_eq!(font.random_glyph(&Style::default(), 512.0).advance, 6.0);
    }

    /// Nexo's GUI glyphs: a bitmap `height` of 287 with no advance, so every glyph wider than 31
    /// pixels has a fishy advance, and is still drawn.
    #[test]
    fn a_wide_bitmap_glyph_is_drawn() {
        let sheet = png(40, 287, |_, _| true);
        let def = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/gui.png","height":287,"ascent":65,"chars":["a"]}]}"#;
        let root = root(
            "wide-bitmap",
            &[
                ("assets/minecraft/font/default.json", def),
                ("assets/minecraft/textures/font/gui.png", &sheet),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        let g = font.glyph('a');
        assert_eq!(g.advance, 41.0, "the glyph's 40 columns and one");
        assert!(g.rect.is_some(), "drawn, not the missing glyph");
        assert!(fishy_advance(g.advance));
        assert_eq!(
            font.glyph_non_fishy(DEFAULT, 'a').advance,
            6.0,
            "excluded from what obfuscated text draws"
        );
    }

    /// Server packs pair a cell with its copy at a negative `height` so the two advance a fixed
    /// amount whatever the cell's width: `(w + 1) + ((int) (0.5 - w) + 1)` is 3 in vanilla.
    #[test]
    fn a_negative_height_cell_advances_as_vanilla_truncates() {
        let sheet = png(8, 8, |x, _| x < 4);
        let def = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/cell.png","height":8,"ascent":7,"chars":["a"]},{"type":"bitmap","file":"minecraft:font/cell.png","height":-8,"ascent":-32768,"chars":["b"]}]}"#;
        let root = root(
            "negative-height",
            &[
                ("assets/minecraft/font/default.json", def),
                ("assets/minecraft/textures/font/cell.png", &sheet),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(font.glyph('a').advance, 5.0);
        assert_eq!(font.glyph('b').advance, -2.0);
    }

    /// A font whose first provider is `first` (its sheet `sheet`), then a
    /// valid provider drawing `a` from a sheet with two columns used.
    fn font_after(name: &str, first: &str, sheet: &[u8]) -> Font {
        let narrow = png(8, 8, |x, _| x < 2);
        let def = format!(
            r#"{{"providers":[{first},{{"type":"bitmap","file":"minecraft:font/narrow.png","ascent":7,"chars":["a"]}}]}}"#
        );
        let root = root(
            name,
            &[
                ("assets/minecraft/font/default.json", def.as_bytes()),
                ("assets/minecraft/textures/font/first.png", sheet),
                ("assets/minecraft/textures/font/narrow.png", &narrow),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        font
    }

    /// `BitmapProvider.Definition` refuses these while the file decodes, and
    /// the whole file goes. Rows of unequal length were sized from the first
    /// row, so the longer row's last cell reached past the sheet and panicked
    /// when drawn.
    #[test]
    fn a_font_file_with_a_refused_bitmap_provider_is_skipped() {
        let sheet = png(16, 16, |_, _| true);
        for (name, first) in [
            ("uneven", r#""ascent":7,"chars":["ab","cdef"]"#),
            (
                "ascent-above-height",
                r#""height":8,"ascent":9,"chars":["af"]"#,
            ),
            ("text-ascent", r#""ascent":"7","chars":["af"]"#),
            ("no-ascent", r#""chars":["af"]"#),
            ("empty-row", r#""ascent":7,"chars":["","af"]"#),
        ] {
            let first = format!(r#"{{"type":"bitmap","file":"minecraft:font/first.png",{first}}}"#);
            let font = font_after(name, &first, &sheet);
            let missing = font.glyph('z');
            for c in ['a', 'f'] {
                let g = font.glyph(c);
                assert_eq!(
                    g.advance, missing.advance,
                    "{name}: {c} is the missing glyph"
                );
                assert_eq!(g.rect.map(|r| r.u0), missing.rect.map(|r| r.u0), "{name}");
            }
        }
    }

    /// Vanilla reads `height` and `ascent` as integers, a fraction truncated
    /// toward zero: 8.9 and 6.5 load as 8 and 6.
    #[test]
    fn fractional_height_and_ascent_are_truncated() {
        let sheet = png(16, 16, |_, _| true);
        let first = r#"{"type":"bitmap","file":"minecraft:font/first.png","height":8.9,"ascent":6.5,"chars":["af"]}"#;
        let g = font_after("fractional", first, &sheet).glyph('f');
        // Cells 8x16 at height 8: half scale; top 7 - 6.
        assert_eq!((g.top, g.bottom), (1.0, 9.0));
        assert_eq!(g.advance, 5.0, "8 columns used, at half scale, and one");
    }

    /// `BitmapProvider.Definition.load` puts each cell in a map: a character
    /// listed twice keeps its last cell. Character 0 is padding, and an earlier
    /// provider still wins over a later one.
    #[test]
    fn a_repeated_character_keeps_its_last_cell() {
        // Cells 8 wide using 2, 6 and 4 columns: advances 3, 7 and 5.
        let sheet = png(24, 8, |x, _| {
            x < 2 || (8..14).contains(&x) || (16..20).contains(&x)
        });
        let first = r#"{"type":"bitmap","file":"minecraft:font/first.png","ascent":7,"chars":["a\u0000a"]}"#;
        let font = font_after("repeated", first, &sheet);
        assert_eq!(
            font.glyph('a').advance,
            5.0,
            "the last cell, not the first (3) or the next provider's (3)"
        );
        assert_eq!(
            font.glyph('\0').advance,
            font.glyph('z').advance,
            "character 0 is not the middle cell (7)"
        );
    }

    /// More rows or columns than the sheet has pixels leave cells of no size.
    /// Vanilla keeps the provider: each character is an empty glyph of advance
    /// 1, and a later provider's glyph for it isn't used.
    #[test]
    fn a_grid_larger_than_its_sheet_draws_empty_glyphs() {
        for (name, sheet, chars) in [
            ("too-many-rows", png(16, 1, |_, _| true), r#"["a","b"]"#),
            ("too-many-columns", png(4, 8, |_, _| true), r#"["abcde"]"#),
        ] {
            let first = format!(
                r#"{{"type":"bitmap","file":"minecraft:font/first.png","ascent":7,"chars":{chars}}}"#
            );
            let g = font_after(name, &first, &sheet).glyph('a');
            assert_eq!(g.advance, 1.0, "{name}: not the next provider's 3");
            assert!(g.rect.is_none(), "{name}: nothing drawn");
            assert!(g.top.is_finite() && g.bottom.is_finite(), "{name}");
        }
    }

    /// StringDecomposer.iterateFormatted: a `§` that ends a part stops that
    /// part only; the next part is still measured and can break.
    #[test]
    fn a_section_sign_ending_a_part_still_breaks_the_next() {
        let sheet = png(16, 8, |x, _| x < 2);
        let default = br#"{"providers":[{"type":"space","advances":{" ":4}},{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ab"]}]}"#;
        let root = root(
            "section-sign",
            &[
                ("assets/minecraft/font/default.json", default),
                ("assets/minecraft/textures/font/a.png", &sheet),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        let text = Text::from_spans(
            ["a§", "a a"]
                .map(|part| Span::new(part, Style::default()))
                .into(),
        );
        // 3 + 3 + 4 + 3 is past 12: the line breaks at the space.
        let lines: Vec<String> = font
            .split_formatted(&text, 12.0)
            .iter()
            .map(Text::plain)
            .collect();
        assert_eq!(lines, ["a§a", "a"]);
        let mixed = Text::literal("aa")
            .append(Text::untrusted("aa"))
            .with_style(&Style {
                click: Some(crate::text::ClickEvent::OpenFile("/tmp/local".into())),
                ..Style::default()
            });
        let lines = font.split_formatted(&mixed, 6.0);
        assert_eq!(
            lines.iter().map(Text::plain).collect::<Vec<_>>(),
            ["aa", "aa"]
        );
        assert_eq!(lines[0].spans[0].origin(), crate::text::Origin::Local);
        assert_eq!(lines[1].spans[0].origin(), crate::text::Origin::Remote);
        assert!(lines[0].spans[0].click_event().is_some());
        assert!(lines[1].spans[0].click_event().is_none());
    }

    /// A renderer rebuilt over the same font (the loading screen's handoff)
    /// makes its atlas texture from the textures alone, so the glyphs it had
    /// rasterized into the previous texture must be placed into the new one.
    #[test]
    fn a_rebuilt_atlas_gets_its_glyphs_again() {
        let sheet = png(16, 8, |x, _| x < 2 || (8..14).contains(&x));
        let default = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ab"]}]}"#;
        let root = root(
            "rebuild",
            &[
                ("assets/minecraft/font/default.json", default),
                ("assets/minecraft/textures/font/a.png", &sheet),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        let first = font.glyph('a');
        assert_eq!(font.take_patches().len(), 1);
        font.reset_cache();
        // Nothing is left over: drawing the character again places its pixels.
        assert!(font.take_patches().is_empty());
        assert_eq!(font.glyph('a').advance, first.advance);
        assert_eq!(font.take_patches().len(), 1);
    }

    #[test]
    fn provider_filters_follow_the_font_options() {
        let sheet = png(8, 8, |x, _| x < 4);
        let default = br#"{"providers":[{"type":"reference","id":"minecraft:include/sheet","filter":{"uniform":false}},{"type":"space","advances":{"a":2}}]}"#;
        let sheet_def = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/s.png","ascent":7,"chars":["a"]}]}"#;
        let root = root(
            "filters",
            &[
                ("assets/minecraft/font/default.json", default),
                ("assets/minecraft/font/include/sheet.json", sheet_def),
                ("assets/minecraft/textures/font/s.png", &sheet),
            ],
        );
        let mut font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(font.glyph('a').advance, 5.0);
        font.uniform = true;
        font.reset_cache();
        assert_eq!(font.glyph('a').advance, 2.0);
    }

    #[test]
    fn a_full_glyph_region_recovers_at_the_next_frame() {
        // Two 8x8 glyphs from a sheet outside the atlas; the region holds one row.
        let sheet = png(16, 8, |x, _| x % 8 < 4);
        let default = br#"{"providers":[{"type":"bitmap","file":"pack:font/s.png","ascent":7,"chars":["ab"]}]}"#;
        let root = root(
            "full",
            &[
                ("assets/minecraft/font/default.json", default),
                ("assets/pack/textures/font/s.png", &sheet),
            ],
        );
        // A 10 wide atlas fits one 8 pixel glyph per row; the region two rows.
        let font = Font::load(
            &ResourceSource::directory(root.clone()),
            &HashMap::new(),
            &[],
            (10, 64),
            (0, 8),
            white(),
        );
        let _ = std::fs::remove_dir_all(&root);
        let epoch = font.epoch();
        let a = font.glyph('a');
        assert!(a.rect.is_some());
        // No room for 'b': missing for now, and asked again next time.
        assert!(font.glyph('b').rect.is_none());
        assert!(font.glyph('b').rect.is_none());
        assert_eq!(font.epoch(), epoch);
        font.begin_frame();
        assert_ne!(font.epoch(), epoch);
        let b = font.glyph('b');
        assert!(b.rect.is_some());
        assert_eq!(b.advance, a.advance);
        // Nothing overflowed since: the next frame keeps the glyphs.
        let epoch = font.epoch();
        font.begin_frame();
        assert_eq!(font.epoch(), epoch);
    }

    #[test]
    fn a_character_without_a_provider_does_not_reset_the_region() {
        let sheet = png(8, 8, |x, _| x < 4);
        let default = br#"{"providers":[{"type":"bitmap","file":"pack:font/s.png","ascent":7,"chars":["a"]}]}"#;
        let root = root(
            "nofill",
            &[
                ("assets/minecraft/font/default.json", default),
                ("assets/pack/textures/font/s.png", &sheet),
            ],
        );
        let font = Font::load(
            &ResourceSource::directory(root.clone()),
            &HashMap::new(),
            &[],
            (10, 64),
            (0, 8),
            white(),
        );
        let _ = std::fs::remove_dir_all(&root);
        let epoch = font.epoch();
        assert!(font.glyph('z').rect.is_none());
        font.begin_frame();
        assert_eq!(font.epoch(), epoch);
    }

    fn rgba_png(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(pixels)
            .unwrap();
        out
    }

    fn cell_rgba(pixels: &[u8], stride: u32, x: u32, y: u32, w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for dy in 0..h {
            let start = (((y + dy) * stride + x) * 4) as usize;
            out.extend_from_slice(&pixels[start..start + (w * 4) as usize]);
        }
        out
    }

    fn off_atlas_strip_bytes(font: &Font) -> usize {
        let mut seen = std::collections::HashSet::new();
        let mut total = 0;
        for (_, set) in &font.cache.borrow().sets {
            for provider in &set.providers {
                if let Provider::Bitmap(_, glyphs) = provider {
                    for glyph in glyphs.values() {
                        if let BitmapGlyph::Sheet(cell) = glyph
                            && seen.insert(cell.sheet.id)
                        {
                            total += cell.sheet.stored_bytes();
                        }
                    }
                }
            }
        }
        total
    }

    #[test]
    fn off_atlas_bitmap_strips_upload_exact_pixels_and_bound_decoded_work() {
        // Large-cell sheet: 8x2 grid of 64x64 cells (512x128 = 256 KiB raw).
        let (large_w, large_h) = (512u32, 128u32);
        let mut large_pixels = vec![0u8; (large_w * large_h * 4) as usize];
        for y in 0..large_h {
            for x in 0..large_w {
                let col = x / 64;
                let row = y / 64;
                let lx = x % 64;
                let ly = y % 64;
                let px = if (4..60).contains(&lx) && (4..60).contains(&ly) {
                    [
                        (col * 29 + 17) as u8,
                        (row * 71 + 43) as u8,
                        ((row * 8 + col) * 13 + 9) as u8,
                        255,
                    ]
                } else if (lx, ly) == (0, 0) {
                    [row as u8, col as u8, 1, 255]
                } else if (lx, ly) == (63, 63) {
                    [row as u8, col as u8, 2, 255]
                } else {
                    [0, 0, 0, 0]
                };
                let at = ((y * large_w + x) * 4) as usize;
                large_pixels[at..at + 4].copy_from_slice(&px);
            }
        }
        let large_png = rgba_png(large_w, large_h, &large_pixels);
        let large_def = br#"{"providers":[{"type":"bitmap","file":"pack:font/large.png","height":64,"ascent":7,"chars":["abcdefgh","ijklmnop"]}]}"#;
        let large_root = root(
            "strips-large",
            &[
                ("assets/minecraft/font/default.json", large_def),
                ("assets/pack/textures/font/large.png", &large_png),
            ],
        );
        let font = load(&large_root);
        let _ = std::fs::remove_dir_all(&large_root);

        let raw_large_bytes = (large_w * large_h * 4) as usize;
        let stored_large = off_atlas_strip_bytes(&font);
        assert!(
            stored_large < raw_large_bytes / 2,
            "expected LZ4-compressed strips to shrink large sheet ({stored_large} vs {raw_large_bytes})"
        );

        let (patches, decoded_pass1) = measure_decoded_bytes(|| {
            for ch in "abcdefghijklmnop".chars() {
                assert!(font.glyph(ch).rect.is_some(), "glyph {ch} missing");
            }
            font.take_patches()
        });
        assert_eq!(patches.len(), 16);
        for (idx, patch) in patches.iter().enumerate() {
            let row = (idx / 8) as u32;
            let col = (idx % 8) as u32;
            assert_eq!((patch.width, patch.height), (64, 64));
            assert_eq!(
                patch.rgba,
                cell_rgba(&large_pixels, large_w, col * 64, row * 64, 64, 64),
                "large cell ({row}, {col}) mismatch on pass 1"
            );
        }
        assert!(
            decoded_pass1 <= raw_large_bytes,
            "resolving all 16 large cells decoded {decoded_pass1} bytes > {raw_large_bytes}"
        );

        // Cache reset re-uploads identical pixels with bounded decoded work.
        font.reset_cache();
        let (patches2, decoded_pass2) = measure_decoded_bytes(|| {
            for ch in "abcdefghijklmnop".chars() {
                assert!(font.glyph(ch).rect.is_some(), "glyph {ch} missing");
            }
            font.take_patches()
        });
        assert_eq!(patches2.len(), 16);
        for (idx, patch) in patches2.iter().enumerate() {
            let row = (idx / 8) as u32;
            let col = (idx % 8) as u32;
            assert_eq!((patch.width, patch.height), (64, 64));
            assert_eq!(
                patch.rgba,
                cell_rgba(&large_pixels, large_w, col * 64, row * 64, 64, 64),
                "large cell ({row}, {col}) mismatch on pass 2"
            );
        }
        assert!(
            decoded_pass2 <= raw_large_bytes,
            "re-resolving all 16 large cells decoded {decoded_pass2} bytes > {raw_large_bytes}"
        );

        // A single cold large-cell lookup must not decompress the whole row.
        font.reset_cache();
        let (single_patches, decoded_single) = measure_decoded_bytes(|| {
            assert!(font.glyph('d').rect.is_some());
            font.take_patches()
        });
        assert_eq!(single_patches.len(), 1);
        assert_eq!(
            single_patches[0].rgba,
            cell_rgba(&large_pixels, large_w, 3 * 64, 0, 64, 64)
        );
        assert!(
            decoded_single <= 64 * 64 * 4,
            "single large cell decoded {decoded_single} bytes > {}",
            64 * 64 * 4
        );

        // Small-cell sheet: 8x10 grid of 16x16 cells (128x160 = 80 KiB raw).
        // Interleaving two rows exercises MRU promotion; walking 10 rows exercises eviction.
        let (small_w, small_h) = (128u32, 160u32);
        let mut small_pixels = vec![0u8; (small_w * small_h * 4) as usize];
        for y in 0..small_h {
            for x in 0..small_w {
                let col = x / 16;
                let row = y / 16;
                let lx = x % 16;
                let ly = y % 16;
                let px = if (2..14).contains(&lx) && (2..14).contains(&ly) {
                    [
                        (row * 23 + 11) as u8,
                        (col * 31 + 5) as u8,
                        ((row * 8 + col) * 7 + 3) as u8,
                        255,
                    ]
                } else if (lx, ly) == (0, 0) {
                    [row as u8, col as u8, 3, 255]
                } else if (lx, ly) == (15, 15) {
                    [row as u8, col as u8, 4, 255]
                } else {
                    [0, 0, 0, 0]
                };
                let at = ((y * small_w + x) * 4) as usize;
                small_pixels[at..at + 4].copy_from_slice(&px);
            }
        }
        let small_png = rgba_png(small_w, small_h, &small_pixels);
        let small_rows: Vec<String> = (0..10u32)
            .map(|row| {
                (0..8u32)
                    .map(|col| char::from_u32(0xE000 + row * 8 + col).unwrap())
                    .collect()
            })
            .collect();
        let small_def = serde_json::json!({
            "providers": [{
                "type": "bitmap",
                "file": "pack:font/small.png",
                "height": 16,
                "ascent": 7,
                "chars": small_rows,
            }]
        })
        .to_string();
        let small_root = root(
            "strips-small",
            &[
                ("assets/minecraft/font/default.json", small_def.as_bytes()),
                ("assets/pack/textures/font/small.png", &small_png),
            ],
        );
        let small_font = load(&small_root);
        let _ = std::fs::remove_dir_all(&small_root);

        let mut order: Vec<(u32, u32)> = Vec::with_capacity(80);
        for col in 0..8 {
            order.push((0, col));
            order.push((1, col));
        }
        for row in 2..10 {
            for col in 0..8 {
                order.push((row, col));
            }
        }
        let (small_patches, decoded_small) = measure_decoded_bytes(|| {
            for &(row, col) in &order {
                let ch = char::from_u32(0xE000 + row * 8 + col).unwrap();
                assert!(
                    small_font.glyph(ch).rect.is_some(),
                    "small glyph ({row}, {col}) missing"
                );
            }
            small_font.take_patches()
        });
        assert_eq!(small_patches.len(), 80);
        for (&(row, col), patch) in order.iter().zip(&small_patches) {
            assert_eq!((patch.width, patch.height), (16, 16));
            assert_eq!(
                patch.rgba,
                cell_rgba(&small_pixels, small_w, col * 16, row * 16, 16, 16),
                "small cell ({row}, {col}) mismatch"
            );
        }
        let raw_small_bytes = (small_w * small_h * 4) as usize;
        assert!(
            decoded_small <= raw_small_bytes,
            "resolving all 80 small cells decoded {decoded_small} bytes > {raw_small_bytes}"
        );
    }

    #[test]
    fn incompressible_off_atlas_strip_stays_raw_without_decompressing() {
        let (w, h) = (64u32, 64u32);
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut noise = vec![0u8; (w * h * 4) as usize];
        for chunk in noise.as_chunks_mut::<8>().0 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            chunk.copy_from_slice(&state.to_le_bytes());
        }
        let sheet = rgba_png(w, h, &noise);
        let def = br#"{"providers":[{"type":"bitmap","file":"pack:font/noise.png","height":64,"ascent":7,"chars":["a"]}]}"#;
        let noise_root = root(
            "strips-incompressible",
            &[
                ("assets/minecraft/font/default.json", def),
                ("assets/pack/textures/font/noise.png", &sheet),
            ],
        );
        let font = load(&noise_root);
        let _ = std::fs::remove_dir_all(&noise_root);

        assert_eq!(
            off_atlas_strip_bytes(&font),
            noise.len(),
            "incompressible strip should store exact raw bytes with no LZ4 bloat"
        );
        for pass in 0..2 {
            let (patches, decoded) = measure_decoded_bytes(|| {
                assert!(font.glyph('a').rect.is_some());
                font.take_patches()
            });
            assert_eq!(patches.len(), 1, "pass {pass}");
            assert_eq!((patches[0].width, patches[0].height), (w, h));
            assert_eq!(patches[0].rgba, noise, "pass {pass}");
            assert_eq!(
                decoded, 0,
                "pass {pass}: incompressible strip should not waste CPU decompressing"
            );
            font.reset_cache();
        }
    }

    #[test]
    fn corrupted_compressed_strip_warns_once_with_file_name_and_draws_empty_cell() {
        let info = Arc::new(SheetInfo::new("pack:font/broken.png"));
        let bad_stream = Arc::new(SheetStrip {
            id: next_strip_id(),
            info: Arc::clone(&info),
            stride: 4,
            cacheable: false,
            pixels: StripPixels::Lz4 {
                len: 64,
                compressed: vec![0xFF; 16],
            },
        });
        let sibling_short = Arc::new(SheetStrip {
            id: next_strip_id(),
            info,
            stride: 4,
            cacheable: false,
            pixels: StripPixels::Lz4 {
                len: 64,
                compressed: lz4_flex::block::compress(&[0xAA; 16]),
            },
        });
        let make_cell = |sheet: Arc<SheetStrip>| SheetCell {
            sheet,
            x: 0,
            y: 0,
            w: 4,
            h: 4,
            glyph: Glyph::default(),
        };
        let cell_bad = make_cell(bad_stream);
        let cell_sibling = make_cell(sibling_short);
        let mut strips = StripCache::default();

        let (rgba1, logs1) = crate::test_log::capture(|| cell_bad.pixels(&mut strips));
        assert_eq!(rgba1, Some(vec![0; 64]));
        assert_eq!(logs1.len(), 1, "expected a single warning, got {logs1:?}");
        assert!(
            logs1[0].contains("pack:font/broken.png"),
            "warning must name the sheet file: {}",
            logs1[0]
        );

        let (rgba2, logs2) = crate::test_log::capture(|| cell_bad.pixels(&mut strips));
        assert_eq!(rgba2, Some(vec![0; 64]));
        assert!(
            logs2.is_empty(),
            "same strip must not warn twice: {logs2:?}"
        );

        let (rgba3, logs3) = crate::test_log::capture(|| cell_sibling.pixels(&mut strips));
        assert_eq!(rgba3, Some(vec![0; 64]));
        assert!(
            logs3.is_empty(),
            "sibling strip of the same sheet must not warn again: {logs3:?}"
        );

        // Wrong decoded length on another sheet warns once and clears any partial bytes.
        let short_strip = Arc::new(SheetStrip {
            id: next_strip_id(),
            info: Arc::new(SheetInfo::new("pack:font/short_decode.png")),
            stride: 4,
            cacheable: false,
            pixels: StripPixels::Lz4 {
                len: 64,
                compressed: lz4_flex::block::compress(&[0xAA; 16]),
            },
        });
        let cell_short = make_cell(short_strip);
        let (rgba4, logs4) = crate::test_log::capture(|| cell_short.pixels(&mut strips));
        assert_eq!(rgba4, Some(vec![0; 64]));
        assert_eq!(logs4.len(), 1, "expected a single warning, got {logs4:?}");
        assert!(
            logs4[0].contains("pack:font/short_decode.png"),
            "warning must name the sheet file: {}",
            logs4[0]
        );

        let (rgba5, logs5) = crate::test_log::capture(|| cell_short.pixels(&mut strips));
        assert_eq!(rgba5, Some(vec![0; 64]));
        assert!(logs5.is_empty(), "must not warn twice: {logs5:?}");
    }

    #[test]
    fn off_atlas_sheet_with_non_multiple_width_and_padding_extracts_exact_cells() {
        // 19x8 sheet with 2 columns -> cell_w = 9, chunk_stride = 18 != sheet stride 19.
        let (odd_w, odd_h) = (19u32, 8u32);
        let mut odd_pixels = vec![0u8; (odd_w * odd_h * 4) as usize];
        for y in 0..odd_h {
            for x in 0..odd_w {
                let at = ((y * odd_w + x) * 4) as usize;
                odd_pixels[at..at + 4].copy_from_slice(&[
                    (x * 13 + 7) as u8,
                    (y * 29 + 3) as u8,
                    ((x + y) * 11) as u8,
                    255,
                ]);
            }
        }
        let odd_png = rgba_png(odd_w, odd_h, &odd_pixels);
        let odd_def = br#"{"providers":[{"type":"bitmap","file":"pack:font/odd.png","height":8,"ascent":7,"chars":["ab"]}]}"#;
        let odd_root = root(
            "strips-odd-width",
            &[
                ("assets/minecraft/font/default.json", odd_def),
                ("assets/pack/textures/font/odd.png", &odd_png),
            ],
        );
        let odd_font = load(&odd_root);
        let _ = std::fs::remove_dir_all(&odd_root);
        assert!(odd_font.glyph('a').rect.is_some());
        assert!(odd_font.glyph('b').rect.is_some());
        let odd_patches = odd_font.take_patches();
        assert_eq!(odd_patches.len(), 2);
        assert_eq!((odd_patches[0].width, odd_patches[0].height), (9, 8));
        assert_eq!(
            odd_patches[0].rgba,
            cell_rgba(&odd_pixels, odd_w, 0, 0, 9, 8)
        );
        assert_eq!((odd_patches[1].width, odd_patches[1].height), (9, 8));
        assert_eq!(
            odd_patches[1].rgba,
            cell_rgba(&odd_pixels, odd_w, 9, 0, 9, 8)
        );

        // 10 cols of 32x32 with all-zero padding chunks, single-active chunks, and multi-active chunks.
        let (pad_w, pad_h) = (320u32, 32u32);
        let mut pad_pixels = vec![0u8; (pad_w * pad_h * 4) as usize];
        for y in 0..pad_h {
            for x in 0..pad_w {
                let col = x / 32;
                let lx = x % 32;
                let ly = y % 32;
                let px = if (4..28).contains(&lx) && (4..28).contains(&ly) {
                    [(col * 25 + 10) as u8, (lx * 7) as u8, (ly * 7) as u8, 255]
                } else {
                    [0, 0, 0, 0]
                };
                let at = ((y * pad_w + x) * 4) as usize;
                pad_pixels[at..at + 4].copy_from_slice(&px);
            }
        }
        let pad_png = rgba_png(pad_w, pad_h, &pad_pixels);
        let pad_def = br#"{"providers":[{"type":"bitmap","file":"pack:font/pad.png","height":32,"ascent":7,"chars":["\u0000\u0000\u0000ab\u0000cd\u0000\u0000"]}]}"#;
        let pad_root = root(
            "strips-padding",
            &[
                ("assets/minecraft/font/default.json", pad_def),
                ("assets/pack/textures/font/pad.png", &pad_png),
            ],
        );
        let pad_font = load(&pad_root);
        let _ = std::fs::remove_dir_all(&pad_root);
        let expected_cols = [('a', 3u32), ('b', 4), ('c', 6), ('d', 7)];
        for &(ch, _) in &expected_cols {
            assert!(pad_font.glyph(ch).rect.is_some(), "glyph {ch} missing");
        }
        let pad_patches = pad_font.take_patches();
        assert_eq!(pad_patches.len(), expected_cols.len());
        for (&(ch, col), patch) in expected_cols.iter().zip(&pad_patches) {
            assert_eq!((patch.width, patch.height), (32, 32));
            assert_eq!(
                patch.rgba,
                cell_rgba(&pad_pixels, pad_w, col * 32, 0, 32, 32),
                "padded glyph {ch} at col {col} mismatch"
            );
        }
    }
}
