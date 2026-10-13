//! The default font (FontSet over `font/default.json`): space and bitmap
//! providers from the shared atlas, the unihex (GNU Unifont) fallback whose
//! glyphs are rasterized on demand into a reserved atlas region, vanilla
//! glyph metrics (ascent, oversampling, advances, bold/shadow offsets),
//! styled drawing and StringSplitter-style line wrapping.
use crate::{
    hud::Rect,
    text::{Style, Text},
    zip::Zip,
};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
};

/// A glyph ready to draw: atlas UVs and its box relative to the pen
/// position (GlyphBitmap left/top/right/bottom), in GUI pixels.
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub rect: Option<Rect>,
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub advance: f32,
    /// Shadow and bold offset (1 for bitmap glyphs, 0.5 for unihex).
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
#[derive(Default)]
struct HexProvider {
    glyphs: HashMap<u32, HexGlyph>,
}
struct Cache {
    glyphs: HashMap<char, Glyph>,
    cursor: (u32, u32),
    pending: Vec<Patch>,
    hex: Option<Vec<HexProvider>>,
}
pub struct Font {
    bitmap: HashMap<char, Glyph>,
    space: HashMap<char, f32>,
    missing: Glyph,
    white: Rect,
    root: PathBuf,
    /// Atlas size and the reserved dynamic region (y, height).
    atlas: (u32, u32),
    region: (u32, u32),
    cache: RefCell<Cache>,
    /// "Force Unicode Font": skip the bitmap providers (font/uniform.json).
    pub uniform: bool,
    /// "Japanese Glyph Variants": enable providers filtered on `jp`.
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
fn read_json(root: &Path, path: &str) -> Option<serde_json::Value> {
    serde_json::from_slice(&std::fs::read(root.join(path)).ok()?).ok()
}
fn resource_path(id: &str, folder: &str, ext: &str) -> String {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    format!("assets/{ns}/{folder}/{}{ext}", path.trim_end_matches(ext))
}
/// Providers of a font definition with references expanded, in order, with
/// their `filter` objects.
fn providers(root: &Path, id: &str, depth: u32, out: &mut Vec<serde_json::Value>) {
    if depth > 8 {
        return;
    }
    let Some(def) = read_json(root, &resource_path(id, "font", ".json")) else {
        return;
    };
    for p in def["providers"].as_array().into_iter().flatten() {
        if p["type"] == "reference" {
            let mut nested = Vec::new();
            providers(root, p["id"].as_str().unwrap_or(""), depth + 1, &mut nested);
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
impl Font {
    /// Build from the atlas holding the bitmap font sheets. `pixels` is the
    /// RGBA atlas (for glyph widths) and `region` the reserved rows.
    pub fn load(
        root: &Path,
        sprites: &HashMap<String, Rect>,
        pixels: &[u8],
        atlas: (u32, u32),
        region: (u32, u32),
        white: Rect,
    ) -> Self {
        let mut bitmap = HashMap::new();
        let mut space = HashMap::new();
        let mut all = Vec::new();
        providers(root, "minecraft:default", 0, &mut all);
        for provider in &all {
            match provider["type"].as_str() {
                Some("space") => {
                    for (k, v) in provider["advances"].as_object().into_iter().flatten() {
                        if let (Some(c), Some(a)) = (k.chars().next(), v.as_f64()) {
                            space.entry(c).or_insert(a as f32);
                        }
                    }
                }
                Some("bitmap") => {
                    // Only non-uniform bitmap sheets exist in the default font.
                    let file = provider["file"].as_str().unwrap_or("");
                    let name = file
                        .trim_start_matches("minecraft:")
                        .trim_end_matches(".png");
                    let name = name.to_string();
                    let Some(rect) = sprites.get(&name).copied() else {
                        continue;
                    };
                    let rows: Vec<Vec<char>> = provider["chars"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|r| r.as_str().map(|s| s.chars().collect()))
                        .collect();
                    if rows.is_empty() || rows[0].is_empty() {
                        continue;
                    }
                    let cell_w = rect.w / rows[0].len() as u32;
                    let cell_h = rect.h / rows.len() as u32;
                    let height = provider["height"].as_f64().unwrap_or(8.0) as f32;
                    let ascent = provider["ascent"].as_f64().unwrap_or(7.0) as f32;
                    let scale = height / cell_h as f32;
                    let (ax, ay) = (
                        (rect.u0 * atlas.0 as f32).round() as u32,
                        (rect.v0 * atlas.1 as f32).round() as u32,
                    );
                    for (r, row) in rows.iter().enumerate() {
                        for (c, ch) in row.iter().enumerate() {
                            if *ch == '\0' || bitmap.contains_key(ch) {
                                continue;
                            }
                            let (cx, cy) = (ax + c as u32 * cell_w, ay + r as u32 * cell_h);
                            // BitmapProvider.getActualGlyphWidth.
                            let mut used = 0;
                            for x in (0..cell_w).rev() {
                                if (0..cell_h).any(|y| {
                                    pixels
                                        .get((((cy + y) * atlas.0 + cx + x) * 4 + 3) as usize)
                                        .is_some_and(|a| *a != 0)
                                }) {
                                    used = x + 1;
                                    break;
                                }
                            }
                            let top = 7.0 - ascent;
                            bitmap.insert(
                                *ch,
                                Glyph {
                                    rect: Some(glyph_rect(cx, cy, cell_w, cell_h, atlas)),
                                    left: 0.0,
                                    top,
                                    right: cell_w as f32 * scale,
                                    bottom: top + cell_h as f32 * scale,
                                    advance: (0.5 + used as f32 * scale).floor() + 1.0,
                                    offset: 1.0,
                                },
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        let missing = Glyph {
            rect: None,
            left: 0.0,
            top: 0.0,
            right: 5.0,
            bottom: 8.0,
            advance: 6.0,
            offset: 1.0,
        };
        Self {
            bitmap,
            space,
            missing,
            white,
            root: root.to_owned(),
            atlas,
            region,
            cache: RefCell::new(Cache {
                glyphs: HashMap::new(),
                cursor: (0, 0),
                pending: Vec::new(),
                hex: None,
            }),
            uniform: false,
            japanese: false,
        }
    }
    fn load_hex(&self) -> Vec<HexProvider> {
        let mut all = Vec::new();
        providers(&self.root, "minecraft:default", 0, &mut all);
        let mut out = Vec::new();
        for p in all.iter().filter(|p| p["type"] == "unihex") {
            if let Some(filter) = p["filter"].as_object() {
                let pass = filter.iter().all(|(k, v)| match k.as_str() {
                    "jp" => v.as_bool() == Some(self.japanese),
                    "uniform" => v.as_bool() == Some(self.uniform),
                    _ => true,
                });
                if !pass {
                    continue;
                }
            }
            let Some(file) = p["hex_file"].as_str() else {
                continue;
            };
            let path = self
                .root
                .join(resource_path(file, "", "").replace("//", "/"));
            let Ok(zip) = Zip::open(&path) else { continue };
            let mut provider = HexProvider::default();
            let names: Vec<String> = zip
                .names()
                .filter(|n| n.ends_with(".hex"))
                .map(str::to_owned)
                .collect();
            for name in names {
                if let Ok(bytes) = zip.read(&name) {
                    parse_hex(&bytes, &mut provider.glyphs);
                }
            }
            for o in p["size_overrides"].as_array().into_iter().flatten() {
                let from = o["from"]
                    .as_str()
                    .and_then(|s| s.chars().next())
                    .map_or(0, u32::from);
                let to = o["to"]
                    .as_str()
                    .and_then(|s| s.chars().next())
                    .map_or(0, u32::from);
                let (left, right) = (
                    o["left"].as_u64().unwrap_or(0) as u32,
                    o["right"].as_u64().unwrap_or(15) as u32,
                );
                for c in from..=to {
                    if let Some(g) = provider.glyphs.get_mut(&c) {
                        g.left = left;
                        g.right = right;
                    }
                }
            }
            out.push(provider);
        }
        out
    }
    /// Forget rasterized fallback glyphs (after changing font options).
    pub fn reset_cache(&self) {
        let mut cache = self.cache.borrow_mut();
        cache.glyphs.clear();
        cache.cursor = (0, 0);
        cache.hex = None;
    }
    /// The glyph for a character (FontSet.getGlyph).
    pub fn glyph(&self, c: char) -> Glyph {
        if let Some(a) = self.space.get(&c) {
            return Glyph {
                rect: None,
                left: 0.0,
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
                advance: *a,
                offset: 1.0,
            };
        }
        if !self.uniform
            && let Some(g) = self.bitmap.get(&c)
        {
            return *g;
        }
        if let Some(g) = self.cache.borrow().glyphs.get(&c) {
            return *g;
        }
        let mut cache = self.cache.borrow_mut();
        if cache.hex.is_none() {
            cache.hex = Some(self.load_hex());
        }
        let hex = cache.hex.as_ref().and_then(|providers| {
            providers
                .iter()
                .find_map(|p| p.glyphs.get(&u32::from(c)).copied())
        });
        let glyph = match hex {
            Some(h) => self.rasterize(&mut cache, h).unwrap_or(self.missing),
            None => self.missing,
        };
        cache.glyphs.insert(c, glyph);
        glyph
    }
    fn rasterize(&self, cache: &mut Cache, h: HexGlyph) -> Option<Glyph> {
        let width = h.right.saturating_sub(h.left) + 1;
        let (mut x, mut y) = cache.cursor;
        if x + width + 1 > self.atlas.0 {
            x = 0;
            y += 17;
        }
        if y + 16 > self.region.1 {
            return None;
        }
        let mut rgba = Vec::with_capacity((width * 16 * 4) as usize);
        for row in h.rows {
            for bit in h.left..=h.right {
                let set = bit < 32 && (row >> (31 - bit)) & 1 != 0;
                rgba.extend(if set { [255; 4] } else { [0; 4] });
            }
        }
        let (ax, ay) = (x, self.region.0 + y);
        cache.pending.push(Patch {
            x: ax,
            y: ay,
            width,
            height: 16,
            rgba,
        });
        cache.cursor = (x + width + 1, y);
        Some(Glyph {
            rect: Some(glyph_rect(ax, ay, width, 16, self.atlas)),
            left: 0.0,
            top: 0.0,
            right: width as f32 / 2.0,
            bottom: 8.0,
            advance: (width / 2 + 1) as f32,
            offset: 0.5,
        })
    }
    /// Atlas pixels rasterized since the last call.
    pub fn take_patches(&self) -> Vec<Patch> {
        std::mem::take(&mut self.cache.borrow_mut().pending)
    }
    pub fn advance(&self, c: char, style: &Style) -> f32 {
        let g = self.glyph(c);
        g.advance + if style.is_bold() { g.offset } else { 0.0 }
    }
    /// Font.width of plain text (legacy codes are formatting, not glyphs).
    pub fn width(&self, s: &str) -> f32 {
        if s.contains('§') {
            return self.width_text(&Text::legacy(s));
        }
        s.chars().map(|c| self.glyph(c).advance).sum::<f32>().ceil()
    }
    pub fn width_text(&self, t: &Text) -> f32 {
        t.chars()
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
    /// `ComponentRenderUtils.wrapComponents`: the split lines, wrapped ones
    /// indented by an unstyled space; one empty line for empty text.
    pub fn wrap_components(&self, t: &Text, max_width: i32) -> Vec<Text> {
        let lines: Vec<Text> = self
            .split_lines(t, max_width as f32)
            .into_iter()
            .map(|(line, wrapped)| {
                if wrapped {
                    Text::literal(" ").append(line)
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
